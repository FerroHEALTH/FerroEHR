// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `OpenPGP` key loading + the rPGP detached-signature primitives.
//!
//! Spec: RM common `master06-change_control_package.adoc` §Digital Signature —
//! the signature "is generated according to the openPGP standard (IETF RFC
//! 4880)". NOTE (master06 §Digital Signature): an RFC 4880 signature
//! internally hashes the signed data, so a detached signature over the
//! `canonical_form` bytes is the standard-conformant realisation of the spec's
//! "digital signature … created from the hash".

use std::path::Path;

use pgp::composed::{
    ArmorOptions, Deserializable, DetachedSignature, SignedPublicKey, SignedSecretKey,
    SignedSecretSubKey,
};
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::public_key::PublicKeyAlgorithm;
use pgp::packet::{PublicKey, Signature, SignatureType, SubpacketData};
use pgp::types::KeyDetails as _;
use pgp::types::{Duration, KeyVersion, Password, SignedUser, Tag, Timestamp};
use rand::rngs::OsRng;

/// The hash algorithm used for `OpenPGP` signatures (SHA-256, matching the digest
/// mode; RM common master06 §Digital Signature leaves the algorithm to the
/// format's self-description).
const HASH: HashAlgorithm = HashAlgorithm::Sha256;

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
    /// The file is not a parseable armored RFC 4880 secret key.
    #[error("parsing armored OpenPGP secret key failed")]
    Parse(#[source] pgp::errors::Error),
    /// The key cannot produce a signature (wrong passphrase, or a non-signing
    /// key) — the fail-closed boot check.
    #[error("the configured OpenPGP key cannot sign (wrong passphrase or non-signing key)")]
    Unusable(#[from] PgpSignError),
    /// The signing component is an ECDSA or Ed448 key, whose private-key
    /// arithmetic runs `crypto-bigint` 0.5 operations that RUSTSEC-2026-0387
    /// shows can branch on secret values.
    ///
    /// The certificate still verifies the signatures it made when it is
    /// listed in `signing.retired_key_paths`, since verification uses only
    /// public values.
    #[error(
        "the configured OpenPGP signing key is {algorithm:?}, which FerroEHR refuses to sign with: \
         its private-key arithmetic is not constant-time in the crypto-bigint 0.5 it runs on \
         (RUSTSEC-2026-0387, no fixed release). Rotate to an Ed25519 signing key and list this \
         certificate in `signing.retired_key_paths` so versions already signed keep verifying"
    )]
    RefusedAlgorithm {
        /// The public-key algorithm of the signing component.
        algorithm: PublicKeyAlgorithm,
    },
    /// The certificate authorises no key to sign data: no subkey qualifies
    /// and the primary key's self-signature does not carry key flag 0x02, or
    /// the primary key has expired or been revoked.
    #[error(
        "the configured OpenPGP certificate authorises no key to sign data: neither a subkey nor \
         the primary key carries the signing key flag 0x02 (RFC 9580 §5.2.3.29) on a valid, \
         unexpired, unrevoked self-signature. Add a signing subkey, or re-issue the primary \
         key's self-signature with the signing flag (for GnuPG, `gpg --quick-add-key <fpr> \
         ed25519 sign`)"
    )]
    NoSigningComponent,
}

/// A failure producing a detached `OpenPGP` signature at runtime.
#[derive(Debug, thiserror::Error)]
#[error("producing the detached OpenPGP signature failed")]
pub struct PgpSignError(#[from] pgp::errors::Error);

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

/// A loaded `OpenPGP` signing key: the armored secret key (which also carries the
/// public key) plus the passphrase that unlocks it. Never logged/serialised.
pub struct PgpKey {
    secret: SignedSecretKey,
    /// The component chosen to sign: a subkey ([`signing_subkey_index`]),
    /// else the primary key when it may sign ([`primary_may_sign_at`]).
    ///
    /// Signing with a subkey is what makes rotation cheap: the operator issues
    /// a new signing subkey, the certificate retains the old one, and history
    /// keeps verifying against the same configured key with no retired-keyring
    /// entry. An index rather than a clone because the subkey is not `Clone`,
    /// and it stays valid for the lifetime of the owned certificate.
    signing: SigningComponent,
    public: SignedPublicKey,
    /// Certificates retired from signing and retained for verification only, so
    /// versions signed before a rotation keep verifying. Public keys by
    /// construction: a retired certificate can never sign again.
    retired: Vec<SignedPublicKey>,
    password: Password,
}

/// The component of the certificate that signs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SigningComponent {
    /// The secret subkey at this index of `secret_subkeys`.
    Subkey(usize),
    /// The primary key.
    Primary,
}

