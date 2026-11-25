// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `OpenPGP` signing key and its detached signatures.
//!
//! RM common `master06-change_control_package.adoc` §Digital Signature names
//! openPGP for the signature; RFC 9580 is the current `OpenPGP` standard. The
//! key is ML-DSA-65+Ed25519 (RFC 9980 algorithm ID 30), signing through
//! `ferrohealth-openpgp-pq`; a classical key is refused at boot. Every
//! certificate verifies through [`crate::openpgp`], which keeps the classical
//! signatures already stored verifiable against retired certificates.

use std::path::Path;

use ferrohealth_openpgp_pq::composite::ALGORITHM_ID;
use ferrohealth_openpgp_pq::encoding::{self, Encoding};
use ferrohealth_openpgp_pq::error::Error as CompositeError;
use ferrohealth_openpgp_pq::key::{Key, Signer};
use ferrohealth_openpgp_pq::mode::Mode;
use pgp::composed::{Deserializable, DetachedSignature, SignedPublicKey, SignedSecretKey};
use pgp::crypto::public_key::PublicKeyAlgorithm;
use pgp::types::KeyDetails as _;
use pgp::types::Password;

use crate::openpgp::Certificate;

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
    /// The key cannot produce a signature: the fail-closed boot check.
    #[error("the configured OpenPGP key cannot sign")]
    Unusable(#[from] PgpSignError),
    /// The signing key is classical, which FerroEHR does not sign with.
    #[error(
        "the configured OpenPGP signing key {fingerprint} is {algorithm:?}, a classical key: \
         FerroEHR signs only with ML-DSA-65+Ed25519 (RFC 9980 algorithm ID 30). Generate one \
         with `ferrohealth-sop generate-key --signing-only`, point record.signing.key_file at it, \
         and list this certificate's public half in record.signing.retired_key_files so the \
         versions it signed keep verifying"
    )]
    ClassicalKey {
        /// The fingerprint of the key, upper-case hex.
        fingerprint: String,
        /// Its public-key algorithm.
        algorithm: PublicKeyAlgorithm,
    },
}

/// A failure producing a detached `OpenPGP` signature at runtime.
#[derive(Debug, thiserror::Error)]
pub enum PgpSignError {
    /// The ML-DSA-65+Ed25519 signer refused or failed.
    #[error("producing the ML-DSA-65+Ed25519 signature failed")]
    Composite(#[from] CompositeError),
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

/// A loaded ML-DSA-65+Ed25519 signing key, never logged or serialised.
///
/// The key was unlocked once at load, so a signature never derives it from the
/// passphrase again (#5616).
pub struct PgpKey {
    signer: Box<Signer>,
    public: Certificate,
    /// Certificates retired from signing and kept to verify what they signed.
    retired: Vec<Certificate>,
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
        Self::from_parts(&armored, passphrase, retired)
    }

    /// Load an armored secret key from an in-memory string, with the
    /// verify-only certificates retired from signing.
    ///
    /// # Errors
    /// [`KeyError::Parse`] if the armor is not a secret key;
    /// [`KeyError::ClassicalKey`] for a key that is not ML-DSA-65+Ed25519;
    /// [`KeyError::Composite`] for a key or certificate that fails its checks
    /// or does not unlock; [`KeyError::Unusable`] if the boot check signature
    /// fails.
    pub fn from_parts(
        armored: &str,
        passphrase: Option<&ferrohealth_secret::secret::Secret>,
        retired: Vec<SignedPublicKey>,
    ) -> Result<Self, KeyError> {
        let (secret, _headers) = SignedSecretKey::from_string(armored).map_err(KeyError::Parse)?;
        if secret.primary_key.algorithm() != PublicKeyAlgorithm::from(ALGORITHM_ID) {
            return Err(KeyError::ClassicalKey {
                fingerprint: format!("{:X}", secret.primary_key.fingerprint()),
                algorithm: secret.primary_key.algorithm(),
            });
        }
        let retired = retired
            .into_iter()
            .map(Certificate::from_signed)
            .collect::<Result<Vec<_>, _>>()
            .map_err(KeyError::Composite)?;
        // NOTE: the passphrase leaves its zeroizing wrapper only for `Password`, which zeroizes too.
        let password =
            passphrase.map_or_else(Password::empty, |secret| Password::from(secret.expose()));
        let key = Key::from_signed(secret).map_err(KeyError::Composite)?;
        let signer = key.unlock(&password).map_err(KeyError::Composite)?;
        let key = Self {
            signer: Box::new(signer),
            public: Certificate::Composite(Box::new(key.certificate().clone())),
            retired,
        };
        // NOTE: fail-closed; a real signature proves the key unlocks and signs.
        key.sign(b"ferroehr-signing boot check")?;
        Ok(key)
    }

    /// The fingerprint of the certificate, upper-case hex.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        format!("{:X}", self.public.fingerprint())
    }

