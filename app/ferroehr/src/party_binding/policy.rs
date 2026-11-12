// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The policy parties: the system `AGENT`, the policy `ROLE`s whose
//! `CAPABILITY.credentials` hold a grant, and the acts-for
//! `PARTY_RELATIONSHIP` from a system to the provider it acts for.
//!
//! FerroEHR recognises a policy party by its root archetype, never by its
//! template: the policy archetype itself, or an ADL 2 specialisation of it
//! (AM `ADL2/master09.01-spec_overview.adoc` §Specialisation: "All possible
//! data instance arrangements that conform to the specialised archetype must
//! also conform to all of its parents"; BASE `base_types` §Archetype
//! Identifiers: `domain_concept = concept_name { '-' specialisation }`). A
//! deployment versions, renames or constrains the templates built on these
//! archetypes, or builds its own, and they stay policy parties. The archetype
//! ids and the node ids FerroEHR reads are the contract; a breaking change to
//! them is a new archetype major version.
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
    ///
    /// The node codec stores the same decomposition of each node's archetype
    /// in its `arch_entity`, `arch_concept` and `arch_major` columns, so a
    /// query tests a stored root against these parts.
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

    /// The POSIX regular expression an archetype id matches exactly when
    /// [`Self::of`] answers this kind for it.
    ///
    /// It spells the lexical form of BASE `base_types` master05 §Archetype
    /// Identifiers (`[namespace::]` entity `.` concept `{'-' specialisation}`
    /// `.v` version, the minor, patch and status optional) around this kind's
    /// parts, their letters in any case, for an id a query reads from stored
    /// JSON, where no decomposed column holds it.
    #[must_use]
    pub fn id_pattern(self) -> Option<String> {
        let (_, entity, concept, major) =
            Self::parts().into_iter().find(|(kind, ..)| *kind == self)?;
        let any_case = |part: &str| -> String {
            part.chars()
                .map(|c| {
                    if c.is_ascii_alphabetic() {
                        format!("[{}{}]", c.to_ascii_lowercase(), c.to_ascii_uppercase())
                    } else {
                        c.to_string()
                    }
                })
                .collect()
        };
        let (entity, concept) = (any_case(&entity), any_case(&concept));
        let segment = "[a-zA-Z][a-zA-Z0-9_]*";
        let number = "(0|[1-9][0-9]*)";
        Some(format!(
            "^[[:space:]]*([a-zA-Z][a-zA-Z0-9_-]*(\\.[a-zA-Z][a-zA-Z0-9_-]*)*::)?{entity}\\.\
             {concept}(-{segment})*\\.v{major}\
             (\\.{number}(\\.{number}(-(rc|alpha|beta)(\\.[0-9]+)?)?)?)?[[:space:]]*$"
        ))
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

    /// The pattern a query reads a stored id with answers what
    /// [`PolicyKind::of`] answers, for every kind and every id below.
    #[test]
    fn the_id_pattern_agrees_with_the_archetype_test() {
        let ids = [
            "openEHR-DEMOGRAPHIC-ROLE.system_client.v1",
            "openehr-demographic-role.system_client.v1.4.2",
            "OPENEHR-DEMOGRAPHIC-ROLE.SYSTEM_CLIENT.V1",
            " openEHR-DEMOGRAPHIC-ROLE.system_client.v1\n",
            "openEHR-DEMOGRAPHIC-ROLE.system_client-hospital.v1",
            "openEHR-DEMOGRAPHIC-ROLE.system_client-hospital-ward.v1.0.0",
            "openEHR-DEMOGRAPHIC-ROLE.system_client.v1.2",
            "openEHR-DEMOGRAPHIC-ROLE.system_client.v1.0.0-rc.1",
            "openEHR-DEMOGRAPHIC-ROLE.system_client.v1.0.0-beta",
            "org.example::openEHR-DEMOGRAPHIC-ROLE.system_client.v1.0.0",
            "openEHR-DEMOGRAPHIC-AGENT.system.v1.0.0",
            "openEHR-DEMOGRAPHIC-AGENT.system-gateway.v1",
            "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.acts_for.v1",
            "openehr-demographic-party_relationship.acts_for.v1.0.0",
            "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.acts_for-regional.v1.1.0",
            "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.acts_for.v2",
            "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.acts_formerly.v1",
            "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1",
            "openEHR-DEMOGRAPHIC-ROLE.system_client.v2",
            "openEHR-DEMOGRAPHIC-ROLE.system_client.v10",
            "openEHR-DEMOGRAPHIC-ROLE.system_clientele.v1",
            "openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1",
            "openEHR-DEMOGRAPHIC-PERSON.system_client.v1",
            "openEHR-DEMOGRAPHIC-ROLE.data_migration.v1",
            "openEHR-DEMOGRAPHIC-ROLE.federation_partner-eu.v1",
            "openEHR-DEMOGRAPHIC-ROLE.privileged.v1.0.0",
            "openEHR-DEMOGRAPHIC-ROLE.system_client.v1.0.0-gamma",
            "openEHR-DEMOGRAPHIC-ROLE.system_client.v01",
            "openEHR-DEMOGRAPHIC-ROLE.system_client.v1.",
            "x-openEHR-DEMOGRAPHIC-ROLE.system_client.v1",
            "id6",
            "",
        ];
        for (kind, _) in PolicyKind::all() {
            let pattern = kind.id_pattern().expect("a policy archetype parses");
            let matcher = regex::Regex::new(&pattern).expect("the pattern compiles");
            for id in ids {
                assert_eq!(
                    matcher.is_match(id),
                    PolicyKind::of(id) == Some(kind),
                    "{kind:?} on {id:?}"
                );
            }
        }
    }
}