impl std::fmt::Debug for PgpKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never render key material or the passphrase.
        f.debug_struct("PgpKey").finish_non_exhaustive()
    }
}

impl PgpKey {
    /// Load an armored RFC 4880 secret key from `path`, unlocked by `passphrase`.
    /// Performs the fail-closed boot check (a test signature).
    ///
    /// # Errors
    /// [`KeyError::Read`] / [`KeyError::Parse`] /
    /// [`KeyError::NoSigningComponent`] / [`KeyError::RefusedAlgorithm`] /
    /// [`KeyError::Unusable`].
    pub fn load(
        path: &Path,
        passphrase: Option<&crate::config::secret::Secret>,
        retired_paths: &[std::path::PathBuf],
    ) -> Result<Self, KeyError> {
        let armored = std::fs::read_to_string(path).map_err(|source| KeyError::Read {
            path: path.display().to_string(),
            source,
        })?;
        let mut retired = Vec::with_capacity(retired_paths.len());
        for retired_path in retired_paths {
            let armored =
                std::fs::read_to_string(retired_path).map_err(|source| KeyError::Read {
                    path: retired_path.display().to_string(),
                    source,
                })?;
            let (certificate, _headers) =
                SignedPublicKey::from_string(&armored).map_err(KeyError::Parse)?;
            retired.push(certificate);
        }
        Self::from_parts(&armored, passphrase, retired)
    }

    /// Load an armored RFC 4880 secret key from an in-memory string (used by
    /// tests).
    ///
    /// # Errors
    /// [`KeyError::Parse`] if the armor is not a secret key;
    /// [`KeyError::NoSigningComponent`] if no key of the certificate may sign;
    /// [`KeyError::RefusedAlgorithm`] if it signs with ECDSA or Ed448;
    /// [`KeyError::Unusable`] if the boot check signature fails.
    pub fn from_armored(
        armored: &str,
        passphrase: Option<&crate::config::secret::Secret>,
    ) -> Result<Self, KeyError> {
        Self::from_parts(armored, passphrase, Vec::new())
    }

    /// [`Self::from_armored`] plus the verify-only certificates retired from
    /// signing.
    ///
    /// # Errors
    /// As [`Self::from_armored`].
    pub fn from_parts(
        armored: &str,
        passphrase: Option<&crate::config::secret::Secret>,
        retired: Vec<SignedPublicKey>,
    ) -> Result<Self, KeyError> {
        let (secret, _headers) = SignedSecretKey::from_string(armored).map_err(KeyError::Parse)?;
        let public = secret.to_public_key();
        // The secret is exposed HERE and nowhere earlier: `pgp`'s `Password` is
        // the one consumer that needs the plaintext, so the value stays inside
        // its `secrecy`-backed wrapper — which zeroizes on drop — until this
        // line.
        let password = match passphrase {
            Some(secret) => Password::from(secret.expose()),
            None => Password::empty(),
        };
        let now = Timestamp::now();
        let signing = match signing_subkey_index(&secret, now) {
            Some(index) => SigningComponent::Subkey(index),
            None if primary_may_sign_at(&secret, now) => SigningComponent::Primary,
            None => return Err(KeyError::NoSigningComponent),
        };
        let key = Self {
            secret,
            signing,
            public,
            retired,
            password,
        };
        // NOTE: RUSTSEC-2026-0387; refused before the boot check, so no signature is made with it.
        let algorithm = key.signing_algorithm();
        if refused_signing_algorithm(algorithm) {
            return Err(KeyError::RefusedAlgorithm { algorithm });
        }
        // Fail-closed: a real signature proves the passphrase unlocks the key
        // and it can actually sign.
        key.sign(b"ferroehr-signing boot check")?;
        if let Some(algorithm) = key.key_algorithm_warning() {
            warn_rsa_signing_key(algorithm);
        }
        Ok(key)
    }

