// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The classification of every operation the REST layer dispatches: one row
//! per operation id, read by the RBAC route map, the ABAC policy-enforcement
//! point, the SMART on openEHR gates, the `EHR_ACCESS` privacy ceiling and the
//! ATNA audit trail.
//!
//! Every generated ITS-REST operation has a row, which the completeness tests
//! of [`crate::extensions::access::authz::classify`] and
//! [`crate::system_log::classify`] hold. An extension operation has a row
//! where it is ABAC-checked, SMART-checked or audited under a class of its own;
//! an operation without a row is RBAC-classified by its path, skipped by ABAC,
//! treated as a write, and audited under the fail-closed default. No
//! specification governs this: our own design.

use std::collections::HashMap;
use std::sync::LazyLock;

use ferroehr::system_log::event::{EventActionCode, ObjectClass};
use ferrohealth_smart::openehr::smart_scopes::Permission;

use crate::extensions::access::authz::classify::{Mode, OperationClass};
use crate::extensions::access::authz::request::{AccessMode, ResourceKind};
use crate::system_log::classify::Classification;

/// What an operation does to the resource it addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verb {
    /// Creates a resource or commits its first version.
    Create,
    /// Reads a resource.
    Read,
    /// Updates a resource.
    Update,
    /// Deletes a resource.
    Delete,
    /// Executes an AQL query, a read despite its `POST` wire verb.
    Search,
    /// Acts on the node itself, outside the four resource verbs.
    Execute,
}

/// The SMART on openEHR `composition-` gate the policy-enforcement point holds
/// an operation to, once the template is known (master08 §Resource Scopes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompositionGate {
    /// No `composition-` gate in the policy-enforcement point.
    None,
    /// The template of the version the dispatch returned, after the backend
    /// call.
    Read,
    /// The template the body names or the deleted version carries, before the
    /// backend call, and the returned version's after it.
    Write,
}

/// One operation's classification.
#[derive(Debug)]
pub(crate) struct Operation {
    /// The operation id.
    pub(crate) id: &'static str,
    /// What the operation does to its resource.
    pub(crate) verb: Verb,
    /// The RBAC class of a generated operation; `None` for an extension
    /// operation, which RBAC classifies by its path.
    pub(crate) class: Option<OperationClass>,
    /// The ABAC resource kind; `None` for an operation ABAC does not model.
    pub(crate) kind: Option<ResourceKind>,
    /// When the ABAC policy-enforcement point checks the operation.
    pub(crate) pep: Mode,
    /// The DICOM resource class the operation is audited under; `None` for the
    /// fail-closed default.
    pub(crate) audit: Option<ObjectClass>,
    /// The SMART `composition-` gate of the policy-enforcement point.
    pub(crate) composition: CompositionGate,
}

impl Operation {
    /// Whether the operation changes stored data, for the read-only gate.
    ///
    /// An `Execute` has no read verb, so the gate treats it as a write.
    #[must_use]
    pub(crate) fn is_write(&self) -> bool {
        !matches!(self.verb, Verb::Read | Verb::Search)
    }

    /// The ABAC access mode (the Cedar action axis) of an operation with a
    /// resource kind.
    #[must_use]
    pub(crate) fn access(&self) -> Option<AccessMode> {
        self.kind.map(|_kind| match self.verb {
            Verb::Create => AccessMode::Create,
            Verb::Update => AccessMode::Update,
            Verb::Delete => AccessMode::Delete,
            Verb::Search => AccessMode::Execute,
            Verb::Read | Verb::Execute => AccessMode::Read,
        })
    }

    /// The SMART permission letter the operation exercises (master08
    /// §Resource Scopes: `c` create, `r` read, `u` update, `d` delete, `s`
    /// search or execute).
    #[must_use]
    pub(crate) fn permission(&self) -> Permission {
        match self.verb {
            Verb::Create => Permission::Create,
            Verb::Update => Permission::Update,
            Verb::Delete => Permission::Delete,
            Verb::Search => Permission::Search,
            Verb::Read | Verb::Execute => Permission::Read,
        }
    }

    /// The explicit audit classification: the DICOM `EventActionCode` of the
    /// verb (PS3.15 §A.5.1) on the operation's resource class.
    #[must_use]
    pub(crate) fn audit(&self) -> Option<Classification> {
        let action = match self.verb {
            Verb::Create => EventActionCode::Create,
            Verb::Read => EventActionCode::Read,
            Verb::Update => EventActionCode::Update,
            Verb::Delete => EventActionCode::Delete,
            Verb::Search | Verb::Execute => EventActionCode::Execute,
        };
        self.audit
            .map(|object| Classification::Audited { action, object })
    }

