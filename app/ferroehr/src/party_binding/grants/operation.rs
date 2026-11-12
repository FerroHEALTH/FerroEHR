// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! FerroEHR's operation terminology, the codes a capability grants by, and
//! the ceiling `[authz.grants] allowed` puts on them.
//!
//! A `CAPABILITY` grants an operation when its `credentials` hold, anywhere
//! in their tree, a `DV_CODED_TEXT` whose `defining_code` names
//! [`TERMINOLOGY`] and an operation code: the RM's `ITEM_STRUCTURE` with a
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

/// The `TERMINOLOGY_ID` of FerroEHR's operation terminology.
pub const TERMINOLOGY: &str = "ferroehr-operations";

/// The `DV_IDENTIFIER.type` that binds an actor to an OAuth 2.0 client: its
/// `issuer` is the token issuer and its `id` the `client_id`.
pub const CLIENT_ID: &str = "client_id";

/// The code of a `PARTY_RELATIONSHIP` from a client to the party it acts
/// for, in its `name` or its details.
pub const ACTS_FOR: &str = "acts_for";

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

/// The codes of the terminology that name no operation.
const QUALIFIERS: [&str; 6] = [
    ACTS_FOR, NOT_BEFORE, NOT_AFTER, SCOPES[0], SCOPES[1], SCOPES[2],
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
}

impl Operation {
    /// Every operation, in contract order.
    pub const ALL: [Self; 9] = [
        Self::CommitComposition,
        Self::ImportEhr,
        Self::ImportEhrExtract,
        Self::ExportEhrExtract,
        Self::FederatedPersonAccess,
        Self::FederatedOrganisationAccess,
        Self::ReadSubjectAudit,
        Self::AccessService,
        Self::ReadIdentifiers,
    ];

    /// The operation's code in [`TERMINOLOGY`].
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

/// Whether `code` of [`TERMINOLOGY`] is a qualifier rather than an
/// operation: the acts-for code, a bound or a scope.
#[must_use]
pub(crate) fn is_qualifier(code: &str) -> bool {
    QUALIFIERS.contains(&code.trim())
}

/// The operations the CDR may grant: `[authz.grants] allowed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantCeiling {
    /// The operations allowed.
    allowed: BTreeSet<Operation>,
}

impl GrantCeiling {
    /// The ceiling allowing `allowed`.
    #[must_use]
    pub fn new(allowed: impl IntoIterator<Item = Operation>) -> Self {
        Self {
            allowed: allowed.into_iter().collect(),
        }
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
