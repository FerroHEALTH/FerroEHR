// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The cipher and the blind-lookup digest for protected national identifiers.
//!
//! **No openEHR spec governs this — our own design/extension.** The RM models
//! identifiers generically (`DV_IDENTIFIER`, RM `data_types`
//! `UML/classes/org.openehr.rm.data_types.dv_identifier.adoc`) and says
//! nothing about protecting the value. The obligations are external: GDPR
//! Art. 32(1)(a) names encryption as an appropriate measure
//! (<https://eur-lex.europa.eu/eli/reg/2016/679/oj>), NEN 7510-2 carries the
//! cryptographic controls, and UAVG Art. 46 with the Wabvpz permit BSN
//! processing in care only for identification and under the act's conditions
//! (<https://wetten.overheid.nl/BWBR0040940>,
//! <https://wetten.overheid.nl/BWBR0023864>).
//!
//! Two primitives, because storage and lookup want opposite things:
//!
//! * **AES-256-GCM** over the identifier value, with a fresh 96-bit nonce per
//!   record. AES-GCM rather than ChaCha20-Poly1305 because NIST specifies it
//!   (SP 800-38D) and this control answers to reviewers who look for that;
//!   both are pure-Rust `RustCrypto` AEADs. The scheme code is bound in as
//!   associated data, so a ciphertext moved to another scheme fails to open
//!   rather than decrypting into the wrong meaning.
//! * **HMAC-SHA-256** over the value, as the blind-lookup column. Equality
//!   search needs a deterministic image of the value; a keyed digest gives one
//!   without letting the database, a backup or a replica reverse it, which an
//!   unkeyed hash of a nine-digit number would not (the whole space is
//!   enumerable in seconds).
//!
//! Keys are per DOMAIN, derived from one configured root key by HMAC-SHA-256
//! over a labelled context — the NIST SP 800-108 KDF-in-counter construction
//! with a single block, which is all a 256-bit subkey needs. One key in
//! configuration therefore yields a distinct cipher key and a distinct lookup
//! key per domain.
//!
//! The domain is bound in because the clinical and
//! demographic sides are separate pseudonymisation domains (GDPR Art. 4(5);
//! EDPB Guidelines 01/2025 §2), and a separation that holds in the schema and
//! in the database roles but shares one cipher key is one key disclosure away
//! from collapsing. With [`KeyDomain`] in the derivation context, a subkey
//! derived for the clinical domain cannot open a demographic record even when
//! both are derived from the same configured root — which is what makes a
//! per-schema backup a genuinely separate artefact rather than two files under
//! one key.

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce};
use hmac::{Hmac, KeyInit as HmacKeyInit, Mac};
use secrecy::ExposeSecret;
use sha2::Sha256;
use uuid::Uuid;

/// The labelled context the per-domain cipher key is derived under.
///
/// Versioned so a future construction change is a new label rather than a
/// silent reinterpretation of the same bytes.
const CIPHER_KEY_LABEL: &str = "ferroehr:national-identifier:cipher:v2";

/// The labelled context the per-domain lookup key is derived under.
///
/// Separate from [`CIPHER_KEY_LABEL`] so the key that produces a searchable
/// digest is not the key that decrypts: a component that only needs to look an
/// identifier up never has to hold the one that opens the ciphertext.
const LOOKUP_KEY_LABEL: &str = "ferroehr:national-identifier:lookup:v2";

/// The derivation label of the subject pseudonym a party is known by on the
/// clinical side (#3232). Its own label under the LINKAGE domain: a holder of
/// the demographic lookup digests must not be able to confirm a pseudonym
/// against them, and the map from party to pseudonym is exactly the
/// re-attribution the linkage domain exists to hold apart.
const SUBJECT_PSEUDONYM_LABEL: &str = "ferroehr:subject-pseudonym:v1";

/// The pseudonymisation domain a subkey belongs to.
///
/// The three domains are the ones the storage layer separates: the clinical
/// record, the identities of its subjects, and the map between them. Binding
/// the domain into the derivation makes a subkey usable in exactly one of them,
/// so a key that escapes with one domain's backup opens nothing in another.
///
/// **No openEHR spec governs this — our own design/extension.** The obligation
/// is GDPR Art. 4(5) and Art. 32(1)(a)
/// (<https://eur-lex.europa.eu/eli/reg/2016/679/oj>) as the EDPB reads them in
/// Guidelines 01/2025 §2: a pseudonymisation domain is defined by who can
/// re-identify within it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyDomain {
    /// The clinical record (`ehr`, `cold`).
    Ehr,
    /// The identities of record subjects (the `party` domain).
    Demographic,
    /// The party-to-EHR resolve map.
    Linkage,
}

