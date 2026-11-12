// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The default policy definitions: example archetypes of a system `AGENT`,
//! the `ROLE`s whose `CAPABILITY.credentials` grant operations, and the
//! acts-for `PARTY_RELATIONSHIP` from a system to the provider it acts for.
//!
//! FerroEHR does not recognise a policy party by these archetypes: a party is
//! one when a version of it holds the coded content
//! [`crate::party_binding::policy_mark`] reads, in any archetype or template
//! a deployment models. The archetypes here are examples that use those
//! codes, and [`PolicyKind`] names them so a party of one is validated
//! against its own template.
//!
//! The binary carries a default definition of each ([`DEFINITIONS`]), which
//! the first boot stores when no definition of its id is stored; after that
//! they are ordinary definitions. Reading a capability's credentials as a
//! grant, and writing or reading a policy party only as an administrator, is
//! our own design: no openEHR spec gives `CAPABILITY` an authorization
//! meaning, and `ACCESS_CONTROL_SETTINGS` is implementation dependent.

use crate::storage::structure::archetype_parts;

/// The archetype of a system's `AGENT`.
pub const AGENT_ARCHETYPE: &str = "openEHR-DEMOGRAPHIC-AGENT.system.v1";

/// The path of the `ELEMENT` holding a system `AGENT`'s client identifier.
pub const AGENT_CLIENT_PATH: &str = "/details[id2]/items[id3]";

/// The `DV_IDENTIFIER.type` of a system `AGENT`'s OAuth 2.0 `client_id`.
pub const AGENT_CLIENT_TYPE: &str = "client_id";

/// The acts-for `PARTY_RELATIONSHIP` archetype.
pub const ACTS_FOR_ARCHETYPE: &str = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.acts_for.v1";

/// The default policy definitions, each its file name and its ADL 2 text,
/// the `AGENT` and the acts-for relationship before the `ROLE`s.
pub const DEFINITIONS: [(&str, &str); 6] = [
    (
        "openEHR-DEMOGRAPHIC-AGENT.system.v1.0.0.adls",
        include_str!(
            "../../assets/definitions/policy/openEHR-DEMOGRAPHIC-AGENT.system.v1.0.0.adls"
        ),
    ),
    (
        "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.acts_for.v1.0.0.adls",
        include_str!(
            "../../assets/definitions/policy/openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.acts_for.v1.0.0.adls"
        ),
    ),
    (
        "openEHR-DEMOGRAPHIC-ROLE.system_client.v1.0.0.adls",
        include_str!(
            "../../assets/definitions/policy/openEHR-DEMOGRAPHIC-ROLE.system_client.v1.0.0.adls"
        ),
    ),
    (
        "openEHR-DEMOGRAPHIC-ROLE.data_migration.v1.0.0.adls",
        include_str!(
            "../../assets/definitions/policy/openEHR-DEMOGRAPHIC-ROLE.data_migration.v1.0.0.adls"
        ),
    ),
    (
        "openEHR-DEMOGRAPHIC-ROLE.federation_partner.v1.0.0.adls",
        include_str!(
            "../../assets/definitions/policy/openEHR-DEMOGRAPHIC-ROLE.federation_partner.v1.0.0.adls"
        ),
    ),
    (
        "openEHR-DEMOGRAPHIC-ROLE.privileged.v1.0.0.adls",
        include_str!(
            "../../assets/definitions/policy/openEHR-DEMOGRAPHIC-ROLE.privileged.v1.0.0.adls"
        ),
    ),
];

/// The kind of grant a policy `ROLE` archetype carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PolicyRole {
    /// What a system client commits, at which facilities, from which origin.
    SystemClient,
    /// A data migration: import and export, its subjects, rate and window.
    DataMigration,
    /// The terms of a federation partner's data-sharing agreement.
    FederationPartner,
    /// A privilege a professional's `PERSON` holds beyond clinical access.
    Privileged,
}

impl PolicyRole {
    /// Every kind.
    pub const ALL: [Self; 4] = [
        Self::SystemClient,
        Self::DataMigration,
        Self::FederationPartner,
        Self::Privileged,
    ];

