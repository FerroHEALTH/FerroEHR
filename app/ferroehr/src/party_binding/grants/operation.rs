// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! FerroEHR's operation terminology, the codes a capability grants by, and
//! the ceiling `[authz.grants] allowed` puts on them.
//!
//! A `CAPABILITY` grants an operation when its `credentials` hold, anywhere
//! in their tree, a `DV_CODED_TEXT` whose `defining_code` names
//! [`crate::party_binding::policy_mark::TERMINOLOGY`] and an operation code: the RM's `ITEM_STRUCTURE` with a
//! terminology binding, which a deployment models in any archetype or
//! template. The same terminology names the acts-for relationship and the
//! qualifiers a grant carries. No openEHR spec gives a capability an
//! authorization meaning: our own design.

#![cfg_attr(
    feature = "schema",
    expect(
        clippy::disallowed_types,
        reason = "owner-approved 2026-08-03 (#1694 family 8, OpenAPI schema literals): the \
                  schemars derive builds the JSON Schema of the operations `[authz.grants]` \
                  lists with serde_json::Map, which no item-level attribute reaches"
    )
)]

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::party_binding::grants::admission::{
    ADMIT_ASSURANCE, ADMIT_GROUP, ADMIT_GUESTS, ADMIT_IDP, ADMIT_ROLE, LOGIN_ISSUER,
};
use crate::party_binding::policy_mark::ACTS_FOR;

/// The code naming the credentials element whose `DV_DATE_TIME` is the
/// instant before which a grant does not hold.
pub const NOT_BEFORE: &str = "not_before";

/// The code naming the credentials element whose `DV_DATE_TIME` is the
/// instant after which a grant does not hold.
pub const NOT_AFTER: &str = "not_after";

/// The codes naming a grant's scope elements, which a check reads with
/// [`crate::party_binding::grants::capabilities::Credentials::named`]: a
/// template written, a facility, a subject namespace.
pub const SCOPES: [&str; 3] = ["template", "facility", "subject_namespace"];

/// The code naming the credentials element whose text is a role a
/// [`Operation::HoldRoles`] grant gives a Basic user.
pub const ROLE: &str = "role";

/// The code naming the credentials element whose text is a SMART scope a
/// [`Operation::HoldScopes`] grant gives a Basic user.
pub const SCOPE: &str = "scope";

/// The codes of the terminology that name no operation.
const QUALIFIERS: [&str; 14] = [
    ACTS_FOR,
    NOT_BEFORE,
    NOT_AFTER,
    SCOPES[0],
    SCOPES[1],
    SCOPES[2],
    ROLE,
    SCOPE,
    LOGIN_ISSUER,
    ADMIT_IDP,
    ADMIT_GROUP,
    ADMIT_ROLE,
    ADMIT_ASSURANCE,
    ADMIT_GUESTS,
];

/// One operation a capability may grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    /// Commit a `COMPOSITION` on the caller's own account, within the
    /// templates and facilities the grant names.
    CommitComposition,
    /// Import an EHR (`message_import_ehr`).
    ImportEhr,
    /// Import an EHR extract (`message_import_ehr_extract`).
    ImportEhrExtract,
    /// Export an EHR extract.
    ExportEhrExtract,
    /// Admit a federation partner's persons under a data-sharing agreement.
    FederatedPersonAccess,
    /// Admit a federation partner organisation itself under a data-sharing
    /// agreement.
    FederatedOrganisationAccess,
    /// Read the access log of one subject.
    ReadSubjectAudit,
    /// Act as an EHDS Article 10 access service, held to a subject's
    /// opt-out.
    AccessService,
    /// Read a party's protected national identifiers.
    ReadIdentifiers,
    /// Hold, as a Basic user, the roles the grant's [`ROLE`] elements name.
    HoldRoles,
    /// Hold, as a Basic user, the SMART scopes the grant's [`SCOPE`] elements
    /// name.
    HoldScopes,
    /// Create a provisional `PERSON` at the first login of a person no party
    /// carries, under the admission conditions the grant's credentials name
    /// ([`crate::party_binding::grants::admission`]).
    CreateAtLogin,
}

impl Operation {
    /// Every operation, in contract order.
    pub const ALL: [Self; 12] = [
        Self::CommitComposition,
        Self::ImportEhr,
        Self::ImportEhrExtract,
        Self::ExportEhrExtract,
        Self::FederatedPersonAccess,
        Self::FederatedOrganisationAccess,
        Self::ReadSubjectAudit,
        Self::AccessService,
        Self::ReadIdentifiers,
        Self::HoldRoles,
        Self::HoldScopes,
        Self::CreateAtLogin,
    ];