impl KeyDomain {
    /// The domain's stable name in a derivation context.
    ///
    /// These strings are key material inputs, so they never change: renaming
    /// one silently re-derives every subkey in that domain.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            KeyDomain::Ehr => "ehr",
            KeyDomain::Demographic => "demographic",
            KeyDomain::Linkage => "linkage",
        }
    }
}

/// What went wrong protecting or resolving an identifier.
///
/// Deliberately says nothing about the value: an error text travels into logs
/// and error bodies, which is exactly where a national identifier must not go.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    /// The configured root key is not 32 bytes of hex.
    #[error(
        "the national-identifier root key must be 64 hex characters (32 bytes); \
         generate one with `openssl rand -hex 32` or an equivalent"
    )]
    RootKey,
    /// The ciphertext did not authenticate under this domain's key.
    ///
    /// A wrong key, a wrong scheme, or a tampered record — AEAD
    /// cannot distinguish them, and neither should this message.
    #[error("the stored identifier could not be decrypted under this deployment's key")]
    Open,
    /// The cipher refused to seal (an impossible-in-practice arm the AEAD API
    /// still declares).
    #[error("the identifier could not be encrypted")]
    Seal,
}

/// The root key material, held as a secret so it never renders.
#[derive(Debug)]
pub struct RootKey(secrecy::SecretBox<[u8; 32]>);

impl RootKey {
    /// Parse a 64-character hex string into the root key.
    ///
    /// # Errors
    /// [`CryptoError::RootKey`] when the text is not exactly 32 bytes of hex.
    pub fn from_hex(hex: &secrecy::SecretString) -> Result<Self, CryptoError> {
        let text = hex.expose_secret().trim();
        if text.len() != 64 {
            return Err(CryptoError::RootKey);
        }
        let mut bytes = [0_u8; 32];
        for (i, slot) in bytes.iter_mut().enumerate() {
            let pair = text.get(i * 2..i * 2 + 2).ok_or(CryptoError::RootKey)?;
            // The cause is dropped on purpose: it names the offending characters,
            // and the key is a secret.
            *slot = u8::from_str_radix(pair, 16).map_err(|_bad_hex| CryptoError::RootKey)?;
        }
        Ok(Self(secrecy::SecretBox::new(Box::new(bytes))))
    }

    /// The opaque subject pseudonym `party` is known by on the clinical side.
    ///
    /// `HMAC-SHA-256(subkey, party)` truncated to sixteen bytes and stamped as
    /// an RFC 9562 version-8 UUID, so it has the shape the subject rule admits
    /// and the same party always yields the same pseudonym. The subkey is
    /// derived under [`KeyDomain::Linkage`] with its own label, so neither the
    /// demographic digests nor any other purpose shares it.
    #[must_use]
    pub fn subject_pseudonym(&self, party: Uuid) -> Uuid {
        let key = self.subkey(SUBJECT_PSEUDONYM_LABEL, KeyDomain::Linkage);
        let tag = keyed_tag(&key, party.as_bytes());
        let mut bytes = [0_u8; 16];
        bytes.copy_from_slice(tag.get(..16).unwrap_or(&[0_u8; 16]));
        uuid::Builder::from_custom_bytes(bytes).into_uuid()
    }

    /// Derive the per-domain subkey for one labelled purpose.
    ///
    /// SP 800-108 KDF in counter mode with HMAC-SHA-256 as the PRF, one block:
    /// `PRF(root, 0x00000001 || label || 0x00 || domain || L)`. One block is
    /// the whole output because the derived key is 256 bits, exactly the PRF's
    /// width.
    fn subkey(&self, label: &str, domain: KeyDomain) -> [u8; 32] {
        // The PRF is keyed with a fixed-size root key, so the construction
        // cannot fail on key length.
        #[expect(
            clippy::expect_used,
            reason = "HMAC accepts a key of any length and the root key is a fixed 32 bytes, \
                      so this constructor has no reachable failure"
        )]
        let mut mac =
            <Hmac<Sha256> as HmacKeyInit>::new_from_slice(self.0.expose_secret().as_slice())
                .expect("HMAC should accept a 32-byte key");
        mac.update(&1_u32.to_be_bytes());
        mac.update(label.as_bytes());
        mac.update(&[0x00]);
        mac.update(domain.as_str().as_bytes());
        mac.update(&256_u32.to_be_bytes());
        mac.finalize().into_bytes().into()
    }
}