    /// Whether the operation commits a CONTRIBUTION.
    #[must_use]
    pub(crate) fn commits_contribution(&self) -> bool {
        self.kind == Some(ResourceKind::Contribution) && self.verb == Verb::Create
    }

    /// Whether the operation reads a COMPOSITION, the set the `EHR_ACCESS`
    /// privacy ceiling and the ABAC post-check share.
    #[must_use]
    pub(crate) fn reads_composition(&self) -> bool {
        self.pep == Mode::Post && self.kind == Some(ResourceKind::Composition)
    }
}

/// The operation `id`, or `None` for an operation without a row.
#[must_use]
pub(crate) fn operation(id: &str) -> Option<&'static Operation> {
    BY_ID.get(id).copied()
}

/// [`OPERATIONS`] by id.
static BY_ID: LazyLock<HashMap<&'static str, &'static Operation>> =
    LazyLock::new(|| OPERATIONS.iter().map(|op| (op.id, op)).collect());

/// Builds [`OPERATIONS`], one row per line: id, verb, RBAC class, ABAC kind,
/// ABAC mode, audit class and `composition-` gate, `None` for an absent value.
macro_rules! operations {
    ($($id:expr => $verb:ident, $class:ident, $kind:ident, $pep:ident, $audit:ident, $gate:ident;)*) => {
        &[$(Operation {
            id: $id,
            verb: Verb::$verb,
            class: operations!(@opt OperationClass $class),
            kind: operations!(@opt ResourceKind $kind),
            pep: Mode::$pep,
            audit: operations!(@opt ObjectClass $audit),
            composition: CompositionGate::$gate,
        },)*]
    };
    (@opt $type:ident None) => { None };
    (@opt $type:ident $value:ident) => { Some($type::$value) };
}

