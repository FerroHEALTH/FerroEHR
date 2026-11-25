// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `OpenPGP` signing key and its detached signatures.
//!
//! RM common `master06-change_control_package.adoc` §Digital Signature names
//! openPGP for the signature; RFC 9580 is the current `OpenPGP` standard. The key
//! signs with ML-DSA-65+Ed25519 (RFC 9980 algorithm ID 30) through
//! `ferrohealth-openpgp-pq`. A classical key signs only behind
//! `record.signing.allow_classical_key`, and every certificate verifies through
//! [`crate::openpgp`], which keeps the signatures already stored verifiable.

mod classical;

use std::path::Path;

use ferrohealth_openpgp_pq::composite::ALGORITHM_ID;
use ferrohealth_openpgp_pq::encoding::{self, Encoding};
use ferrohealth_openpgp_pq::error::Error as CompositeError;
use ferrohealth_openpgp_pq::key::{Key, Signer};
use ferrohealth_openpgp_pq::mode::Mode;
use pgp::composed::{
    ArmorOptions, Deserializable, DetachedSignature, SignedPublicKey, SignedSecretKey,
    SignedSecretSubKey,
};
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::public_key::PublicKeyAlgorithm;
use pgp::types::KeyDetails as _;
use pgp::types::{Password, Timestamp};
use rand::rngs::OsRng;

use crate::openpgp::Certificate;
use classical::{
    primary_may_sign_at, refused_signing_algorithm, rsa_family, signing_subkey_index,
    warn_rsa_signing_key,
};

/// The hash a classical key signs with (RFC 9580 §9.5: SHA2-256 is the one
/// every implementation supports).
const CLASSICAL_HASH: HashAlgorithm = HashAlgorithm::Sha256;

/// A failure loading or exercising the configured `OpenPGP` signing key.
#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    /// The armored key file could not be read.
    #[error("reading OpenPGP key file {path}")]
    Read {
        /// The path that failed.
        path: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The file is not a parseable armored RFC 9580 key or certificate.
    #[error("parsing the armored OpenPGP key failed")]
    Parse(#[source] pgp::errors::Error),
    /// An ML-DSA-65+Ed25519 key or certificate failed its checks (RFC 9980,
    /// RFC 9580 §5.2.3.10), or its passphrase does not unlock it.
    #[error("the ML-DSA-65+Ed25519 OpenPGP key or certificate is not usable")]
    Composite(#[source] CompositeError),
    /// The key cannot produce a signature (wrong passphrase, or a non-signing
    /// key): the fail-closed boot check.
    #[error("the configured OpenPGP key cannot sign (wrong passphrase or non-signing key)")]
    Unusable(#[from] PgpSignError),
    /// The signing key is classical and `record.signing.allow_classical_key`
    /// is off.
    #[error(
        "the configured OpenPGP signing key {fingerprint} is {algorithm:?}, a classical key that \
         FerroEHR signs with only when record.signing.allow_classical_key = true: classical \
         signatures are quantum-vulnerable and high-risk use must complete the move by \
         31.12.2030 (EU coordinated implementation roadmap). Generate an ML-DSA-65+Ed25519 key \
         (`ferrohealth-sop generate-key --signing-only`) and list this certificate in \
         record.signing.retired_key_files so the versions it signed keep verifying"
    )]
    ClassicalKey {
        /// The fingerprint of the key, upper-case hex.
        fingerprint: String,
        /// Its public-key algorithm.
        algorithm: PublicKeyAlgorithm,
    },
    /// The signing component is an ECDSA or Ed448 key, whose private-key
    /// arithmetic runs `crypto-bigint` 0.5 operations that RUSTSEC-2026-0387
    /// shows can branch on secret values.
    #[error(
        "the configured OpenPGP signing key is {algorithm:?}, which FerroEHR refuses to sign with: \
         its private-key arithmetic is not constant-time in the crypto-bigint 0.5 it runs on \
         (RUSTSEC-2026-0387, no fixed release). Generate an ML-DSA-65+Ed25519 key and list this \
         certificate in `record.signing.retired_key_files` so versions already signed keep verifying"
    )]
    RefusedAlgorithm {
        /// The public-key algorithm of the signing component.
        algorithm: PublicKeyAlgorithm,
    },
    /// The classical certificate authorises no key to sign data: no subkey
    /// qualifies and the primary key's self-signature does not carry key flag
    /// 0x02, or the primary key has expired or been revoked.
    #[error(
        "the configured OpenPGP certificate authorises no key to sign data: neither a subkey nor \
         the primary key carries the signing key flag 0x02 (RFC 9580 §5.2.3.29) on a valid, \
         unexpired, unrevoked self-signature. Generate an ML-DSA-65+Ed25519 key \
         (`ferrohealth-sop generate-key --signing-only`), or add a signing subkey"
    )]
    NoSigningComponent,
}

