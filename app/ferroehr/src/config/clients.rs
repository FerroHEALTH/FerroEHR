// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The system clients that commit (`[[authz.system_client]]`) and the systems
//! whose imported committers are trusted (`[authz.import]`).
//!
//! A system commit is declared and checked, never filled: each committing
//! OAuth client-credentials client is declared with the provider it acts for
//! (EHDS Annex II 3.2(a)), the facilities it may commit for, the templates it
//! may write and the origins of what it commits. A Basic user has no issuer,
//! so it is never a system client. No openEHR spec governs configuration: our
//! own design.

use std::collections::BTreeSet;

use ferrohealth_secret::secret_url::SecretUrl;
use ferrohealth_smart::config::Resolved;
use ferrohealth_smart::kind::IssuerKind;
use serde::{Deserialize, Serialize};

use crate::config::binding::ProviderId;

/// One `[[authz.system_client]]` table, keyed by `issuer` and `client_id`.
// TODO(#5008): admit a system commit only from a declared client, within its facilities and writes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct SystemClient {
    /// The `machines` or `persons` issuer of the client's tokens. It renders with any userinfo masked.
    pub issuer: SecretUrl,
    /// The client the token names.
    pub client_id: String,
    /// The healthcare provider the client acts for, recorded on every system
    /// access.
    pub provider: Option<ProviderId>,
    /// The facilities the client may commit for.
    pub facilities: Vec<Facility>,
    /// The template ids the client may commit; none by default.
    pub writes: Vec<String>,
    /// What the client commits: a relayed record of a human act, a device's
    /// registration, or both.
    pub origins: Vec<Origin>,
    /// The issuers of the device identifiers a `device` client names.
    pub device_identifier_issuers: Vec<String>,
}

/// A facility a system client may commit for: an identifier issuer with the
/// id, or with an id prefix.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct Facility {
    /// The issuer of the facility identifier.
    pub issuer: String,
    /// The facility identifier; exclusive with `id_prefix`.
    pub id: Option<String>,
    /// The prefix every admitted facility identifier starts with; exclusive
    /// with `id`.
    pub id_prefix: Option<String>,
}

/// The origin of what a system client commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// It relays another system's record of a human act, such as HL7 v2.
    Relay,
    /// A device registers with no human acting.
    Device,
}

/// The `[authz.import]` table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct ImportConfig {
    /// The `creating_system_id` values whose wrapped committers may name a
    /// local party; never this node's own system id.
    // TODO(#5005): hold an imported committer naming a local party to these systems.
    pub trusted_systems: Vec<String>,
}

/// The boot refusals of `[[authz.system_client]]`, each naming its table.
#[must_use]
pub fn system_client_errors(clients: &[SystemClient], resolved: &Resolved) -> Vec<String> {
    let mut errors = Vec::new();
    let mut seen = BTreeSet::new();
    for client in clients {
        let name = format!(
            "the [[authz.system_client]] table of {:?} and {:?}",
            client.issuer, client.client_id
        );
        match resolved.kinds.get(client.issuer.expose()) {
            Some(IssuerKind::Machines | IssuerKind::Persons) => {}
            Some(kind) => errors.push(format!(
                "{name} names a {kind} issuer: a system client's issuer is of kind machines or \
                 persons"
            )),
            None => errors.push(format!("{name} names no [[auth.issuer]]")),
        }
        if client.client_id.trim().is_empty() {
            errors.push(format!("{name} needs client_id"));
        }
        if !seen.insert((client.issuer.expose(), client.client_id.as_str())) {
            errors.push(format!("{name} is declared twice"));
        }
        if client
            .provider
            .as_ref()
            .is_none_or(|p| p.issuer.trim().is_empty() || p.id.trim().is_empty())
        {
            errors.push(format!("{name} needs provider with issuer and id"));
        }
        for facility in &client.facilities {
            if facility.issuer.trim().is_empty()
                || facility.id.is_some() == facility.id_prefix.is_some()
            {
                errors.push(format!(
                    "{name} has a facility without an issuer and exactly one of id and id_prefix"
                ));
            }
        }
        if client.origins.is_empty() {
            errors.push(format!("{name} needs origins: relay, device or both"));
        }
        if client.origins.contains(&Origin::Device) && client.device_identifier_issuers.is_empty() {
            errors.push(format!(
                "{name} has origins containing device and needs device_identifier_issuers"
            ));
        }
    }
    errors
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