    /// Produce an ASCII-armored version 6 ML-DSA-65+Ed25519 detached signature
    /// over `data`, over SHA2-512.
    ///
    /// # Errors
    /// [`PgpSignError`] if signing or armoring fails.
    pub fn sign(&self, data: &[u8]) -> Result<String, PgpSignError> {
        let signature = self.signer.sign(data, Mode::Binary)?;
        let armored = encoding::encode(&[signature], Encoding::Armored)?;
        Ok(String::from_utf8(armored)?)
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

#[cfg(test)]
mod tests {
    use ferrohealth_secret::secret::Secret;
    use pgp::composed::{ArmorOptions, KeyType, SecretKeyParamsBuilder};
    use rand::rngs::OsRng;

    use super::*;

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
        let loaded = PgpKey::from_parts(&armored(&key), None, Vec::new()).expect("loads");
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
        let text = armored(&composite(Some("correct horse")));
        PgpKey::from_parts(&text, Some(&Secret::new("correct horse")), Vec::new()).expect("loads");
        assert!(matches!(
            PgpKey::from_parts(&text, Some(&Secret::new("wrong")), Vec::new()),
            Err(KeyError::Composite(_))
        ));
    }

    /// A classical key is refused, naming it, and its certificate kept as
    /// retired still verifies what it signed.
    #[test]
    fn a_classical_key_is_refused_and_its_certificate_still_verifies() {
        let mut builder = SecretKeyParamsBuilder::default();
        builder
            .key_type(KeyType::Ed25519Legacy)
            .can_sign(true)
            .primary_user_id("ferroehr-signing classical <test@ferroehr.local>".into());
        let classical = builder
            .build()
            .expect("key params")
            .generate(OsRng)
            .expect("a key");
        let text = classical
            .to_armored_string(ArmorOptions::default())
            .expect("armor");
        let Err(KeyError::ClassicalKey {
            fingerprint,
            algorithm,
        }) = PgpKey::from_parts(&text, None, Vec::new())
        else {
            panic!("a classical key must be refused");
        };
        assert_eq!(algorithm, PublicKeyAlgorithm::EdDSALegacy);
        assert_eq!(
            fingerprint,
            format!("{:X}", classical.primary_key.fingerprint())
        );
        let stored = DetachedSignature::sign_binary_data(
            OsRng,
            &classical.primary_key,
            &Password::empty(),
            pgp::crypto::hash::HashAlgorithm::Sha256,
            &b"version"[..],
        )
        .expect("a stored signature")
        .to_armored_string(ArmorOptions::default())
        .expect("armor");
        let new = PgpKey::from_parts(
            &armored(&composite(None)),
            None,
            vec![classical.to_public_key()],
        )
        .expect("new key");
        assert_eq!(new.verify(b"version", &stored), PgpVerdict::Valid);
        assert_eq!(new.verify(b"versioN", &stored), PgpVerdict::Invalid);
    }
}