/// A failure producing a detached `OpenPGP` signature at runtime.
#[derive(Debug, thiserror::Error)]
pub enum PgpSignError {
    /// The ML-DSA-65+Ed25519 signer refused or failed.
    #[error("producing the ML-DSA-65+Ed25519 signature failed")]
    Composite(#[from] CompositeError),
    /// rPGP refused or failed a classical signature.
    #[error("producing the classical OpenPGP signature failed")]
    Classical(#[from] pgp::errors::Error),
    /// The armor was not text, which ASCII armor (RFC 9580 §6) always is.
    #[error("the armored signature is not text")]
    Armor(#[from] std::string::FromUtf8Error),
}

/// The structural outcome of verifying a PGP-armored signature against the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PgpVerdict {
    /// A valid detached signature by the configured key over the content.
    Valid,
    /// A well-formed detached signature that does not verify.
    Invalid,
    /// The armor could not be parsed as a detached signature.
    Malformed,
}

/// A loaded `OpenPGP` signing key, never logged or serialised.
///
/// The key was unlocked once at load, so a signature never derives it from the
/// passphrase again (#5616).
pub struct PgpKey {
    signing: SigningKey,
    public: Certificate,
    /// Certificates retired from signing and kept to verify what they signed.
    retired: Vec<Certificate>,
}

/// The key that signs.
enum SigningKey {
    /// An ML-DSA-65+Ed25519 key (RFC 9980 algorithm ID 30).
    Composite(Box<Signer>),
    /// A classical key, signing behind `record.signing.allow_classical_key`.
    Classical {
        secret: Box<SignedSecretKey>,
        component: SigningComponent,
    },
}

/// The component of a classical certificate that signs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SigningComponent {
    /// The secret subkey at this index of `secret_subkeys`.
    Subkey(usize),
    /// The primary key.
    Primary,
}

impl std::fmt::Debug for PgpKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgpKey").finish_non_exhaustive()
    }
}