    /// The public-key algorithm of the component that actually signs (the
    /// same choice [`Self::sign`] makes).
    fn signing_algorithm(&self) -> PublicKeyAlgorithm {
        match self.signing {
            SigningComponent::Subkey(index) => self.signing_subkey(index).key.algorithm(),
            SigningComponent::Primary => self.secret.primary_key.algorithm(),
        }
    }

    /// The secret subkey at `index`, which [`signing_subkey_index`] chose.
    #[expect(
        clippy::expect_used,
        reason = "the index comes from `signing_subkey_index` over this same owned \
                  `secret_subkeys`, which nothing mutates after construction"
    )]
    fn signing_subkey(&self, index: usize) -> &SignedSecretSubKey {
        self.secret
            .secret_subkeys
            .get(index)
            .expect("the chosen signing subkey should be in the certificate it was chosen from")
    }

    /// Returns the signing algorithm of this key when it is one whose
    /// PRIVATE-key operations carry the Marvin timing sidechannel, `None` for a
    /// key that keeps `rsa` off the signing path.
    ///
    /// The classification is a pure seam over [`Self::signing_algorithm`]; the
    /// boot loader turns a `Some` into the operator-facing warning
    /// ([`warn_rsa_signing_key`]).
    pub(crate) fn key_algorithm_warning(&self) -> Option<PublicKeyAlgorithm> {
        let algorithm = self.signing_algorithm();
        rsa_family(algorithm).then_some(algorithm)
    }

    /// Produce an ASCII-armored RFC 4880 detached signature over `data`.
    ///
    /// # Errors
    /// [`PgpSignError`] if signing or armoring fails.
    pub fn sign(&self, data: &[u8]) -> Result<String, PgpSignError> {
        let sig = match self.signing {
            SigningComponent::Subkey(index) => DetachedSignature::sign_binary_data(
                OsRng,
                &self.signing_subkey(index).key,
                &self.password,
                HASH,
                data,
            ),
            SigningComponent::Primary => DetachedSignature::sign_binary_data(
                OsRng,
                &self.secret.primary_key,
                &self.password,
                HASH,
                data,
            ),
        }
        .map_err(PgpSignError)?;
        sig.to_armored_string(ArmorOptions::default())
            .map_err(PgpSignError)
    }

    /// Verify an armored detached signature over `data` against the public key.
    pub(crate) fn verify(&self, data: &[u8], armored_sig: &str) -> PgpVerdict {
        let Ok((sig, _headers)) = DetachedSignature::from_string(armored_sig) else {
            return PgpVerdict::Malformed;
        };
        for certificate in std::iter::once(&self.public).chain(&self.retired) {
            if verify_against_certificate(&sig, certificate, data) {
                return PgpVerdict::Valid;
            }
        }
        PgpVerdict::Invalid
    }
}

/// Whether `algorithm` signs with RSA, in any of the three spellings RFC 9580
/// §9.1 registers (`RSA`, plus the deprecated sign-only and encrypt-only
/// codepoints an older certificate can still carry).
fn rsa_family(algorithm: PublicKeyAlgorithm) -> bool {
    matches!(
        algorithm,
        PublicKeyAlgorithm::RSA | PublicKeyAlgorithm::RSASign | PublicKeyAlgorithm::RSAEncrypt
    )
}

/// Returns the index of the secret subkey that signs, or `None` when none
/// may.
///
/// No subkey qualifies while the primary key is revoked or expired
/// ([`primary_in_force_at`]). Otherwise a subkey qualifies when its newest valid Subkey Binding (RFC 9580
/// §5.2.3.10) carries key flag 0x02 (§5.2.3.29), neither that binding nor the
/// subkey has expired at `now` (§5.2.3.13), and no valid Subkey Revocation
/// covers it (§5.2.1.12: "A revoked subkey is not to be used"). Among those,
/// the subkey created last signs, the later one in the file on a tie. RFC 9580
/// gives no rule for choosing among several signing subkeys; preferring the
/// newest is our own design, so that adding a signing subkey rotates to it.
fn signing_subkey_index(secret: &SignedSecretKey, now: Timestamp) -> Option<usize> {
    if !primary_in_force_at(secret, now) {
        return None;
    }
    let primary = secret.primary_key.public_key();
    secret
        .secret_subkeys
        .iter()
        .enumerate()
        .filter(|(_, subkey)| may_sign_at(primary, subkey, now))
        .max_by_key(|(_, subkey)| subkey.key.created_at())
        .map(|(index, _)| index)
}