    /// The kind's name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SystemClient => "system_client",
            Self::DataMigration => "data_migration",
            Self::FederationPartner => "federation_partner",
            Self::Privileged => "privileged",
        }
    }

    /// The kind's `ROLE` archetype.
    #[must_use]
    pub const fn archetype_id(self) -> &'static str {
        match self {
            Self::SystemClient => "openEHR-DEMOGRAPHIC-ROLE.system_client.v1",
            Self::DataMigration => "openEHR-DEMOGRAPHIC-ROLE.data_migration.v1",
            Self::FederationPartner => "openEHR-DEMOGRAPHIC-ROLE.federation_partner.v1",
            Self::Privileged => "openEHR-DEMOGRAPHIC-ROLE.privileged.v1",
        }
    }
}

/// What a policy archetype makes a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyKind {
    /// A system's `AGENT`.
    Agent,
    /// A policy `ROLE` of its kind of grant.
    Role(PolicyRole),
    /// An acts-for `PARTY_RELATIONSHIP`.
    ActsFor,
}

impl PolicyKind {
    /// Every kind, with its archetype.
    #[must_use]
    pub fn all() -> [(Self, &'static str); 6] {
        [
            (Self::Agent, AGENT_ARCHETYPE),
            (Self::ActsFor, ACTS_FOR_ARCHETYPE),
            (
                Self::Role(PolicyRole::SystemClient),
                PolicyRole::SystemClient.archetype_id(),
            ),
            (
                Self::Role(PolicyRole::DataMigration),
                PolicyRole::DataMigration.archetype_id(),
            ),
            (
                Self::Role(PolicyRole::FederationPartner),
                PolicyRole::FederationPartner.archetype_id(),
            ),
            (
                Self::Role(PolicyRole::Privileged),
                PolicyRole::Privileged.archetype_id(),
            ),
        ]
    }

    /// The kind of the archetype `archetype_id`, in any case and at any
    /// minor or patch version: a policy archetype or a specialisation of one.
    #[must_use]
    pub fn of(archetype_id: &str) -> Option<Self> {
        let (entity, concept, major) = archetype_parts(archetype_id.trim())?;
        Self::parts()
            .into_iter()
            .find_map(|(kind, p_entity, p_concept, p_major)| {
                let specialises = concept == p_concept
                    || concept
                        .strip_prefix(p_concept.as_str())
                        .is_some_and(|rest| rest.starts_with('-'));
                (entity == p_entity && major == p_major && specialises).then_some(kind)
            })
    }

    /// Every kind with the parts of its archetype that [`Self::of`] compares:
    /// the lowercased `qualified_rm_entity` and `domain_concept`, and the
    /// major version, as [`archetype_parts`] decomposes them.
    #[must_use]
    pub fn parts() -> Vec<(Self, String, String, i32)> {
        Self::all()
            .into_iter()
            .filter_map(|(kind, policy)| {
                let (entity, concept, major) = archetype_parts(policy)?;
                Some((kind, entity, concept, major))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_policy_archetype_and_its_specialisations_are_recognised() {
        assert_eq!(
            PolicyKind::of("openEHR-DEMOGRAPHIC-ROLE.system_client.v1"),
            Some(PolicyKind::Role(PolicyRole::SystemClient))
        );
        assert_eq!(
            PolicyKind::of("openehr-demographic-role.system_client.v1.4.2"),
            Some(PolicyKind::Role(PolicyRole::SystemClient))
        );
        assert_eq!(
            PolicyKind::of("openEHR-DEMOGRAPHIC-ROLE.system_client-hospital.v1"),
            Some(PolicyKind::Role(PolicyRole::SystemClient))
        );
        assert_eq!(
            PolicyKind::of("openEHR-DEMOGRAPHIC-AGENT.system.v1.0.0"),
            Some(PolicyKind::Agent)
        );
        for other in [
            "openEHR-DEMOGRAPHIC-ROLE.system_client.v2",
            "openEHR-DEMOGRAPHIC-ROLE.system_clientele.v1",
            "openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1",
            "openEHR-DEMOGRAPHIC-PERSON.system_client.v1",
            "id6",
        ] {
            assert_eq!(PolicyKind::of(other), None, "{other}");
        }
    }
}