impl PgpKey {
    /// Load an armored secret key from `path`, unlocked by `passphrase`, with
    /// the certificates at `retired_paths` kept for verification, and make the
    /// fail-closed boot check signature.
    ///
    /// # Errors
    /// [`KeyError::Read`] and [`KeyError::Parse`] for a file that does not
    /// read or parse, and the errors of [`Self::from_parts`].
    pub fn load(
        path: &Path,
        passphrase: Option<&ferrohealth_secret::secret::Secret>,
        retired_paths: &[std::path::PathBuf],
        allow_classical_key: bool,
    ) -> Result<Self, KeyError> {
        let armored = read(path)?;
        let retired = retired_paths
            .iter()
            .map(|retired_path| {
                SignedPublicKey::from_string(&read(retired_path)?)
                    .map(|(certificate, _headers)| certificate)
                    .map_err(KeyError::Parse)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::from_parts(&armored, passphrase, retired, allow_classical_key)
    }

    /// Load an armored secret key from an in-memory string, with the
    /// verify-only certificates retired from signing.
    ///
    /// # Errors
    /// [`KeyError::Parse`] if the armor is not a secret key;
    /// [`KeyError::Composite`] for an ML-DSA-65+Ed25519 key or certificate
    /// that fails its checks or does not unlock; [`KeyError::ClassicalKey`] for
    /// a classical key while `allow_classical_key` is off;
    /// [`KeyError::NoSigningComponent`], [`KeyError::RefusedAlgorithm`] and
    /// [`KeyError::Unusable`] for a classical key that cannot sign.
    pub fn from_parts(
        armored: &str,
        passphrase: Option<&ferrohealth_secret::secret::Secret>,
        retired: Vec<SignedPublicKey>,
        allow_classical_key: bool,
    ) -> Result<Self, KeyError> {
        let (secret, _headers) = SignedSecretKey::from_string(armored).map_err(KeyError::Parse)?;
        let retired = retired
            .into_iter()
            .map(Certificate::from_signed)
            .collect::<Result<Vec<_>, _>>()
            .map_err(KeyError::Composite)?;
        // NOTE: the passphrase leaves its zeroizing wrapper only for `Password`, which zeroizes too.
        let password =
            passphrase.map_or_else(Password::empty, |secret| Password::from(secret.expose()));
        let key = if secret.primary_key.algorithm() == PublicKeyAlgorithm::from(ALGORITHM_ID) {
            let key = Key::from_signed(secret).map_err(KeyError::Composite)?;
            let signer = key.unlock(&password).map_err(KeyError::Composite)?;
            let public = Certificate::Composite(Box::new(key.certificate().clone()));
            Self {
                signing: SigningKey::Composite(Box::new(signer)),
                public,
                retired,
            }
        } else if allow_classical_key {
            Self::classical(secret, &password, retired)?
        } else {
            return Err(KeyError::ClassicalKey {
                fingerprint: format!("{:X}", secret.primary_key.fingerprint()),
                algorithm: secret.primary_key.algorithm(),
            });
        };
        // NOTE: fail-closed; a real signature proves the key unlocks and signs.
        key.sign(b"ferroehr-signing boot check")?;
        Ok(key)
    }

    /// The classical key `secret`, unlocked by `password`, with its refusals
    /// and warnings.
    fn classical(
        mut secret: SignedSecretKey,
        password: &Password,
        retired: Vec<Certificate>,
    ) -> Result<Self, KeyError> {
        let now = Timestamp::now();
        let component = match signing_subkey_index(&secret, now) {
            Some(index) => SigningComponent::Subkey(index),
            None if primary_may_sign_at(&secret, now) => SigningComponent::Primary,
            None => return Err(KeyError::NoSigningComponent),
        };
        // NOTE: RFC 9580 §3.7, the S2K derivation is deliberately slow, so it runs once here.
        let unlocked = match component {
            SigningComponent::Subkey(index) => secret
                .secret_subkeys
                .get_mut(index)
                .map_or(Ok(()), |subkey| subkey.key.remove_password(password)),
            SigningComponent::Primary => secret.primary_key.remove_password(password),
        };
        unlocked.map_err(|error| KeyError::Unusable(PgpSignError::Classical(error)))?;
        let algorithm = match component {
            SigningComponent::Subkey(index) => subkey(&secret, index).key.algorithm(),
            SigningComponent::Primary => secret.primary_key.algorithm(),
        };
        // NOTE: RUSTSEC-2026-0387; refused before the boot check, so no signature is made with it.
        if refused_signing_algorithm(algorithm) {
            return Err(KeyError::RefusedAlgorithm { algorithm });
        }
        let fingerprint = format!("{:X}", secret.primary_key.fingerprint());
        tracing::warn!(
            key_fingerprint = %fingerprint,
            key_algorithm = ?algorithm,
            "record.signing.allow_classical_key is on, so FerroEHR signs with a classical, \
             quantum-vulnerable OpenPGP key; high-risk use must complete the move to post-quantum \
             signatures by 31.12.2030 (EU coordinated implementation roadmap). Generate an \
             ML-DSA-65+Ed25519 key and retire this one"
        );
        if rsa_family(algorithm) {
            warn_rsa_signing_key(algorithm);
        }
        let public = Certificate::Classical(Box::new(secret.to_public_key()));
        Ok(Self {
            signing: SigningKey::Classical {
                secret: Box::new(secret),
                component,
            },
            public,
            retired,
        })
    }

    /// Whether the key signs with ML-DSA-65+Ed25519.
    #[must_use]
    pub fn is_post_quantum(&self) -> bool {
        matches!(self.signing, SigningKey::Composite(_))
    }

    /// The fingerprint of the certificate, upper-case hex.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        format!("{:X}", self.public.fingerprint())
    }

    /// Produce an ASCII-armored detached signature over `data`: a version 6
    /// ML-DSA-65+Ed25519 signature over SHA2-512, or a classical one over
    /// SHA2-256.
    ///
    /// # Errors
    /// [`PgpSignError`] if signing or armoring fails.
    pub fn sign(&self, data: &[u8]) -> Result<String, PgpSignError> {
        match &self.signing {
            SigningKey::Composite(signer) => {
                let signature = signer.sign(data, Mode::Binary)?;
                let armored = encoding::encode(&[signature], Encoding::Armored)?;
                Ok(String::from_utf8(armored)?)
            }
            SigningKey::Classical { secret, component } => {
                let signature = match component {
                    SigningComponent::Subkey(index) => DetachedSignature::sign_binary_data(
                        OsRng,
                        &subkey(secret, *index).key,
                        &Password::empty(),
                        CLASSICAL_HASH,
                        data,
                    ),
                    SigningComponent::Primary => DetachedSignature::sign_binary_data(
                        OsRng,
                        &secret.primary_key,
                        &Password::empty(),
                        CLASSICAL_HASH,
                        data,
                    ),
                }?;
                Ok(signature.to_armored_string(ArmorOptions::default())?)
            }
        }
    }

    /// Verify an armored detached signature over `data` against the key's
    /// certificate and the retired ones.
    pub(crate) fn verify(&self, data: &[u8], armored_sig: &str) -> PgpVerdict {
        let Ok((signature, _headers)) = DetachedSignature::from_string(armored_sig) else {
            return PgpVerdict::Malformed;
        };
        let verified = std::iter::once(&self.public)
            .chain(&self.retired)
            .any(|certificate| certificate.verify(&signature.signature, data).is_ok());
        if verified {
            PgpVerdict::Valid
        } else {
            PgpVerdict::Invalid
        }
    }
}

/// The text of the file at `path`.
fn read(path: &Path) -> Result<String, KeyError> {
    std::fs::read_to_string(path).map_err(|source| KeyError::Read {
        path: path.display().to_string(),
        source,
    })
}

/// The secret subkey at `index`, which `signing_subkey_index` chose.
#[expect(
    clippy::expect_used,
    reason = "the index comes from `signing_subkey_index` over this same owned \
              `secret_subkeys`, which nothing mutates after construction"
)]
fn subkey(secret: &SignedSecretKey, index: usize) -> &SignedSecretSubKey {
    secret
        .secret_subkeys
        .get(index)
        .expect("the chosen signing subkey should be in the certificate it was chosen from")
}