    /// The operation's code in [`crate::party_binding::policy_mark::TERMINOLOGY`].
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::CommitComposition => "commit_composition",
            Self::ImportEhr => "import_ehr",
            Self::ImportEhrExtract => "import_ehr_extract",
            Self::ExportEhrExtract => "export_ehr_extract",
            Self::FederatedPersonAccess => "federated_person_access",
            Self::FederatedOrganisationAccess => "federated_organisation_access",
            Self::ReadSubjectAudit => "read_subject_audit",
            Self::AccessService => "access_service",
            Self::ReadIdentifiers => "read_identifiers",
            Self::HoldRoles => "hold_roles",
            Self::HoldScopes => "hold_scopes",
            Self::CreateAtLogin => "create_at_login",
        }
    }

    /// The operation `code` names, `None` for another code.
    #[must_use]
    pub fn of(code: &str) -> Option<Self> {
        let code = code.trim();
        Self::ALL
            .into_iter()
            .find(|operation| operation.code() == code)
    }
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// Whether `code` of [`crate::party_binding::policy_mark::TERMINOLOGY`] is a qualifier rather than an
/// operation: the acts-for code, a bound or a scope.
#[must_use]
pub(crate) fn is_qualifier(code: &str) -> bool {
    QUALIFIERS.contains(&code.trim())
}

/// The operations the CDR may grant (`[authz.grants] allowed`), and the
/// roles a [`Operation::HoldRoles`] grant may never name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantCeiling {
    /// The operations allowed.
    allowed: BTreeSet<Operation>,
    /// The privileged roles of `[authz.rbac]`, upper-cased: each comes from
    /// the configuration, a `role_issuers` token or an operation of its own.
    reserved: BTreeSet<String>,
}

impl GrantCeiling {
    /// The ceiling allowing `allowed`, reserving no role.
    #[must_use]
    pub fn new(allowed: impl IntoIterator<Item = Operation>) -> Self {
        Self {
            allowed: allowed.into_iter().collect(),
            reserved: BTreeSet::new(),
        }
    }

    /// This ceiling with `roles` reserved: a [`Operation::HoldRoles`] grant
    /// naming one is refused at commit and never honoured.
    #[must_use]
    pub fn with_reserved_roles(mut self, roles: impl IntoIterator<Item = String>) -> Self {
        self.reserved = roles
            .into_iter()
            .map(|role| role.trim().to_ascii_uppercase())
            .collect();
        self
    }

    /// Whether `role` is reserved, compared without regard to ASCII case.
    #[must_use]
    pub fn reserves(&self, role: &str) -> bool {
        self.reserved.contains(&role.trim().to_ascii_uppercase())
    }

    /// Whether the ceiling allows `operation`.
    #[must_use]
    pub fn allows(&self, operation: Operation) -> bool {
        self.allowed.contains(&operation)
    }

    /// Whether the ceiling allows the operation stored as `code`: never for a
    /// code that names no [`Operation`].
    #[must_use]
    pub fn allows_code(&self, code: &str) -> bool {
        Operation::of(code).is_some_and(|operation| self.allows(operation))
    }

    /// The operations allowed, in contract order.
    pub fn allowed(&self) -> impl Iterator<Item = Operation> + '_ {
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
    fn every_operation_reads_back_from_its_code() {
        for operation in Operation::ALL {
            assert_eq!(Operation::of(operation.code()), Some(operation));
            assert!(!is_qualifier(operation.code()));
        }
        assert_eq!(Operation::of("import_everything"), None);
        assert!(is_qualifier(ACTS_FOR) && is_qualifier("template"));
        assert!(is_qualifier(LOGIN_ISSUER) && is_qualifier(ADMIT_GUESTS));
    }

    #[test]
    fn a_reserved_role_is_reserved_in_any_case() {
        let ceiling = GrantCeiling::new([Operation::HoldRoles])
            .with_reserved_roles(["Admin".to_owned(), " importer ".to_owned()]);
        assert!(ceiling.reserves("ADMIN") && ceiling.reserves("admin"));
        assert!(ceiling.reserves("IMPORTER"));
        assert!(!ceiling.reserves("CLINICIAN"));
        assert!(!GrantCeiling::new([]).reserves("ADMIN"));
    }

    #[test]
    fn the_ceiling_allows_only_what_it_names() {
        let ceiling = GrantCeiling::new([Operation::CommitComposition]);
        assert!(ceiling.allows(Operation::CommitComposition));
        assert!(!ceiling.allows(Operation::ReadIdentifiers));
        assert!(ceiling.allows_code("commit_composition"));
        assert!(!ceiling.allows_code("read_identifiers"));
        assert!(!GrantCeiling::new(Operation::ALL).allows_code("import_everything"));
    }
}