/// Whether `subkey` qualifies to sign at `now`, by the rule of
/// [`signing_subkey_index`].
fn may_sign_at(primary: &PublicKey, subkey: &SignedSecretSubKey, now: Timestamp) -> bool {
    let subkey_public = subkey.key.public_key();
    // NOTE: RFC 9580 §5.2.3.10; a binding that does not verify is not a valid
    // self-signature, so it is skipped rather than refused.
    let valid = || {
        subkey
            .signatures
            .iter()
            .filter(|sig| sig.verify_subkey_binding(primary, subkey_public).is_ok())
    };
    if valid().any(|sig| sig.typ() == Some(SignatureType::SubkeyRevocation)) {
        return false;
    }
    let Some(binding) = valid()
        .filter(|sig| sig.typ() == Some(SignatureType::SubkeyBinding))
        .max_by_key(|sig| sig.created())
    else {
        return false;
    };
    let binding_live = binding
        .created()
        .is_some_and(|created| !expired(created, binding.signature_expiration_time(), now));
    binding.key_flags().sign()
        && binding_live
        && !expired(subkey.key.created_at(), binding.key_expiration_time(), now)
}

/// The primary key's self-signatures that state facts about it.
#[derive(Debug, Clone, Copy)]
struct PrimarySelfSignatures<'a> {
    /// The newest valid Direct Key signature.
    direct: Option<&'a Signature>,
    /// The self-signature the primary key's key flags are read from.
    governing: Option<&'a Signature>,
}

/// The primary key's self-signatures, or `None` when the primary key is not
/// to be used at all: a valid Key Revocation signature covers it (RFC 9580
/// §5.2.1.11: "A revoked key is not to be used"), or it is a version 6 key
/// without a valid Direct Key signature (§5.2.3.10).
///
/// The governing self-signature follows §5.2.3.10, which keeps the primary
/// key's key flags in a Direct Key signature (the version 6 place) or, by
/// version 4 convention, in a User ID self-signature: the newest valid Direct
/// Key signature when it carries a Key Flags subpacket, else the newest valid
/// self-signature on the primary User ID. RFC 9580 gives no precedence
/// between the two places; preferring the Direct Key signature, whose
/// subpackets "apply to the entire key", is our own design.
fn primary_self_signatures(secret: &SignedSecretKey) -> Option<PrimarySelfSignatures<'_>> {
    let primary = secret.primary_key.public_key();
    let details = &secret.details;
    // NOTE: RFC 9580 §5.2.1.11; only a revocation by the key itself is valid.
    let revoked = details.revocation_signatures.iter().any(|sig| {
        sig.typ() == Some(SignatureType::KeyRevocation) && sig.verify_key(primary).is_ok()
    });
    if revoked {
        return None;
    }
    let direct = details
        .direct_signatures
        .iter()
        .filter(|sig| sig.typ() == Some(SignatureType::Key) && sig.verify_key(primary).is_ok())
        .max_by_key(|sig| sig.created());
    if primary.version() == KeyVersion::V6 && direct.is_none() {
        return None;
    }
    let governing = direct
        .filter(|sig| states_key_flags(sig))
        .or_else(|| primary_user_self_signature(primary, &details.users));
    Some(PrimarySelfSignatures { direct, governing })
}

/// Whether the primary key is in force at `now`: usable at all
/// ([`primary_self_signatures`]) and not past the Key Expiration Time its
/// governing or Direct Key self-signature states (RFC 9580 §5.2.3.13, counted
/// from the primary key's creation).
///
/// A subkey signs only while its primary key is in force. RFC 9580 does not
/// say in so many words that a subkey inherits its primary key's revocation
/// or expiry; reading the Subkey Binding (§5.2.1.8) as a statement a revoked
/// or expired primary key can no longer make is our own design.
fn primary_in_force_at(secret: &SignedSecretKey, now: Timestamp) -> bool {
    let created = secret.primary_key.created_at();
    primary_self_signatures(secret).is_some_and(|sigs| {
        // NOTE: our own design; an expiry stated on the Direct Key signature binds
        // even when the primary User ID governs.
        [sigs.direct, sigs.governing]
            .into_iter()
            .flatten()
            .all(|sig| !expired(created, sig.key_expiration_time(), now))
    })
}