#[cfg(test)]
mod tests {
    use ferrohealth_openpgp_pq::key::Key;
    use ferrohealth_secret::secret::Secret;

    use super::*;

    fn classical(passphrase: Option<&str>) -> String {
        let mut builder = pgp::composed::SecretKeyParamsBuilder::default();
        builder
            .key_type(pgp::composed::KeyType::Ed25519Legacy)
            .can_sign(true)
            .passphrase(passphrase.map(str::to_owned))
            .primary_user_id("ferroehr-signing test <test@ferroehr.local>".into());
        builder
            .build()
            .expect("key params")
            .generate(OsRng)
            .expect("a key")
            .to_armored_string(ArmorOptions::default())
            .expect("armor")
    }

    fn composite(passphrase: Option<&str>) -> Key {
        let mut key =
            Key::generate(&["ferroehr-signing test <test@ferroehr.local>"]).expect("a key");
        if let Some(passphrase) = passphrase {
            key.lock(&Password::from(passphrase)).expect("locked");
        }
        key
    }

    fn armored(key: &Key) -> String {
        String::from_utf8(
            encoding::encode(std::slice::from_ref(key.signed()), Encoding::Armored).expect("armor"),
        )
        .expect("text")
    }

    /// An ML-DSA-65+Ed25519 key signs version 6 signatures that verify, and a
    /// changed payload does not.
    #[test]
    fn a_composite_key_signs_and_verifies() {
        let key = composite(None);
        let loaded = PgpKey::from_parts(&armored(&key), None, Vec::new(), false).expect("loads");
        assert!(loaded.is_post_quantum());
        assert_eq!(loaded.fingerprint(), format!("{:X}", key.fingerprint()));
        let signature = loaded.sign(b"payload").expect("a signature");
        let (parsed, _) = DetachedSignature::from_string(&signature).expect("armored");
        assert_eq!(
            parsed.signature.version(),
            pgp::packet::SignatureVersion::V6
        );
        assert_eq!(loaded.verify(b"payload", &signature), PgpVerdict::Valid);
        assert_eq!(loaded.verify(b"paylOad", &signature), PgpVerdict::Invalid);
        assert_eq!(
            loaded.verify(b"payload", "not armor"),
            PgpVerdict::Malformed
        );
    }

