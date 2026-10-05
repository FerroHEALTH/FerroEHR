// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Read-time verification. We do not persist a signature's provenance (the
//! schema stores only `signature text`), so `verify` classifies by
//! **format**:
//!
//! - a `sha256:` digest is recomputed and compared;
//! - an armored PGP signature is verified against the configured key (if any);
//! - anything else is a foreign/opaque client signature — served as-is.
//!
//! NOTE (RM common `master06-change_control_package.adoc` §Digital Signature):
//! the spec models `VERSION.signature` as a stored fact carried with the data (a
//! signature "created by the committer", potentially in another agreed
//! serialization), so a client-supplied signature — neither our `sha256:` digest
//! nor a PGP signature we hold a key for — is [`Verdict::ClientForeign`] and
//! never a failure; only our own output formats (`sha256:` in any mode, PGP in
//! `pgp` mode) are judged against the recomputed canonical form.

use pgp::composed::{Deserializable as _, DetachedSignature};

use super::key::PgpVerdict;
use super::signer::{DIGEST_PREFIX, SignerMode, digest_signature};

/// The verification outcome for a stored signature against a served Version's
/// `canonical_form` (RM common master06 §Digital Signature).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// A `sha256:` digest that matches the recomputed canonical form.
    DigestMatch,
    /// A `sha256:` digest that does **not** match — the record is corrupt.
    DigestMismatch,
    /// A PGP signature that verifies against the configured key.
    PgpValid,
    /// A PGP signature that does not verify (tampered, or malformed armor).
    PgpInvalid,
    /// A foreign/opaque signature we cannot judge against our canonical form
    /// (a client-supplied signature) — served as-is, never a failure.
    ClientForeign,
}

impl Verdict {
    /// Whether this verdict is an integrity failure (mismatch / invalid) — the
    /// signal `warn` logs+meters and `strict` turns into a 5xx.
    #[must_use]
    pub fn is_failure(self) -> bool {
        matches!(self, Verdict::DigestMismatch | Verdict::PgpInvalid)
    }

    /// A short stable label for the `version_signature_invalid_total{verdict}`
    /// metric and log lines.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Verdict::DigestMatch => "digest_match",
            Verdict::DigestMismatch => "digest_mismatch",
            Verdict::PgpValid => "pgp_valid",
            Verdict::PgpInvalid => "pgp_invalid",
            Verdict::ClientForeign => "client_foreign",
        }
    }
}

/// Marker that a value is an ASCII-armored PGP signature.
const PGP_ARMOR: &str = "-----BEGIN PGP SIGNATURE-----";

/// Classify + verify a stored `signature` against the served `canonical` form.
pub(crate) fn verify(mode: &SignerMode, canonical: &str, signature: &str) -> Verdict {
    if signature.starts_with(DIGEST_PREFIX) {
        return if signature == digest_signature(canonical) {
            Verdict::DigestMatch
        } else {
            Verdict::DigestMismatch
        };
    }
    if signature.trim_start().starts_with(PGP_ARMOR) {
        return match mode {
            SignerMode::Pgp(key) => match key.verify(canonical.as_bytes(), signature) {
                PgpVerdict::Valid => Verdict::PgpValid,
                PgpVerdict::Invalid | PgpVerdict::Malformed => Verdict::PgpInvalid,
            },
            // No key to verify against (digest-mode deployment): a client PGP
            // signature — structural armor check only.
            SignerMode::Digest => {
                if DetachedSignature::from_string(signature).is_ok() {
                    Verdict::ClientForeign
                } else {
                    Verdict::PgpInvalid
                }
            }
        };
    }
    Verdict::ClientForeign
}