/// Whether the primary key may sign data at `now`.
///
/// It may when it is in force ([`primary_in_force_at`]) and its governing
/// self-signature ([`primary_self_signatures`]) carries key flag 0x02
/// (RFC 9580 §5.2.3.29), with neither that self-signature nor the Direct Key
/// signature past its Signature Expiration Time (§5.2.3.18). A self-signature
/// with no Key Flags subpacket states no flags, and §5.2.3.29 counts an
/// unstated flag as zero, so it grants no signing.
fn primary_may_sign_at(secret: &SignedSecretKey, now: Timestamp) -> bool {
    let Some(sigs) = primary_self_signatures(secret) else {
        return false;
    };
    let Some(governing) = sigs.governing else {
        return false;
    };
    let signature_live = |sig: &Signature| {
        sig.created()
            .is_some_and(|created| !expired(created, sig.signature_expiration_time(), now))
    };
    primary_in_force_at(secret, now)
        && governing.key_flags().sign()
        && signature_live(governing)
        && sigs.direct.is_none_or(signature_live)
}

/// Whether `sig` carries a Key Flags subpacket in its hashed area, the only
/// area a self-signature's statements about the key are read from.
fn states_key_flags(sig: &Signature) -> bool {
    sig.config().is_some_and(|config| {
        config
            .hashed_subpackets()
            .any(|subpacket| matches!(subpacket.data, SubpacketData::KeyFlags(_)))
    })
}

/// The newest valid certification self-signature on the primary User ID, or
/// `None` when no User ID carries one.
///
/// A User ID whose newest valid self-signature is followed by a valid
/// Certification Revocation is retired (RFC 9580 §5.2.1.13, §5.2.3.10) and
/// skipped. The primary User ID is the one whose newest self-signature sets
/// Primary User ID, the one with the most recent self-signature when several
/// do (§5.2.3.27). When none does, the User ID with the most recent
/// self-signature stands in: §5.2.3.27 leaves that case open, so this is our
/// own design.
fn primary_user_self_signature<'a>(
    primary: &PublicKey,
    users: &'a [SignedUser],
) -> Option<&'a Signature> {
    let newest = users.iter().filter_map(|user| {
        let valid = || {
            user.signatures.iter().filter(|sig| {
                sig.verify_certification(primary, Tag::UserId, &user.id)
                    .is_ok()
            })
        };
        let certification = valid()
            .filter(|sig| sig.typ() != Some(SignatureType::CertRevocation))
            .max_by_key(|sig| sig.created())?;
        let retired = valid().any(|sig| {
            sig.typ() == Some(SignatureType::CertRevocation)
                && sig.created() >= certification.created()
        });
        (!retired).then_some(certification)
    });
    let newest: Vec<&Signature> = newest.collect();
    newest
        .iter()
        .copied()
        .filter(|sig| sig.is_primary())
        .max_by_key(|sig| sig.created())
        .or_else(|| newest.iter().copied().max_by_key(|sig| sig.created()))
}

/// Whether a lifetime of `lifetime` from `start` has run out at `now`; an
/// absent or zero lifetime never runs out (RFC 9580 §5.2.3.13).
fn expired(start: Timestamp, lifetime: Option<Duration>, now: Timestamp) -> bool {
    match lifetime {
        Some(lifetime) if lifetime.as_secs() > 0 => {
            u64::from(start.as_secs()) + u64::from(lifetime.as_secs()) <= u64::from(now.as_secs())
        }
        _ => false,
    }
}

/// Whether `algorithm` signs with private-key arithmetic over `crypto-bigint`
/// 0.5: ECDSA on any curve (`elliptic-curve` 0.13 scalars) and Ed448 (`cx448`).
///
/// Ed25519 runs on `curve25519-dalek` and RSA on `num-bigint-dig`, so neither
/// reaches the crate.
fn refused_signing_algorithm(algorithm: PublicKeyAlgorithm) -> bool {
    matches!(
        algorithm,
        PublicKeyAlgorithm::ECDSA | PublicKeyAlgorithm::Ed448
    )
}