    /// A locked ML-DSA-65+Ed25519 key unlocks with its passphrase only.
    #[test]
    fn a_locked_composite_key_needs_its_passphrase() {
        let key = composite(Some("correct horse"));
        let text = armored(&key);
        PgpKey::from_parts(
            &text,
            Some(&Secret::new("correct horse")),
            Vec::new(),
            false,
        )
        .expect("loads");
        assert!(matches!(
            PgpKey::from_parts(&text, Some(&Secret::new("wrong")), Vec::new(), false),
            Err(KeyError::Composite(_))
        ));
    }

    /// A classical key is refused while `allow_classical_key` is off, naming
    /// the key.
    #[test]
    fn a_classical_key_is_refused_by_default() {
        let text = classical(None);
        let Err(KeyError::ClassicalKey {
            fingerprint,
            algorithm,
        }) = PgpKey::from_parts(&text, None, Vec::new(), false)
        else {
            panic!("a classical key must be refused by default");
        };
        assert_eq!(algorithm, PublicKeyAlgorithm::EdDSALegacy);
        let message = KeyError::ClassicalKey {
            fingerprint: fingerprint.clone(),
            algorithm,
        }
        .to_string();
        assert!(
            message.contains(&fingerprint) && message.contains("31.12.2030"),
            "{message}"
        );
    }

    /// A passphrase-locked classical key, allowed explicitly, is unlocked once
    /// at load and what it signs verifies (#5616).
    #[test]
    fn an_allowed_classical_key_is_unlocked_once_at_load() {
        let text = classical(Some("correct horse"));
        let key = PgpKey::from_parts(&text, Some(&Secret::new("correct horse")), Vec::new(), true)
            .expect("the key loads");
        let SigningKey::Classical { secret, component } = &key.signing else {
            panic!("a classical key");
        };
        assert_eq!(*component, SigningComponent::Primary);
        assert!(matches!(
            secret.primary_key.secret_params(),
            pgp::types::SecretParams::Plain(_)
        ));
        assert!(!key.is_post_quantum());
        let signature = key.sign(b"payload").expect("a signature");
        assert_eq!(key.verify(b"payload", &signature), PgpVerdict::Valid);
        assert!(matches!(
            PgpKey::from_parts(&text, Some(&Secret::new("wrong")), Vec::new(), true),
            Err(KeyError::Unusable(_))
        ));
    }

    /// After a rotation to ML-DSA-65+Ed25519, the signatures the retired
    /// classical key made still verify.
    #[test]
    fn a_retired_classical_certificate_keeps_verifying() {
        let old_text = classical(None);
        let old = PgpKey::from_parts(&old_text, None, Vec::new(), true).expect("old key");
        let stored = old.sign(b"version").expect("a stored signature");
        let (old_secret, _) = SignedSecretKey::from_string(&old_text).expect("parse");
        let new = PgpKey::from_parts(
            &armored(&composite(None)),
            None,
            vec![old_secret.to_public_key()],
            false,
        )
        .expect("new key");
        assert_eq!(new.verify(b"version", &stored), PgpVerdict::Valid);
        assert_eq!(new.verify(b"versioN", &stored), PgpVerdict::Invalid);
    }
}
