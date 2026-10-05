// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The [`Signer`]: builds the two spec-blessed signature modes from
//! [`SigningConfig`] and produces a signature over a `canonical_form` string.
//!
//! Boot validation is fail-closed: `pgp` mode without a loadable, usable key
//! refuses to construct.
//!
//! Spec: RM common `master06-change_control_package.adoc` §Digital Signature.

use base64::Engine as _;
use sha2::{Digest as _, Sha256};

use super::config::{Mode, SigningConfig, VerifyOnRead};
use super::key::{KeyError, PgpKey, PgpSignError};
use super::verify::{self, Verdict};

/// The self-describing prefix stamped on a digest signature so a bare radix-64
/// hash is not ambiguous.
///
/// Both spec sentences behind this are quoted here because the property they
/// state is the same and the wordings are easy to conflate. RM common
/// `master06-change_control_package.adoc` §Digital Signature makes it a
/// property of the format openEHR chose: "The openPGP standard ensures that the
/// trasformations and algorithms used to create the signature are indicated
/// within it" (upstream's own "trasformations" typo, quoted verbatim). BASE
/// `architecture_overview/master07-security.adoc` §Digital Signature names the
/// property when it gives the reason for that choice: openPGP is "A likely
/// candidate for defining the signature and digest strings in openEHR … due to
/// being an open specification and self-describing".
///
/// This prefix carries that self-description into the DIGEST form, which the
/// same master06 section admits as the other depth of the mechanism ("If only
/// the hashing step is done, the digest acts as a data integrity check") but
/// leaves without a wire spelling of its own — a bare radix-64 hash names
/// neither its algorithm nor its encoding. The `sha256:` prefix is OUR OWN
/// extension: no released openEHR text licenses this token.
pub(crate) const DIGEST_PREFIX: &str = "sha256:";

/// A failure constructing a [`Signer`] at boot.
#[derive(Debug, thiserror::Error)]
pub enum SigningError {
    /// `pgp` mode was selected but no `key_path` was configured.
    #[error("signing mode is `pgp` but no key_path is configured (FERROEHR_SIGNING_KEY_PATH)")]
    MissingKeyPath,
    /// The configured `OpenPGP` key could not be loaded or used.
    #[error("loading the OpenPGP signing key: {0}")]
    Key(#[from] KeyError),
}

/// A failure producing a signature at commit time.
#[derive(Debug, thiserror::Error)]
pub enum SignError {
    /// An `OpenPGP` signing failure.
    #[error(transparent)]
    Pgp(#[from] PgpSignError),
}

/// The resolved signing mode, holding the loaded key for `pgp`.
pub(crate) enum SignerMode {
    /// SHA-256 digest, radix-64 encoded.
    Digest,
    /// `OpenPGP` RFC 4880 detached signature with the loaded key.
    Pgp(Box<PgpKey>),
}

/// The version signer: produces and verifies `VERSION.signature` values
/// (master06 §Digital Signature).
pub struct Signer {
    enabled: bool,
    verify_on_read: VerifyOnRead,
    pub(crate) mode: SignerMode,
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signer")
            .field("enabled", &self.enabled)
            .field("verify_on_read", &self.verify_on_read)
            .field("mode", &self.mode)
            .finish()
    }
}

impl std::fmt::Debug for SignerMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignerMode::Digest => f.write_str("Digest"),
            SignerMode::Pgp(_) => f.write_str("Pgp(..)"),
        }
    }
}

impl Signer {
    /// Build a [`Signer`] from configuration, loading + boot-validating the
    /// `OpenPGP` key when in `pgp` mode (fail-closed).
    ///
    /// # Errors
    /// [`SigningError`] if `pgp` mode has no key path, or the key cannot be
    /// loaded/used.
    pub fn from_config(config: &SigningConfig) -> Result<Self, SigningError> {
        let mode = match config.mode {
            Mode::Digest => SignerMode::Digest,
            Mode::Pgp => {
                let path = config
                    .key_path
                    .as_ref()
                    .ok_or(SigningError::MissingKeyPath)?;
                // Passed as the wrapper, not as `&str`: the plaintext is
                // produced only at the `pgp` call that needs it (see
                // `PgpKey::from_armored`), so no un-zeroized copy exists here.
                SignerMode::Pgp(Box::new(PgpKey::load(
                    path,
                    config.key_passphrase.as_ref(),
                    &config.retired_key_paths,
                )?))
            }
        };
        Ok(Self {
            enabled: config.enabled,
            verify_on_read: config.effective_verify_on_read(),
            mode,
        })
    }

    /// A digest-mode signer with no key handling — the default when no signing
    /// config is present. Mirrors [`SigningConfig::default`]: signing enabled,
    /// `digest` mode, and (enabled ⇒) `verify_on_read = strict`
    /// ([`SigningConfig::effective_verify_on_read`]).
    #[must_use]
    pub fn digest_default() -> Self {
        let config = SigningConfig::default();
        Self {
            enabled: config.enabled,
            verify_on_read: config.effective_verify_on_read(),
            mode: SignerMode::Digest,
        }
    }

    /// Whether server-side signing of committed versions is enabled.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The read-time verification policy.
    #[must_use]
    pub fn verify_on_read(&self) -> VerifyOnRead {
        self.verify_on_read
    }

    /// Sign a Version's `canonical_form` (master06 §Digital Signature): a
    /// SHA-256 digest (`sha256:<radix-64>`) or an armored RFC 4880 detached
    /// signature.
    ///
    /// # Errors
    /// [`SignError`] if `OpenPGP` signing fails (digest signing is infallible).
    pub fn sign(&self, canonical: &str) -> Result<String, SignError> {
        match &self.mode {
            SignerMode::Digest => Ok(digest_signature(canonical)),
            SignerMode::Pgp(key) => Ok(key.sign(canonical.as_bytes())?),
        }
    }

    /// Verify a stored signature against a served Version's `canonical_form`.
    /// See [`Verdict`].
    #[must_use]
    pub fn verify(&self, canonical: &str, signature: &str) -> Verdict {
        verify::verify(&self.mode, canonical, signature)
    }
}

/// The digest signature for `canonical`: `sha256:` + radix-64(SHA-256(bytes)).
pub(crate) fn digest_signature(canonical: &str) -> String {
    let hash = Sha256::digest(canonical.as_bytes());
    format!(
        "{DIGEST_PREFIX}{}",
        base64::engine::general_purpose::STANDARD.encode(hash)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_golden_vector() {
        // Golden vector: `sha256:` + standard-base64 of the raw 32-byte
        // SHA-256 of the input. Pins the digest format + encoding.
        let sig = digest_signature("openehr");
        assert_eq!(sig, "sha256:jtWX/CULavvzX0ehjowv2XZPICTQhN1t0+AXHfbEaNc=");
        assert_eq!(sig, digest_signature("openehr"));
        assert_ne!(sig, digest_signature("openEHR"));
    }
}
