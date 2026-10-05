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
};
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::public_key::PublicKeyAlgorithm;
use pgp::types::KeyDetails as _;
use pgp::types::Password;
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
    /// Index into `secret.secret_subkeys` of the signing-capable subkey, when
    /// the certificate carries one.
    ///
    /// Signing with a subkey is what makes rotation cheap: the operator issues
    /// a new signing subkey, the certificate retains the old one, and history
    /// keeps verifying against the same configured key with no retired-keyring
    /// entry. An index rather than a clone because the subkey is not `Clone`,
    /// and it stays valid for the lifetime of the owned certificate.
    signing_subkey: Option<usize>,
    public: SignedPublicKey,
    /// Certificates retired from signing and retained for verification only, so
    /// versions signed before a rotation keep verifying. Public keys by
    /// construction: a retired certificate can never sign again.
    retired: Vec<SignedPublicKey>,
    password: Password,
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
    /// [`KeyError::Read`] / [`KeyError::Parse`] / [`KeyError::Unusable`].
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
    /// [`KeyError::Parse`] if the armor is not a secret key; [`KeyError::Unusable`]
    /// if the boot check signature fails.
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
        // RFC 9580 §5.2.3.29 key flag 0x02 ("this key may be used to sign
        // data"): the subkey is chosen by CAPABILITY, never by position — an
        // encryption subkey signing would be a key-usage violation a strict
        // verifier should reject.
        let signing_subkey = secret
            .secret_subkeys
            .iter()
            .position(|subkey| subkey.signatures.iter().any(|sig| sig.key_flags().sign()));
        let key = Self {
            secret,
            signing_subkey,
            public,
            retired,
            password,
        };
        // Fail-closed: a real signature proves the passphrase unlocks the key
        // and it can actually sign.
        key.sign(b"ferroehr-signing boot check")?;
        if let Some(algorithm) = key.key_algorithm_warning() {
            warn_rsa_signing_key(algorithm);
        }
        Ok(key)
    }

    /// The public-key algorithm of the component that actually signs — the
    /// signing subkey when the certificate carries one, else the primary key
    /// (the same choice [`Self::sign`] makes).
    fn signing_algorithm(&self) -> PublicKeyAlgorithm {
        match self
            .signing_subkey
            .and_then(|index| self.secret.secret_subkeys.get(index))
        {
            Some(subkey) => subkey.key.algorithm(),
            None => self.secret.primary_key.algorithm(),
        }
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
        // A certificate with no signing subkey signs with the primary key.
        let sig = match self
            .signing_subkey
            .and_then(|index| self.secret.secret_subkeys.get(index))
        {
            Some(subkey) => {
                DetachedSignature::sign_binary_data(OsRng, &subkey.key, &self.password, HASH, data)
            }
            None => DetachedSignature::sign_binary_data(
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
         (or other ECC) signing key and keep this certificate in `signing.retired_key_paths` so \
         versions already signed keep verifying."
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
