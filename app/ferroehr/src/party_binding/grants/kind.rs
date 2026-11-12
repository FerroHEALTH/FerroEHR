// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The kinds of grant a policy `ROLE` carries, and the ceiling
//! `[authz.grants] allowed` puts on them.
//!
//! A kind is the policy `ROLE` archetype's kind with the coded `name` of the
//! `CAPABILITY` (`/capabilities[id6]/name`), each code as the default
//! definitions define it. An ADL 2 specialised code (`at10.1`) is of the kind
//! of the code it specialises (AM `ADL2/master09.01-spec_overview.adoc`
//! §Specialisation). No openEHR spec gives a capability an authorization
//! meaning: our own design.

#![cfg_attr(
    feature = "schema",
    expect(
        clippy::disallowed_types,
        reason = "owner-approved 2026-08-03 (#1694 family 8, OpenAPI schema literals): the \
                  schemars derive builds the JSON Schema of the grant kinds `[authz.grants]` \
                  lists with serde_json::Map, which no item-level attribute reaches"
    )
)]

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::party_binding::policy::PolicyRole;

/// One kind of grant the CDR may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum GrantKind {
    /// A system client commits data other systems recorded (system client
    /// `ROLE`, `at10`).
    #[serde(rename = "system_client.relay")]
    SystemClientRelay,
    /// A system client commits data a device measured (system client `ROLE`,
    /// `at11`).
    #[serde(rename = "system_client.device")]
    SystemClientDevice,
    /// A data migration imports or exports EHRs (data migration `ROLE`,
    /// `at10`).
    #[serde(rename = "data_migration")]
    DataMigration,
    /// The agreement of a federation partner's persons (federation partner
    /// `ROLE`, `at10`).
    #[serde(rename = "federation_partner.person")]
    FederationPerson,
    /// The agreement of a federation partner organisation itself (federation
    /// partner `ROLE`, `at11`).
    #[serde(rename = "federation_partner.organisation")]
    FederationOrganisation,
    /// Reading the access log of one subject (privileged `ROLE`, `at10`).
    #[serde(rename = "privileged.subject_audit")]
    SubjectAudit,
    /// Acting as an EHDS Article 10 access service (privileged `ROLE`,
    /// `at11`).
    #[serde(rename = "privileged.access_service")]
    AccessService,
    /// Reading a party's protected national identifiers (privileged `ROLE`,
    /// `at12`).
    #[serde(rename = "privileged.identifier_reader")]
    IdentifierReader,
    /// Importing an EHR or an EHR extract (privileged `ROLE`, `at13`).
    #[serde(rename = "privileged.import")]
    Import,
}

impl GrantKind {
    /// Every kind, in contract order.
    pub const ALL: [Self; 9] = [
        Self::SystemClientRelay,
        Self::SystemClientDevice,
        Self::DataMigration,
        Self::FederationPerson,
        Self::FederationOrganisation,
        Self::SubjectAudit,
        Self::AccessService,
        Self::IdentifierReader,
        Self::Import,
    ];

    /// The kind's name, as `[authz.grants] allowed` and the side table spell
    /// it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SystemClientRelay => "system_client.relay",
            Self::SystemClientDevice => "system_client.device",
            Self::DataMigration => "data_migration",
            Self::FederationPerson => "federation_partner.person",
            Self::FederationOrganisation => "federation_partner.organisation",
            Self::SubjectAudit => "privileged.subject_audit",
            Self::AccessService => "privileged.access_service",
            Self::IdentifierReader => "privileged.identifier_reader",
            Self::Import => "privileged.import",
        }
    }

    /// The policy `ROLE` kind that carries this grant.
    #[must_use]
    pub const fn role(self) -> PolicyRole {
        match self {
            Self::SystemClientRelay | Self::SystemClientDevice => PolicyRole::SystemClient,
            Self::DataMigration => PolicyRole::DataMigration,
            Self::FederationPerson | Self::FederationOrganisation => PolicyRole::FederationPartner,
            Self::SubjectAudit | Self::AccessService | Self::IdentifierReader | Self::Import => {
                PolicyRole::Privileged
            }
        }
    }

    /// The `CAPABILITY` name code of this grant in its `ROLE` archetype.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::SystemClientRelay
            | Self::DataMigration
            | Self::FederationPerson
            | Self::SubjectAudit => "at10",
            Self::SystemClientDevice | Self::FederationOrganisation | Self::AccessService => "at11",
            Self::IdentifierReader => "at12",
            Self::Import => "at13",
        }
    }

    /// The kind a `CAPABILITY` of a `role` kind `ROLE` named by `code`
    /// carries, `None` for a code the contract does not define. A specialised
    /// code is of the kind of the code it specialises.
    #[must_use]
    pub fn of(role: PolicyRole, code: &str) -> Option<Self> {
        let code = code.trim();
        let parent = code.split_once('.').map_or(code, |(parent, _)| parent);
        Self::ALL
            .into_iter()
            .find(|kind| kind.role() == role && kind.code() == parent)
    }

    /// The kind its name `name` spells, `None` for another name.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}

