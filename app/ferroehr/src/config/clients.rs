// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The systems whose imported committers are trusted (`[authz.import]`).
//!
//! The system clients themselves, what each may commit, import or export and
//! the provider it acts for, are policy parties in the CDR, read per request
//! (`crate::party_binding::grants`). No openEHR spec governs configuration:
//! our own design.

use serde::{Deserialize, Serialize};

/// The `[authz.import]` table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
pub struct ImportConfig {
    /// The `creating_system_id` values whose wrapped committers may name a
    /// local party; never this node's own system id.
    // TODO(#5005): hold an imported committer naming a local party to these systems.
    pub trusted_systems: Vec<String>,
}

/// The boot refusals of `[authz.import]`: a trusted system is never this
/// node, whose own committers are its own.
#[must_use]
pub fn import_errors(import: &ImportConfig, system_id: &str) -> Vec<String> {
    import
        .trusted_systems
        .iter()
        .filter(|system| system.trim().is_empty() || system.as_str() == system_id)
        .map(|system| {
            format!(
                "authz.import.trusted_systems names {system:?}, which is empty or this node's own \
                 api.system_id"
            )
        })
        .collect()
}