/// `HMAC-SHA-256(key, message)`, the PRF every derivation here rests on.
fn keyed_tag(key: &[u8; 32], message: &[u8]) -> [u8; 32] {
    #[expect(
        clippy::expect_used,
        reason = "HMAC accepts a key of any length and the key is a fixed 32 bytes, so this \
                  constructor has no reachable failure"
    )]
    let mut mac = <Hmac<Sha256> as HmacKeyInit>::new_from_slice(key)
        .expect("HMAC should accept a 32-byte key");
    mac.update(message);
    mac.finalize().into_bytes().into()
}

/// The per-domain keys one deployment protects identifiers with.
///
/// Built per write or resolve rather than cached: the derivation is one HMAC
/// block, and a cache would be one more place key material lives.
#[derive(Debug)]
pub struct DomainKeys {
    cipher: [u8; 32],
    lookup: [u8; 32],
}

impl DomainKeys {
    /// Derive both subkeys for one domain from `root`.
    ///
    /// The domain is part of the derivation, so the same root key yields
    /// unrelated subkeys in the clinical and demographic domains and neither
    /// can read the other's records.
    #[must_use]
    pub fn derive(root: &RootKey, domain: KeyDomain) -> Self {
        Self {
            cipher: root.subkey(CIPHER_KEY_LABEL, domain),
            lookup: root.subkey(LOOKUP_KEY_LABEL, domain),
        }
    }

    /// The blind-lookup digest of `value` under this domain's lookup key.
    ///
    /// Deterministic, so equality search works; keyed, so the digest cannot be
    /// reversed by enumerating a national identifier's small value space. The
    /// scheme is bound in, so the same digits under two schemes are two
    /// different digests.
    ///
    /// # Panics
    /// Never: HMAC accepts a key of any length and this one is a fixed 32
    /// bytes, so the keying step has no reachable failure.
    #[must_use]
    pub fn lookup_digest(&self, scheme: &str, value: &str) -> Vec<u8> {
        #[expect(
            clippy::expect_used,
            reason = "HMAC accepts a key of any length and this one is a fixed 32 bytes, \
                      so this constructor has no reachable failure"
        )]
        let mut mac = <Hmac<Sha256> as HmacKeyInit>::new_from_slice(&self.lookup)
            .expect("HMAC should accept a 32-byte key");
        mac.update(scheme.as_bytes());
        mac.update(&[0x00]);
        mac.update(value.as_bytes());
        mac.finalize().into_bytes().to_vec()
    }

    /// Seal `value`, returning the nonce and the ciphertext.
    ///
    /// The scheme is the associated data, so a record moved between schemes
    /// fails authentication instead of opening into a value that means
    /// something else.
    ///
    /// # Errors
    /// [`CryptoError::Seal`] if the AEAD refuses, which has no reachable cause
    /// for a value of this size.
    pub fn seal(&self, scheme: &str, value: &str) -> Result<(Vec<u8>, Vec<u8>), CryptoError> {
        let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(self.cipher));
        let nonce_bytes: [u8; 12] = rand::random();
        let nonce = Nonce::from(nonce_bytes);
        let aad = associated_data(scheme);
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: value.as_bytes(),
                    aad: &aad,
                },
            )
            .map_err(|_aead| CryptoError::Seal)?;
        Ok((nonce_bytes.to_vec(), ciphertext))
    }

    /// Open a sealed identifier.
    ///
    /// # Errors
    /// [`CryptoError::Open`] when the record does not authenticate under this
    /// domain's key and this scheme, whatever the reason.
    pub fn open(
        &self,
        scheme: &str,
        nonce: &[u8],
        ciphertext: &[u8],
    ) -> Result<String, CryptoError> {
        let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(self.cipher));
        if nonce.len() != 12 {
            return Err(CryptoError::Open);
        }
        let aad = associated_data(scheme);
        let plaintext = cipher
            .decrypt(
                &Nonce::try_from(nonce).map_err(|_len| CryptoError::Open)?,
                Payload {
                    msg: ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_aead| CryptoError::Open)?;
        String::from_utf8(plaintext).map_err(|_not_utf8| CryptoError::Open)
    }
}