impl fmt::Display for GrantKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The name a capability's kind is stored under: the kind's name, or for a
/// code the contract does not define, the `ROLE` kind and the code as
/// written, which never names a [`GrantKind`].
#[must_use]
pub(crate) fn stored_name(role: PolicyRole, code: Option<&str>) -> String {
    match code.and_then(|code| GrantKind::of(role, code)) {
        Some(kind) => kind.as_str().to_owned(),
        None => format!("{}:{}", role.as_str(), code.unwrap_or_default().trim()),
    }
}

/// The kinds of grant the CDR may hold: `[authz.grants] allowed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantCeiling {
    /// The kinds allowed.
    allowed: BTreeSet<GrantKind>,
}

impl GrantCeiling {
    /// The ceiling allowing `allowed`.
    #[must_use]
    pub fn new(allowed: impl IntoIterator<Item = GrantKind>) -> Self {
        Self {
            allowed: allowed.into_iter().collect(),
        }
    }

    /// Whether the ceiling allows `kind`.
    #[must_use]
    pub fn allows(&self, kind: GrantKind) -> bool {
        self.allowed.contains(&kind)
    }

    /// Whether the ceiling allows the kind stored as `name`: never for a name
    /// that is no [`GrantKind`].
    #[must_use]
    pub fn allows_name(&self, name: &str) -> bool {
        GrantKind::named(name).is_some_and(|kind| self.allows(kind))
    }

    /// The kinds allowed, in contract order.
    pub fn allowed(&self) -> impl Iterator<Item = GrantKind> + '_ {
        self.allowed.iter().copied()
    }
}

impl Default for GrantCeiling {
    /// The ceiling of the default `[authz.grants] allowed`.
    fn default() -> Self {
        Self::new(crate::config::authz::GrantsConfig::default().allowed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_reads_back_from_its_role_code_and_name() {
        for kind in GrantKind::ALL {
            assert_eq!(GrantKind::of(kind.role(), kind.code()), Some(kind));
            assert_eq!(GrantKind::named(kind.as_str()), Some(kind));
            assert_eq!(stored_name(kind.role(), Some(kind.code())), kind.as_str());
        }
    }

    #[test]
    fn a_specialised_code_is_of_its_parents_kind_and_an_unknown_one_of_none() {
        assert_eq!(
            GrantKind::of(PolicyRole::Privileged, "at12.1"),
            Some(GrantKind::IdentifierReader)
        );
        assert_eq!(GrantKind::of(PolicyRole::DataMigration, "at11"), None);
        assert_eq!(GrantKind::of(PolicyRole::SystemClient, "at99"), None);
        let unknown = stored_name(PolicyRole::SystemClient, Some("at99"));
        assert_eq!(unknown, "system_client:at99");
        assert!(!GrantCeiling::new(GrantKind::ALL).allows_name(&unknown));
    }

    #[test]
    fn the_ceiling_allows_only_what_it_names() {
        let ceiling = GrantCeiling::new([GrantKind::SystemClientRelay]);
        assert!(ceiling.allows(GrantKind::SystemClientRelay));
        assert!(!ceiling.allows(GrantKind::IdentifierReader));
        assert!(ceiling.allows_name("system_client.relay"));
        assert!(!ceiling.allows_name("privileged.identifier_reader"));
    }
}
