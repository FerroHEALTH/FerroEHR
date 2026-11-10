// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `[privacy.identifier_protection]` — which identifier schemes are
//! protected, and the key they are protected under.
//!
//! **No openEHR spec governs configuration — our own design.**

#![cfg_attr(
    feature = "schema",
    expect(
        clippy::disallowed_types,
        reason = "owner-approved 2026-08-03 (#1694 family 8, OpenAPI schema literals): the \
                  schemars derive builds the JSON Schema of this module's configuration type \
                  with serde_json::Map, which no item-level attribute reaches"
    )
)]

use serde::{Deserialize, Serialize};

/// The national-identifier protection policy.
///
/// Off by default; the table's presence turns it on. Turning it on is a
/// deployment decision with an operational consequence — the key becomes
/// load-bearing for reading the identifiers back — so the server never
/// assumes it. With it on, an identifier in a protected scheme leaves the
/// versioned body for `party.national_identifier` and the body keeps a
/// reference. With it off the body carries the value as written.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct IdentifierProtectionConfig {
    /// The scheme codes to protect
    /// (`FERROEHR__PRIVACY__IDENTIFIER_PROTECTION__SCHEMES`).
    ///
    /// Matched against `DV_IDENTIFIER.type` and against the registry in the
    /// database; a code absent from the registry is a boot error rather than a
    /// silently unprotected identifier.
    pub schemes: Vec<String>,
    /// The root key, 64 hex characters
    /// (`FERROEHR__PRIVACY__IDENTIFIER_PROTECTION__KEY`), or its
    /// `key_file` sibling.
    ///
    /// One key per deployment; the per-domain cipher and lookup subkeys are
    /// derived from it, so rotating it is a re-encryption rather than an edit
    /// (the runbook is in the operations page).
    pub key: Option<ferrohealth_secret::secret::Secret>,
    /// A file holding the root key, read at boot
    /// (`FERROEHR__PRIVACY__IDENTIFIER_PROTECTION__KEY_FILE`).
    pub key_file: Option<std::path::PathBuf>,
}

impl Default for IdentifierProtectionConfig {
    fn default() -> Self {
        Self {
            schemes: vec!["nl-bsn".to_owned()],
            key: None,
            key_file: None,
        }
    }
}