/// Warns that the configured signing key performs RSA private-key operations.
///
/// A warning rather than a boot refusal: an operator whose corpus is already
/// RSA-signed must keep the key configured to verify that history (a
/// `VERSION.signature` is an immutable committed fact — RM common
/// `master06-change_control_package.adoc` §Digital Signature), so refusing the
/// key would strand them with no read path at all.
fn warn_rsa_signing_key(algorithm: PublicKeyAlgorithm) {
    tracing::warn!(
        key_algorithm = ?algorithm,
        advisory = "RUSTSEC-2023-0071",
        "the configured OpenPGP signing key is RSA, so every signature is an RSA private-key \
         operation — the operation the Marvin timing sidechannel (RUSTSEC-2023-0071 / \
         CVE-2023-49092) concerns, and the `rsa` crate has no fixed release. Rotate to an Ed25519 \
         signing key and keep this certificate in `signing.retired_key_paths` so versions \
         already signed keep verifying."
    );
}

/// Verify `sig` against every component of one certificate — the primary key
/// and each of its subkeys.
///
/// The subkeys are not an optimisation: RFC 9580 §10.1 defines a transferable
/// public key as a primary key plus its subkeys, and rotating the *signing
/// subkey* is the mechanism `OpenPGP` provides for rotation without discarding
/// the primary key's accumulated trust. `rpgp`'s `VerifyingKey for
/// SignedPublicKey` consults the primary key alone, so a signature made by any
/// subkey would be rejected by a certificate that legitimately contains it.
fn verify_against_certificate(
    sig: &DetachedSignature,
    certificate: &SignedPublicKey,
    data: &[u8],
) -> bool {
    if sig.verify(certificate, data).is_ok() {
        return true;
    }
    certificate
        .public_subkeys
        .iter()
        .any(|subkey| sig.verify(subkey, data).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every RFC 9580 §9.1 RSA codepoint classifies as RSA — including the two
    /// deprecated single-purpose ones, which an older certificate still uses
    /// and which sign with exactly the same private-key operation.
    #[test]
    fn every_rsa_codepoint_is_classified_as_rsa() {
        for algorithm in [
            PublicKeyAlgorithm::RSA,
            PublicKeyAlgorithm::RSASign,
            PublicKeyAlgorithm::RSAEncrypt,
        ] {
            assert!(rsa_family(algorithm), "{algorithm:?} must classify as RSA");
        }
    }

    /// A lifetime runs out at its end, and an absent or zero one never does.
    #[test]
    fn a_lifetime_runs_out_at_its_end() {
        let start = Timestamp::from_secs(1_000);
        let lifetime = Some(Duration::from_secs(100));
        assert!(!expired(start, lifetime, Timestamp::from_secs(1_099)));
        assert!(expired(start, lifetime, Timestamp::from_secs(1_100)));
        assert!(!expired(start, None, Timestamp::from_secs(u32::MAX)));
        assert!(!expired(
            start,
            Some(Duration::from_secs(0)),
            Timestamp::from_secs(u32::MAX)
        ));
    }

    /// ECDSA and Ed448 are refused as signing algorithms; Ed25519, in both
    /// encodings, and RSA are not.
    #[test]
    fn ecdsa_and_ed448_are_refused_signing_algorithms() {
        for algorithm in [PublicKeyAlgorithm::ECDSA, PublicKeyAlgorithm::Ed448] {
            assert!(refused_signing_algorithm(algorithm), "{algorithm:?}");
        }
        for algorithm in [
            PublicKeyAlgorithm::Ed25519,
            PublicKeyAlgorithm::EdDSALegacy,
            PublicKeyAlgorithm::RSA,
        ] {
            assert!(!refused_signing_algorithm(algorithm), "{algorithm:?}");
        }
    }

    /// The algorithms that keep `rsa` off the signing path raise no advisory.
    #[test]
    fn elliptic_curve_algorithms_raise_no_advisory() {
        for algorithm in [
            PublicKeyAlgorithm::Ed25519,
            PublicKeyAlgorithm::Ed448,
            PublicKeyAlgorithm::EdDSALegacy,
            PublicKeyAlgorithm::ECDSA,
            PublicKeyAlgorithm::ECDH,
            PublicKeyAlgorithm::X25519,
        ] {
            assert!(
                !rsa_family(algorithm),
                "{algorithm:?} performs no RSA private-key operation"
            );
        }
    }
}
