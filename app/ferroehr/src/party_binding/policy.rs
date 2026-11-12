// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The policy parties: the system `AGENT`, the policy `ROLE`s whose
//! `CAPABILITY.credentials` hold a grant, and the acts-for
//! `PARTY_RELATIONSHIP` from a system to the provider it acts for.
//!
//! The RM demographic model gives the classes (`agent.adoc`: "any kind of
//! agent, including devices, software systems"; `role.adoc`: "Roles should
//! have credentials qualifying the performer to perform the role";
//! `capability.adoc`; `party_relationship.adoc`). Reading a capability's
//! credentials as a grant, and writing a policy party only as an
//! administrator, is our own design: no openEHR spec gives `CAPABILITY` an
//! authorization meaning, and `ACCESS_CONTROL_SETTINGS` is implementation
//! dependent.

use super::IdentifierSelector;

/// The kind of grant a policy `ROLE` template carries.
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
    /// Every kind, in the order the configuration lists them.
    pub const ALL: [Self; 4] = [
        Self::SystemClient,
        Self::DataMigration,
        Self::FederationPartner,
        Self::Privileged,
    ];

    /// The kind's name, as the mapping digest and the refusals write it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SystemClient => "system_client",
            Self::DataMigration => "data_migration",
            Self::FederationPartner => "federation_partner",
            Self::Privileged => "privileged",
        }
    }
}

/// The system `AGENT` template and the selector of the client identifier its
/// `PARTY.details` carry (RM demographic §Party Identification).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentSelectors {
    /// The canonical template id.
    pub(crate) template: String,
    /// The `DV_IDENTIFIER` whose issuer is the token issuer and whose id is
    /// the OAuth 2.0 `client_id`.
    pub(crate) client: IdentifierSelector,
}