/// The AEAD associated data: the scheme this record belongs to.
fn associated_data(scheme: &str) -> Vec<u8> {
    let mut aad = Vec::with_capacity(scheme.len() + 1);
    aad.extend_from_slice(scheme.as_bytes());
    aad.push(0x00);
    aad
}

#[cfg(test)]
mod tests {
    //! Every value here is synthetic. The BSN-shaped digits were constructed by
    //! running the elfproef forward over a chosen prefix; no register issues
    //! them.

    use super::{CryptoError, DomainKeys, KeyDomain, RootKey, associated_data};

    const ROOT: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    const OTHER_ROOT: &str = "0f0e0d0c0b0a09080706050403020100f1e2d3c4b5a697887970615243342516";
    const VALUE: &str = "111222333"; // privacy-allow: synthetic, passes the eleven-test

    fn root(hex: &str) -> RootKey {
        RootKey::from_hex(&secrecy::SecretString::from(hex.to_owned())).expect("a 32-byte key")
    }

    #[test]
    fn a_sealed_identifier_opens_to_the_same_value() {
        let keys = DomainKeys::derive(&root(ROOT), KeyDomain::Demographic);
        let (nonce, ciphertext) = keys.seal("nl-bsn", VALUE).expect("seal");
        assert_ne!(
            ciphertext.as_slice(),
            VALUE.as_bytes(),
            "the stored bytes must not be the value"
        );
        let opened = keys.open("nl-bsn", &nonce, &ciphertext).expect("open");
        assert_eq!(opened, VALUE);
    }

    #[test]
    fn every_seal_of_one_value_differs() {
        // A deterministic ciphertext would leak equality of identifiers across
        // records, which is exactly what the separate lookup digest is for.
        let keys = DomainKeys::derive(&root(ROOT), KeyDomain::Demographic);
        let (first_nonce, first) = keys.seal("nl-bsn", VALUE).expect("seal");
        let (second_nonce, second) = keys.seal("nl-bsn", VALUE).expect("seal");
        assert_ne!(first, second, "two seals of one value must differ");
        assert_ne!(first_nonce, second_nonce, "each record gets a fresh nonce");
    }

    #[test]
    fn the_associated_data_is_the_scheme_alone() {
        // The instance is single-tenant, so the associated data binds a record
        // to its SCHEME and nothing else. Pinning the bytes makes the format a
        // decision rather than an accident: changing them makes every stored
        // record unopenable, so the change has to be deliberate and migrated.
        assert_eq!(associated_data("nl-bsn"), b"nl-bsn\x00".to_vec());
        assert_eq!(associated_data(""), vec![0x00]);
    }

    #[test]
    fn a_record_does_not_open_under_another_key_or_scheme() {
        let keys = DomainKeys::derive(&root(ROOT), KeyDomain::Demographic);
        let (nonce, ciphertext) = keys.seal("nl-bsn", VALUE).expect("seal");

        let other_deployment = DomainKeys::derive(&root(OTHER_ROOT), KeyDomain::Demographic);
        assert!(
            matches!(
                other_deployment.open("nl-bsn", &nonce, &ciphertext),
                Err(CryptoError::Open)
            ),
            "another deployment's key must not open it"
        );

        // The scheme is bound in as associated data, so even the right key
        // refuses when the record is read under the wrong label.
        assert!(
            matches!(
                keys.open("se-personnummer", &nonce, &ciphertext),
                Err(CryptoError::Open)
            ),
            "a record read under another scheme must not open"
        );
    }