/// Every classified operation.
///
/// The CONTRIBUTION ids are shared by the EHR and DEMOGRAPHIC groups, and one
/// row is right for both: RM common master06 change control governs every
/// `VERSIONED_OBJECT`. Auditing an `ITEM_TAG` operation under its parent's
/// class is our own design, since RM ehr `master04-ehr_package.adoc` §Tags puts
/// tags outside change control. `options` is RBAC-classified for totality only:
/// the router mounts the manifest above the authentication layer.
static OPERATIONS: &[Operation] = operations! {
    "admin_ehr_delete" => Delete, Admin, None, Skip, Ehr, None;
    "admin_ehr_delete_all" => Delete, Admin, None, Skip, Ehr, None;
    "admin_party_delete" => Delete, None, None, Skip, Demographic, None;
    "admin_party_identifiers" => Read, None, None, Skip, Demographic, None;
    "agent_create" => Create, Clinical, None, Skip, Demographic, None;
    "agent_delete" => Delete, Clinical, None, Skip, Demographic, None;
    "agent_get" => Read, Clinical, None, Skip, Demographic, None;
    "agent_tags_delete" => Delete, Clinical, None, Skip, Demographic, None;
    "agent_tags_get" => Read, Clinical, None, Skip, Demographic, None;
    "agent_tags_update" => Update, Clinical, None, Skip, Demographic, None;
    "agent_update" => Update, Clinical, None, Skip, Demographic, None;
    crate::api::ehr::access_delay::RELEASE => Update, None, Composition, Pre, None, Read;
    "composition_create" => Create, Clinical, Composition, Pre, Composition, Write;
    "composition_delete" => Delete, Clinical, Composition, Pre, Composition, Write;
    "composition_get" => Read, Clinical, Composition, Post, Composition, Read;
    "composition_tags_delete" => Delete, Clinical, Composition, Skip, Composition, None;
    "composition_tags_get" => Read, Clinical, Composition, Skip, Composition, None;
    "composition_tags_update" => Update, Clinical, Composition, Skip, Composition, None;
    "composition_update" => Update, Clinical, Composition, Pre, Composition, Write;
    "contribution_create" => Create, Clinical, Contribution, Pre, Contribution, None;
    "contribution_get" => Read, Clinical, Contribution, Post, Contribution, None;
    "contribution_list" => Read, None, Contribution, Pre, None, None;
    "definition_query_list" => Read, Clinical, None, Skip, Query, None;
    "definition_query_store.yaml" => Create, Clinical, None, Skip, Query, None;
    "definition_query_version_get" => Read, Clinical, None, Skip, Query, None;
    "definition_query_version_store.yaml" => Create, Clinical, None, Skip, Query, None;
    "definition_template_adl1.4_example_get" => Read, Clinical, None, Skip, Template, None;
    "definition_template_adl1.4_get" => Read, Clinical, None, Skip, Template, None;
    "definition_template_adl1.4_list" => Read, Clinical, None, Skip, Template, None;
    "definition_template_adl1.4_upload" => Create, Clinical, None, Skip, Template, None;
    "definition_template_adl14_convert" => Read, None, None, Skip, Template, None;
    "definition_template_adl2_example_get" => Read, Clinical, None, Skip, Template, None;
    "definition_template_adl2_get" => Read, Clinical, None, Skip, Template, None;
    "definition_template_adl2_list" => Read, Clinical, None, Skip, Template, None;
    "definition_template_adl2_upload" => Create, Clinical, None, Skip, Template, None;
    "definition_template_adl2_version_get" => Read, Clinical, None, Skip, Template, None;
    "demographic_tags_get" => Read, Clinical, None, Skip, Demographic, None;
    "directory_create" => Create, Clinical, Directory, Pre, Directory, None;
    "directory_delete" => Delete, Clinical, Directory, Pre, Directory, None;
    "directory_get_at_time" => Read, Clinical, Directory, Pre, Directory, None;
    "directory_get_by_version_id" => Read, Clinical, Directory, Pre, Directory, None;
    "directory_update" => Update, Clinical, Directory, Pre, Directory, None;
    "ehr_create" => Create, Clinical, Ehr, Pre, Ehr, None;
    "ehr_create_with_id" => Create, Clinical, Ehr, Pre, Ehr, None;
    "ehr_get_by_id" => Read, Clinical, Ehr, Pre, Ehr, None;
    "ehr_get_by_subject" => Read, Clinical, Ehr, Pre, Ehr, None;
    "ehr_status_get_at_time" => Read, Clinical, EhrStatus, Pre, Ehr, None;
    "ehr_status_get_by_version_id" => Read, Clinical, EhrStatus, Pre, Ehr, None;
    "ehr_status_tags_delete" => Delete, Clinical, EhrStatus, Pre, Ehr, None;
    "ehr_status_tags_get" => Read, Clinical, EhrStatus, Pre, Ehr, None;
    "ehr_status_tags_update" => Update, Clinical, EhrStatus, Pre, Ehr, None;
    "ehr_status_update" => Update, Clinical, EhrStatus, Pre, Ehr, None;
    "ehr_tags_get" => Read, Clinical, Ehr, Skip, Ehr, None;
    "group_create" => Create, Clinical, None, Skip, Demographic, None;
    "group_delete" => Delete, Clinical, None, Skip, Demographic, None;
    "group_get" => Read, Clinical, None, Skip, Demographic, None;
    "group_tags_delete" => Delete, Clinical, None, Skip, Demographic, None;
    "group_tags_get" => Read, Clinical, None, Skip, Demographic, None;
    "group_tags_update" => Update, Clinical, None, Skip, Demographic, None;
    "group_update" => Update, Clinical, None, Skip, Demographic, None;
    "management" => Execute, None, None, Skip, ApplicationActivity, None;
    "management_env" => Execute, None, None, Skip, ApplicationActivity, None;
    "management_flamegraph" => Execute, None, None, Skip, ApplicationActivity, None;
    "management_health" => Execute, None, None, Skip, ApplicationActivity, None;
    "management_info" => Execute, None, None, Skip, ApplicationActivity, None;
    "management_loggers_get" => Execute, None, None, Skip, ApplicationActivity, None;
    "management_loggers_reset" => Delete, None, None, Skip, ApplicationActivity, None;
    "management_loggers_set" => Update, None, None, Skip, ApplicationActivity, None;
    "management_metrics" => Execute, None, None, Skip, ApplicationActivity, None;
    "management_prometheus" => Execute, None, None, Skip, ApplicationActivity, None;
    "management_status" => Execute, None, None, Skip, ApplicationActivity, None;
    "options" => Execute, Clinical, None, Skip, ApplicationActivity, None;
    "organisation_create" => Create, Clinical, None, Skip, Demographic, None;
    "organisation_delete" => Delete, Clinical, None, Skip, Demographic, None;
    "organisation_get" => Read, Clinical, None, Skip, Demographic, None;
    "organisation_tags_delete" => Delete, Clinical, None, Skip, Demographic, None;
    "organisation_tags_get" => Read, Clinical, None, Skip, Demographic, None;
    "organisation_tags_update" => Update, Clinical, None, Skip, Demographic, None;
    "organisation_update" => Update, Clinical, None, Skip, Demographic, None;
    "person_create" => Create, Clinical, None, Skip, Demographic, None;
    "person_delete" => Delete, Clinical, None, Skip, Demographic, None;
    "person_get" => Read, Clinical, None, Skip, Demographic, None;
    "person_tags_delete" => Delete, Clinical, None, Skip, Demographic, None;
    "person_tags_get" => Read, Clinical, None, Skip, Demographic, None;
    "person_tags_update" => Update, Clinical, None, Skip, Demographic, None;
    "person_update" => Update, Clinical, None, Skip, Demographic, None;
    "query_execute_adhoc_query" => Search, Clinical, Query, Skip, Query, None;
    "query_execute_adhoc_query_body" => Search, Clinical, Query, Skip, Query, None;
    "query_execute_cohort" => Search, None, Query, Skip, Query, None;
    "query_execute_stored_query" => Search, Clinical, Query, Skip, Query, None;
    "query_execute_stored_query_body" => Search, Clinical, Query, Skip, Query, None;
    "query_execute_stored_query_version" => Search, Clinical, Query, Skip, Query, None;
    "query_execute_stored_query_version_body" => Search, Clinical, Query, Skip, Query, None;
    "role_create" => Create, Clinical, None, Skip, Demographic, None;
    "role_delete" => Delete, Clinical, None, Skip, Demographic, None;
    "role_get" => Read, Clinical, None, Skip, Demographic, None;
    "role_tags_delete" => Delete, Clinical, None, Skip, Demographic, None;
    "role_tags_get" => Read, Clinical, None, Skip, Demographic, None;
    "role_tags_update" => Update, Clinical, None, Skip, Demographic, None;
    "role_update" => Update, Clinical, None, Skip, Demographic, None;
    "versioned_composition_get" => Read, Clinical, Composition, Post, Composition, Read;
    "versioned_composition_revision_history" => Read, Clinical, Composition, Post, Composition, Read;
    "versioned_composition_version_get_at_time" => Read, Clinical, Composition, Post, Composition, Read;
    "versioned_composition_version_get_by_id" => Read, Clinical, Composition, Post, Composition, Read;
    "versioned_ehr_status_get" => Read, Clinical, EhrStatus, Pre, Ehr, None;
    "versioned_ehr_status_revision_history" => Read, Clinical, EhrStatus, Pre, Ehr, None;
    "versioned_ehr_status_version_get_at_time" => Read, Clinical, EhrStatus, Pre, Ehr, None;
    "versioned_ehr_status_version_get_by_id" => Read, Clinical, EhrStatus, Pre, Ehr, None;
    "versioned_party_get" => Read, Clinical, None, Skip, Demographic, None;
    "versioned_party_revision_history" => Read, Clinical, None, Skip, Demographic, None;
    "versioned_party_version_get_at_time" => Read, Clinical, None, Skip, Demographic, None;
    "versioned_party_version_get_by_id" => Read, Clinical, None, Skip, Demographic, None;
};

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// The committed classification of every operation id, one row each.
    fn golden_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/operation-classification.tsv")
    }

    /// Whether `op` is in a generated ITS-REST `ROUTES` table.
    fn generated(op: &str) -> bool {
        [
            openehr_its::rest::generated::ehr::ROUTES,
            openehr_its::rest::generated::definition::ROUTES,
            openehr_its::rest::generated::demographic::ROUTES,
            openehr_its::rest::generated::query::ROUTES,
            openehr_its::rest::generated::admin::ROUTES,
            openehr_its::rest::generated::system::ROUTES,
        ]
        .iter()
        .any(|table| table.iter().any(|(_method, _path, id)| *id == op))
    }

    /// The classification row of `op`, each column over the operations its
    /// callers pass: the RBAC class and the write class over the generated
    /// tables the RBAC route map is built from, the SMART permission over those
    /// and the COMPOSITION operations the policy-enforcement point checks, and
    /// every other column over every operation.
    fn row(op: &str) -> String {
        let generated = generated(op);
        let row = operation(op);
        let gate = row.map_or(CompositionGate::None, |row| row.composition);
        let smart_read = gate != CompositionGate::None;
        let smart_write = gate == CompositionGate::Write;
        let dash = || "-".to_owned();
        [
            op.to_owned(),
            generated.to_string(),
            if generated {
                format!("{:?}", row.and_then(|row| row.class))
            } else {
                dash()
            },
            format!("{:?}", row.and_then(|row| row.kind)),
            format!("{:?}", row.and_then(Operation::access)),
            if generated {
                row.is_none_or(Operation::is_write).to_string()
            } else {
                dash()
            },
            format!("{:?}", row.map_or(Mode::Skip, |row| row.pep)),
            format!("{:?}", row.and_then(Operation::audit)),
            if generated || smart_read || smart_write {
                format!("{:?}", row.map_or(Permission::Read, Operation::permission))
            } else {
                dash()
            },
            smart_read.to_string(),
            smart_write.to_string(),
            row.is_some_and(Operation::reads_composition).to_string(),
        ]
        .join("\t")
    }

    /// The operation ids of the committed classification, in its order.
    fn golden_ops(text: &str) -> Vec<&str> {
        text.lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| line.split('\t').next())
            .collect()
    }

    /// Every operation classifies exactly as the committed classification
    /// records, column by column.
    #[test]
    fn every_operation_classifies_as_recorded() {
        let text = std::fs::read_to_string(golden_path()).expect("read the classification");
        let mut differing = Vec::new();
        for line in text.lines().filter(|line| !line.starts_with('#')) {
            let op = line.split('\t').next().unwrap_or_default();
            let now = row(op);
            if now != line {
                differing.push(format!("recorded: {line}\nnow:      {now}"));
            }
        }
        assert!(
            differing.is_empty(),
            "operations that classify differently from the record:\n{}",
            differing.join("\n")
        );
    }

    /// Every generated operation is in the committed classification.
    #[test]
    fn every_generated_operation_is_recorded() {
        let text = std::fs::read_to_string(golden_path()).expect("read the classification");
        let ops = golden_ops(&text);
        let mut missing = Vec::new();
        for table in [
            openehr_its::rest::generated::ehr::ROUTES,
            openehr_its::rest::generated::definition::ROUTES,
            openehr_its::rest::generated::demographic::ROUTES,
            openehr_its::rest::generated::query::ROUTES,
            openehr_its::rest::generated::admin::ROUTES,
            openehr_its::rest::generated::system::ROUTES,
        ] {
            for (_method, _path, op) in table {
                if !ops.contains(op) {
                    missing.push(*op);
                }
            }
        }
        assert!(
            missing.is_empty(),
            "generated operations missing from the record: {missing:?}"
        );
    }

    /// The rows the policy-enforcement point and the `EHR_ACCESS` gate-keeper
    /// select by their fields are exactly the operations they named before.
    #[test]
    fn the_content_and_gate_keeper_selections_name_their_operations() {
        let ids = |keep: fn(&Operation) -> bool| -> Vec<&str> {
            let mut ids: Vec<&str> = OPERATIONS
                .iter()
                .filter(|row| keep(row))
                .map(|row| row.id)
                .collect();
            ids.sort_unstable();
            ids
        };
        assert_eq!(
            ids(|row| row.composition == CompositionGate::Write),
            [
                "composition_create",
                "composition_delete",
                "composition_update"
            ]
        );
        assert_eq!(
            ids(|row| row.composition == CompositionGate::Write && row.verb == Verb::Delete),
            ["composition_delete"]
        );
        assert_eq!(
            ids(Operation::commits_contribution),
            ["contribution_create"]
        );
    }

    /// Rewrites the committed classification from the current code.
    #[test]
    #[ignore = "the classification writer: run it only for a deliberate change of a classification"]
    fn write_the_operation_classification() {
        let text = std::fs::read_to_string(golden_path()).expect("read the classification");
        let header = "# op\tgenerated\tclass\tkind\taccess\tis_write\tpep_mode\taudit\tsmart_permission\tsmart_read\tsmart_write\tcomposition_read";
        let mut out = vec![header.to_owned()];
        out.extend(golden_ops(&text).into_iter().map(row));
        out.push(String::new());
        std::fs::write(golden_path(), out.join("\n")).expect("write the classification");
    }
}
