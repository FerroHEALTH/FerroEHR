// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `[demographic.identifier_protection]` — which identifier schemes are
//! protected, and the key they are protected under.
//!
//! **No openEHR spec governs configuration — our own design.**

use serde::{Deserialize, Serialize};

/// The national-identifier protection policy.
///
/// Off by default. Turning it on is a deployment decision with an operational
/// consequence — the key becomes load-bearing for reading the identifiers back
/// — so the server never assumes it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IdentifierProtectionConfig {
    /// Whether protection is in force
    /// (`FERROEHR__DEMOGRAPHIC__IDENTIFIER_PROTECTION__ENABLED`).
    ///
    /// With it on, an identifier in a protected scheme leaves the versioned
    /// body for `demographic.national_identifier` and the body keeps a
    /// reference. With it off the body carries the value as written, which is
    /// the pre-existing behaviour.
    pub enabled: bool,
    /// The scheme codes to protect
    /// (`FERROEHR__DEMOGRAPHIC__IDENTIFIER_PROTECTION__SCHEMES`).
    ///
    /// Matched against `DV_IDENTIFIER.type` and against the registry in the
    /// database; a code absent from the registry is a boot error rather than a
    /// silently unprotected identifier.
    pub schemes: Vec<String>,
    /// The root key, 64 hex characters
    /// (`FERROEHR__DEMOGRAPHIC__IDENTIFIER_PROTECTION__KEY`), or its
    /// `key_file` sibling.
    ///
    /// One key per deployment; the per-domain cipher and lookup subkeys are
    /// derived from it, so rotating it is a re-encryption rather than an edit
    /// (the runbook is in the operations page).
    pub key: Option<crate::config::secret::Secret>,
    /// A file holding the root key, read at boot
    /// (`FERROEHR__DEMOGRAPHIC__IDENTIFIER_PROTECTION__KEY_FILE`).
    pub key_file: Option<std::path::PathBuf>,
}

impl Default for IdentifierProtectionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            schemes: vec!["nl-bsn".to_owned()],
            key: None,
            key_file: None,
        }
    }
}

/// `[demographic]` — the demographic domain's own settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DemographicConfig {
    /// `[demographic.identifier_protection]`.
    pub identifier_protection: IdentifierProtectionConfig,
}