    #[test]
    fn the_clinical_domain_key_cannot_read_a_demographic_record() {
        // The property #3157 asks for: even where an operator configures ONE
        // root key, the clinical domain's key material opens nothing in the
        // demographic domain, so a clinical backup and a demographic backup
        // are separate artefacts under separate keys rather than two files.
        let demographic = DomainKeys::derive(&root(ROOT), KeyDomain::Demographic);
        let clinical = DomainKeys::derive(&root(ROOT), KeyDomain::Ehr);
        let linkage = DomainKeys::derive(&root(ROOT), KeyDomain::Linkage);
        let (nonce, ciphertext) = demographic.seal("nl-bsn", VALUE).expect("seal");

        for (other, domain) in [(&clinical, "clinical"), (&linkage, "linkage")] {
            assert!(
                matches!(
                    other.open("nl-bsn", &nonce, &ciphertext),
                    Err(CryptoError::Open)
                ),
                "the {domain} domain's key must not open a demographic record"
            );
            assert_ne!(
                other.lookup_digest("nl-bsn", VALUE),
                demographic.lookup_digest("nl-bsn", VALUE),
                "the {domain} domain must not be able to reproduce the lookup digest either, \
                 which is what would let it ask whether a known identifier is present"
            );
        }

        // And the same-root/same-domain derivation is stable, so the refusals
        // above are the domain doing the work rather than a derivation that
        // never reproduces anything.
        let again = DomainKeys::derive(&root(ROOT), KeyDomain::Demographic);
        assert_eq!(
            again
                .open("nl-bsn", &nonce, &ciphertext)
                .expect("the demographic domain's own key opens it"),
            VALUE
        );
    }

    #[test]
    fn each_domain_name_is_its_own_derivation_context() {
        // A rename would silently re-derive every subkey in that domain, so the
        // strings are pinned here as the key-material inputs they are.
        assert_eq!(KeyDomain::Ehr.as_str(), "ehr");
        assert_eq!(KeyDomain::Demographic.as_str(), "demographic");
        assert_eq!(KeyDomain::Linkage.as_str(), "linkage");
    }

    #[test]
    fn a_tampered_ciphertext_is_refused() {
        let keys = DomainKeys::derive(&root(ROOT), KeyDomain::Demographic);
        let (nonce, mut ciphertext) = keys.seal("nl-bsn", VALUE).expect("seal");
        ciphertext[0] ^= 0x01;
        assert!(
            matches!(
                keys.open("nl-bsn", &nonce, &ciphertext),
                Err(CryptoError::Open)
            ),
            "AEAD authentication must refuse a flipped bit"
        );
    }

    #[test]
    fn the_lookup_digest_is_deterministic_keyed_and_scheme_bound() {
        let keys = DomainKeys::derive(&root(ROOT), KeyDomain::Demographic);
        let digest = keys.lookup_digest("nl-bsn", VALUE);
        assert_eq!(
            digest,
            keys.lookup_digest("nl-bsn", VALUE),
            "equality search needs the same value to give the same digest"
        );
        assert_eq!(digest.len(), 32, "HMAC-SHA-256 is 32 bytes");
        assert_ne!(
            digest,
            keys.lookup_digest("se-personnummer", VALUE),
            "the scheme is bound in, so the same digits under two schemes differ"
        );
        assert_ne!(
            digest,
            DomainKeys::derive(&root(OTHER_ROOT), KeyDomain::Demographic)
                .lookup_digest("nl-bsn", VALUE),
            "the digest is keyed: another deployment cannot reproduce it"
        );
        assert!(
            !digest.windows(VALUE.len()).any(|w| w == VALUE.as_bytes()),
            "the digest must not carry the value"
        );
    }

    #[test]
    fn the_cipher_and_lookup_subkeys_are_different_keys() {
        // The lookup key is handed to components that must search without being
        // able to decrypt, so the two derivations must not coincide.
        let keys = DomainKeys::derive(&root(ROOT), KeyDomain::Demographic);
        assert_ne!(
            keys.cipher, keys.lookup,
            "one label per purpose, so a searcher never holds the opener"
        );
    }

    #[test]
    fn a_root_key_of_the_wrong_shape_is_refused_at_parse() {
        for bad in ["", "abcd", &"z".repeat(64), &"ab".repeat(31)] {
            assert!(
                matches!(
                    RootKey::from_hex(&secrecy::SecretString::from(bad.to_owned())),
                    Err(CryptoError::RootKey)
                ),
                "{bad:?} is not a 32-byte hex key"
            );
        }
    }
}
