// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[signing]` section — VERSION signing configuration.
//!
//! No openEHR spec governs the configuration surface — our own design. This is
//! a field of the one config tree ([`crate::config::FerroEhrConfig`]); no loader of its own. The modes it
//! selects are the spec-blessed ones (RM common
//! `master06-change_control_package.adoc` §Digital Signature: `OpenPGP`
//! signature or digest-only integrity check).
//!
//! The passphrase is a shared [`crate::config::secret::Secret`] so it can never leak
//! through a config snapshot (`/management/env`) or a `Debug` log; it has a
//! `*_file` sibling for file-based indirection, resolved by the loader.

use std::path::PathBuf;

use crate::config::secret::Secret;
use serde::{Deserialize, Serialize};

/// The signing mode: a data-integrity digest, or an `OpenPGP` (RFC 4880) digital
/// signature (RM common master06 §Digital Signature).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// `radix-64(SHA-256(canonical_form))` — a data-integrity check, no key
    /// management (master07 §Digital Signature: "the encryption step might be
    /// omitted, resulting in a digest only").
    #[default]
    Digest,
    /// An RFC 4880 detached signature over the canonical form with a
    /// server-held `OpenPGP` private key — authentication + non-repudiation.
    Pgp,
}

/// Recompute-and-compare policy at read/reassembly time — for our OWN
/// signatures only (a client-supplied signature is stored verbatim and never
/// re-verified; RM common master06 §Digital Signature).
///
/// Left unset in `[signing]`, the effective policy is resolved by
/// [`SigningConfig::effective_verify_on_read`]: **`strict` when signing is
/// enabled** and `off` when signing is disabled. No openEHR spec governs
/// read-time (re-)verification timing — master06 frames verification as the
/// reader/receiver's role and marks the exact canonical serialisation "To Be
/// Determined" — so this is our own integrity-hardening design, not conformance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VerifyOnRead {
    /// Serve the stored signature untouched (RM common master06 §Digital
    /// Signature models it as a stored fact).
    Off,
    /// Log + meter a mismatch (`version_signature_invalid_total`); still serve.
    Warn,
    /// Recompute-and-compare on EVERY read; a mismatch is a 5xx integrity
    /// failure (the record is provably corrupt).
    Strict,
}

/// Version-signing configuration.
///
/// Every field has a default, so an all-defaults value is valid: signing
/// **on**, `digest` mode, and — with signing enabled — read-time verification
/// **strict** (see [`Self::effective_verify_on_read`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SigningConfig {
    /// Server-side signing of committed versions. Defaults **on** so the
    /// STANDARD "Signing" capability is demonstrably met.
    pub enabled: bool,
    /// Signing mode: `digest` | `pgp`.
    pub mode: Mode,
    /// Armored RFC 4880 secret key; **required for `pgp` mode** (validated at
    /// boot, fail-closed).
    pub key_path: Option<PathBuf>,
    /// Key passphrase, kept in [`crate::config::secret::Secret`] and redacted from any
    /// config snapshot.
    pub key_passphrase: Option<Secret>,
    /// File-based indirection for [`Self::key_passphrase`] (K8s/Docker secrets).
    /// Exactly one of the pair may be set; the loader reads and trims the file.
    pub key_passphrase_file: Option<PathBuf>,
    /// Armored RFC 4880 **public** keys retired from signing but retained for
    /// verification, so versions signed before a key rotation keep verifying.
    ///
    /// A stored `VERSION.signature` records no key identifier, and a signature
    /// is an immutable committed fact (RM common `master06` §Digital Signature)
    /// that cannot be re-issued — so keeping the retired key is the only way
    /// history stays verifiable across a rotation. These are public keys: a
    /// retired key can verify and can never sign again.
    pub retired_key_paths: Vec<PathBuf>,
    /// Read-time verification policy: `off` | `warn` | `strict`. **Unset**
    /// (the default) resolves via [`Self::effective_verify_on_read`] to `strict`
    /// when signing is enabled — the explicit "sign but never check" state is
    /// reachable only by deliberately setting `off`.
    pub verify_on_read: Option<VerifyOnRead>,
}

impl Default for SigningConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: Mode::default(),
            key_path: None,
            key_passphrase: None,
            key_passphrase_file: None,
            retired_key_paths: Vec::new(),
            verify_on_read: None,
        }
    }
}

impl SigningConfig {
    /// The effective read-time verification policy, resolving the enabled-
    /// dependent default of an unset [`Self::verify_on_read`].
    ///
    /// - unset + signing enabled → [`VerifyOnRead::Strict`] (our-own-design
    ///   integrity hardening: a served version whose stored server signature no
    ///   longer recomputes is provably corrupt, so it fails loud rather than
    ///   being silently served);
    /// - unset + signing disabled → [`VerifyOnRead::Off`] (there are no server
    ///   signatures to verify);
    /// - explicit `off` / `warn` / `strict` → honoured as configured.
    ///
    /// No openEHR spec governs read-time verification timing (RM common master06
    /// §Digital Signature) — our own design.
    #[must_use]
    pub fn effective_verify_on_read(&self) -> VerifyOnRead {
        match self.verify_on_read {
            Some(policy) => policy,
            None if self.enabled => VerifyOnRead::Strict,
            None => VerifyOnRead::Off,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_sign_on_digest_verify_strict_when_enabled() {
        let c = SigningConfig::default();
        assert!(c.enabled);
        assert_eq!(c.mode, Mode::Digest);
        // The raw field is unset (None), but the effective policy resolves to
        // strict for a signing-enabled server (#273 — our-own-design hardening).
        assert_eq!(c.verify_on_read, None);
        assert_eq!(c.effective_verify_on_read(), VerifyOnRead::Strict);
        assert!(c.key_path.is_none());
    }

    #[test]
    fn effective_verify_on_read_resolves_the_enabled_dependent_default() {
        // unset + disabled → off (nothing to verify).
        let disabled = SigningConfig {
            enabled: false,
            ..SigningConfig::default()
        };
        assert_eq!(disabled.effective_verify_on_read(), VerifyOnRead::Off);
        // every explicit policy is honoured even with signing enabled.
        for policy in [VerifyOnRead::Off, VerifyOnRead::Warn, VerifyOnRead::Strict] {
            let c = SigningConfig {
                verify_on_read: Some(policy),
                ..SigningConfig::default()
            };
            assert_eq!(c.effective_verify_on_read(), policy);
        }
    }

    #[test]
    fn debug_does_not_leak_passphrase() {
        let c = SigningConfig {
            key_passphrase: Some(Secret::new("top-secret-value")),
            ..SigningConfig::default()
        };
        let dbg = format!("{c:?}");
        assert!(
            !dbg.contains("top-secret-value"),
            "passphrase leaked: {dbg}"
        );
    }

    /// The loader must take the WRAPPER, not a `&str`.
    ///
    /// This is a type-level assertion, and that is the point: if the signature
    /// ever goes back to `Option<&str>`, the passphrase leaves its
    /// zeroizing wrapper at the boundary and an un-zeroized copy survives in
    /// freed memory. A runtime test cannot observe that; the compiler can.
    #[test]
    fn the_key_loader_takes_the_secret_wrapper_not_a_str() {
        type KeyLoader = fn(
            &std::path::Path,
            Option<&Secret>,
            &[PathBuf],
        ) -> Result<
            crate::versioning::signature::key::PgpKey,
            crate::versioning::signature::key::KeyError,
        >;
        fn assert_signature(_f: KeyLoader) {}
        assert_signature(crate::versioning::signature::key::PgpKey::load);
    }
}
