// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The classification of every operation the REST layer dispatches: one row
//! per operation id, read by the RBAC route map, the SMART on openEHR gates,
//! the access evaluator's operation class and the ATNA audit trail.
//!
//! Every mounted operation has a row, which the completeness tests of
//! [`crate::extensions::access::authz::classify`] and
//! [`crate::system_log::classify`] and the route-coverage test hold, and every
//! row names the class the operation is audited under. An extension row with
//! no RBAC class is RBAC-classified by its path. No specification governs
//! this: our own design.

use std::collections::HashMap;
use std::sync::LazyLock;

use ferroehr::system_log::event::{EventActionCode, ObjectClass};
use ferrohealth_smart::openehr::smart_scopes::Permission;

use crate::extensions::access::authz::classify::OperationClass;

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
    /// The DICOM resource class the operation is audited under.
    pub(crate) audit: ObjectClass,
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

    /// The audit classification: the DICOM `EventActionCode` of the verb
    /// (PS3.15 §A.5.1) on the operation's resource class.
    #[must_use]
    pub(crate) fn audit(&self) -> (EventActionCode, ObjectClass) {
        let action = match self.verb {
            Verb::Create => EventActionCode::Create,
            Verb::Read => EventActionCode::Read,
            Verb::Update => EventActionCode::Update,
            Verb::Delete => EventActionCode::Delete,
            Verb::Search | Verb::Execute => EventActionCode::Execute,
        };
        (action, self.audit)
    }
}

/// The operation `id`, or `None` for an operation without a row.
#[must_use]
pub(crate) fn operation(id: &str) -> Option<&'static Operation> {
    BY_ID.get(id).copied()
}

/// [`OPERATIONS`] by id.
static BY_ID: LazyLock<HashMap<&str, &Operation>> =
    LazyLock::new(|| OPERATIONS.iter().map(|op| (op.id, op)).collect());

/// Builds [`OPERATIONS`], one row per line: id, verb, RBAC class, audit class
/// and `composition-` gate, `None` for an absent RBAC class.
macro_rules! operations {
    ($($id:expr => $verb:ident, $class:ident, $audit:ident, $gate:ident;)*) => {
        &[$(Operation {
            id: $id,
            verb: Verb::$verb,
            class: operations!(@opt OperationClass $class),
            audit: ObjectClass::$audit,
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
/// `VERSIONED_OBJECT`. An `ITEM_TAG` operation is audited under its parent's
/// class: our own design, since RM ehr `master04-ehr_package.adoc`
/// §Tags puts tags outside change control. `options` is RBAC-classified for
/// totality only: the router mounts the manifest above the authentication
/// layer.
static OPERATIONS: &[Operation] = operations! {
    "admin_access_delay_register" => Read, None, ApplicationActivity, None;
    "admin_access_log_chain" => Read, None, ApplicationActivity, None;
    "admin_access_log_erase" => Delete, None, ApplicationActivity, None;
    "admin_archive_ehrs" => Update, None, ApplicationActivity, None;
    "admin_archive_parties" => Update, None, ApplicationActivity, None;
    "admin_config" => Read, None, ApplicationActivity, None;
    "admin_delay_access" => Create, None, ApplicationActivity, None;
    "admin_dump" => Read, None, ApplicationActivity, None;
    "admin_ehr_access_commit" => Update, None, ApplicationActivity, None;
    "admin_ehr_access_repair" => Update, None, ApplicationActivity, None;
    "admin_ehr_access_versions" => Read, None, ApplicationActivity, None;
    "admin_ehr_delete" => Delete, Admin, Ehr, None;
    "admin_ehr_delete_all" => Delete, Admin, Ehr, None;
    "admin_export_access_log" => Read, None, ApplicationActivity, None;
    "admin_get_access_log_checkpoint" => Read, None, ApplicationActivity, None;
    "admin_lift_restriction" => Update, None, ApplicationActivity, None;
    "admin_list_access_log" => Read, None, ApplicationActivity, None;
    "admin_load" => Create, None, ApplicationActivity, None;
    "admin_party_delete" => Delete, None, Demographic, None;
    "admin_party_identifiers" => Read, None, Demographic, None;
    crate::api::admin::access_policy::HISTORY => Read, None, ApplicationActivity, None;
    crate::api::admin::access_policy::READ => Read, None, ApplicationActivity, None;
    crate::api::admin::access_policy::VERSIONED => Read, None, ApplicationActivity, None;
    crate::api::admin::access_policy::WRITE => Update, None, ApplicationActivity, None;
    crate::api::admin::outbox::ACK => Update, None, ApplicationActivity, None;
    crate::api::admin::outbox::READ => Read, None, ApplicationActivity, None;
    crate::api::admin::registers::HISTORY => Read, None, ApplicationActivity, None;
    crate::api::admin::registers::READ => Read, None, ApplicationActivity, None;
    crate::api::admin::registers::WRITE => Update, None, ApplicationActivity, None;
    "admin_put_retention_policy" => Update, None, ApplicationActivity, None;
    "admin_query_delete" => Delete, None, Query, None;
    "admin_rebuild_version_nodes" => Update, None, ApplicationActivity, None;
    "admin_rekey_party" => Update, None, ApplicationActivity, None;
    "admin_release_access_delay" => Update, None, ApplicationActivity, None;
    "admin_report_composition_version_count" => Read, None, ApplicationActivity, None;
    "admin_report_contribution_count" => Read, None, ApplicationActivity, None;
    "admin_report_contributions" => Read, None, ApplicationActivity, None;
    "admin_report_versioned_composition_count" => Read, None, ApplicationActivity, None;
    "admin_research_objection" => Create, None, ApplicationActivity, None;
    "admin_restore_archived_ehrs" => Update, None, ApplicationActivity, None;
    "admin_restore_archived_parties" => Update, None, ApplicationActivity, None;
    "admin_restrict_processing" => Create, None, ApplicationActivity, None;
    "admin_restriction_register" => Read, None, ApplicationActivity, None;
    "admin_retention_anchor" => Update, None, ApplicationActivity, None;
    "admin_retention_due" => Read, None, ApplicationActivity, None;
    "admin_retention_hold" => Create, None, ApplicationActivity, None;
    "admin_retention_policies" => Read, None, ApplicationActivity, None;
    "admin_seal_access_log_checkpoint" => Create, None, ApplicationActivity, None;
    "admin_template_delete" => Delete, None, Template, None;
    "admin_verify_access_log" => Execute, None, ApplicationActivity, None;
    "admin_verify_storage_parity" => Execute, None, ApplicationActivity, None;
    "agent_create" => Create, Clinical, Demographic, None;
    "agent_delete" => Delete, Clinical, Demographic, None;
    "agent_get" => Read, Clinical, Demographic, None;
    "agent_tags_delete" => Delete, Clinical, Demographic, None;
    "agent_tags_get" => Read, Clinical, Demographic, None;
    "agent_tags_update" => Update, Clinical, Demographic, None;
    "agent_update" => Update, Clinical, Demographic, None;
    crate::api::ehr::access_delay::RELEASE => Update, None, Composition, Read;
    "audit_event_search" => Search, None, ApplicationActivity, None;
    "composition_create" => Create, Clinical, Composition, Write;
    "composition_delete" => Delete, Clinical, Composition, Write;
    "composition_get" => Read, Clinical, Composition, Read;
    "composition_tags_delete" => Delete, Clinical, Composition, None;
    "composition_tags_get" => Read, Clinical, Composition, None;
    "composition_tags_update" => Update, Clinical, Composition, None;
    "composition_update" => Update, Clinical, Composition, Write;
    "contribution_create" => Create, Clinical, Contribution, None;
    "contribution_get" => Read, Clinical, Contribution, None;
    "contribution_list" => Read, None, Contribution, None;
    "definition_archetype_adl14_convert" => Read, None, Template, None;
    "definition_archetype_adl14_delete" => Delete, None, Template, None;
    "definition_archetype_adl14_get" => Read, None, Template, None;
    "definition_archetype_adl14_list" => Read, None, Template, None;
    "definition_archetype_adl14_upload" => Create, None, Template, None;
    "definition_archetype_adl2_count" => Read, None, Template, None;
    "definition_archetype_adl2_list" => Read, None, Template, None;
    "definition_artefact_adl2_count" => Read, None, Template, None;
    "definition_artefact_adl2_delete" => Delete, None, Template, None;
    "definition_artefact_adl2_list" => Read, None, Template, None;
    "definition_query_list" => Read, Clinical, Query, None;
    "definition_query_list_all" => Read, None, Query, None;
    "definition_query_store.yaml" => Create, Clinical, Query, None;
    "definition_query_version_get" => Read, Clinical, Query, None;
    "definition_query_version_store.yaml" => Create, Clinical, Query, None;
    "definition_template_adl1.4_example_get" => Read, Clinical, Template, None;
    "definition_template_adl1.4_get" => Read, Clinical, Template, None;
    "definition_template_adl1.4_list" => Read, Clinical, Template, None;
    "definition_template_adl1.4_upload" => Create, Clinical, Template, None;
    "definition_template_adl14_convert" => Read, None, Template, None;
    "definition_template_adl2_example_get" => Read, Clinical, Template, None;
    "definition_template_adl2_get" => Read, Clinical, Template, None;
    "definition_template_adl2_list" => Read, Clinical, Template, None;
    "definition_template_adl2_upload" => Create, Clinical, Template, None;
    "definition_template_adl2_version_get" => Read, Clinical, Template, None;
    "demographic_tags_get" => Read, Clinical, Demographic, None;
    "directory_create" => Create, Clinical, Directory, None;
    "directory_delete" => Delete, Clinical, Directory, None;
    "directory_get_at_time" => Read, Clinical, Directory, None;
    "directory_get_by_version_id" => Read, Clinical, Directory, None;
    "directory_update" => Update, Clinical, Directory, None;
    "ehr_access_get_at_time" => Read, None, Ehr, None;
    "ehr_access_get_by_version_id" => Read, None, Ehr, None;
    "ehr_access_log_get" => Read, None, ApplicationActivity, None;
    "ehr_access_log_list" => Read, None, ApplicationActivity, None;
    "ehr_create" => Create, Clinical, Ehr, None;
    "ehr_create_with_id" => Create, Clinical, Ehr, None;
    "ehr_get_by_id" => Read, Clinical, Ehr, None;
    "ehr_get_by_subject" => Read, Clinical, Ehr, None;
    "ehr_status_get_at_time" => Read, Clinical, Ehr, None;
    "ehr_status_get_by_version_id" => Read, Clinical, Ehr, None;
    "ehr_status_tags_delete" => Delete, Clinical, Ehr, None;
    "ehr_status_tags_get" => Read, Clinical, Ehr, None;
    "ehr_status_tags_update" => Update, Clinical, Ehr, None;
    "ehr_status_update" => Update, Clinical, Ehr, None;
    "ehr_tags_get" => Read, Clinical, Ehr, None;
    "group_create" => Create, Clinical, Demographic, None;
    "group_delete" => Delete, Clinical, Demographic, None;
    "group_get" => Read, Clinical, Demographic, None;
    "group_tags_delete" => Delete, Clinical, Demographic, None;
    "group_tags_get" => Read, Clinical, Demographic, None;
    "group_tags_update" => Update, Clinical, Demographic, None;
    "group_update" => Update, Clinical, Demographic, None;
    "management" => Execute, None, ApplicationActivity, None;
    "management_env" => Execute, None, ApplicationActivity, None;
    "management_flamegraph" => Execute, None, ApplicationActivity, None;
    "management_health" => Execute, None, ApplicationActivity, None;
    "management_info" => Execute, None, ApplicationActivity, None;
    "management_loggers_get" => Execute, None, ApplicationActivity, None;
    "management_loggers_reset" => Delete, None, ApplicationActivity, None;
    "management_loggers_set" => Update, None, ApplicationActivity, None;
    "management_metrics" => Execute, None, ApplicationActivity, None;
    "management_prometheus" => Execute, None, ApplicationActivity, None;
    "management_status" => Execute, None, ApplicationActivity, None;
    "message_export_ehr_extracts" => Read, None, ApplicationActivity, None;
    "message_export_ehrs" => Read, None, ApplicationActivity, None;
    "message_import_ehr" => Create, None, Ehr, None;
    "message_import_ehr_extract" => Update, None, Ehr, None;
    "options" => Execute, Clinical, ApplicationActivity, None;
    "organisation_create" => Create, Clinical, Demographic, None;
    "organisation_delete" => Delete, Clinical, Demographic, None;
    "organisation_get" => Read, Clinical, Demographic, None;
    "organisation_tags_delete" => Delete, Clinical, Demographic, None;
    "organisation_tags_get" => Read, Clinical, Demographic, None;
    "organisation_tags_update" => Update, Clinical, Demographic, None;
    "organisation_update" => Update, Clinical, Demographic, None;
    "party_relationship_create" => Create, None, Demographic, None;
    "party_relationship_delete" => Delete, None, Demographic, None;
    "party_relationship_get" => Read, None, Demographic, None;
    "party_relationship_revision_history" => Read, None, Demographic, None;
    "party_relationship_update" => Update, None, Demographic, None;
    "party_relationship_version_get_at_time" => Read, None, Demographic, None;
    "party_relationship_version_get_by_id" => Read, None, Demographic, None;
    "party_reverse_relationships_get" => Read, None, Demographic, None;
    "person_create" => Create, Clinical, Demographic, None;
    "person_delete" => Delete, Clinical, Demographic, None;
    "person_get" => Read, Clinical, Demographic, None;
    "person_tags_delete" => Delete, Clinical, Demographic, None;
    "person_tags_get" => Read, Clinical, Demographic, None;
    "person_tags_update" => Update, Clinical, Demographic, None;
    "person_update" => Update, Clinical, Demographic, None;
    "query_execute_adhoc_query" => Search, Clinical, Query, None;
    "query_execute_adhoc_query_body" => Search, Clinical, Query, None;
    "query_execute_cohort" => Search, None, Query, None;
    "query_execute_stored_query" => Search, Clinical, Query, None;
    "query_execute_stored_query_body" => Search, Clinical, Query, None;
    "query_execute_stored_query_version" => Search, Clinical, Query, None;
    "query_execute_stored_query_version_body" => Search, Clinical, Query, None;
    "role_create" => Create, Clinical, Demographic, None;
    "role_delete" => Delete, Clinical, Demographic, None;
    "role_get" => Read, Clinical, Demographic, None;
    "role_tags_delete" => Delete, Clinical, Demographic, None;
    "role_tags_get" => Read, Clinical, Demographic, None;
    "role_tags_update" => Update, Clinical, Demographic, None;
    "role_update" => Update, Clinical, Demographic, None;
    "scim_group_create" => Create, None, Demographic, None;
    "scim_group_delete" => Delete, None, Demographic, None;
    "scim_group_get" => Read, None, Demographic, None;
    "scim_group_list" => Search, None, Demographic, None;
    "scim_group_patch" => Update, None, Demographic, None;
    "scim_group_replace" => Update, None, Demographic, None;
    "scim_user_create" => Create, None, Demographic, None;
    "scim_user_delete" => Delete, None, Demographic, None;
    "scim_user_get" => Read, None, Demographic, None;
    "scim_user_list" => Search, None, Demographic, None;
    "scim_user_patch" => Update, None, Demographic, None;
    "scim_user_replace" => Update, None, Demographic, None;
    "terminology_description" => Read, None, ApplicationActivity, None;
    "terminology_get_term" => Read, None, ApplicationActivity, None;
    "terminology_ids" => Read, None, ApplicationActivity, None;
    "terminology_subsumes" => Read, None, ApplicationActivity, None;
    "terminology_value_set" => Read, None, ApplicationActivity, None;
    "terminology_value_set_validate" => Read, None, ApplicationActivity, None;
    "versioned_composition_get" => Read, Clinical, Composition, Read;
    "versioned_composition_revision_history" => Read, Clinical, Composition, Read;
    "versioned_composition_version_get_at_time" => Read, Clinical, Composition, Read;
    "versioned_composition_version_get_by_id" => Read, Clinical, Composition, Read;
    "versioned_ehr_access_get" => Read, None, Ehr, None;
    "versioned_ehr_access_revision_history" => Read, None, Ehr, None;
    "versioned_ehr_access_version_get_at_time" => Read, None, Ehr, None;
    "versioned_ehr_access_version_get_by_id" => Read, None, Ehr, None;
    "versioned_ehr_status_get" => Read, Clinical, Ehr, None;
    "versioned_ehr_status_revision_history" => Read, Clinical, Ehr, None;
    "versioned_ehr_status_version_get_at_time" => Read, Clinical, Ehr, None;
    "versioned_ehr_status_version_get_by_id" => Read, Clinical, Ehr, None;
    "versioned_party_get" => Read, Clinical, Demographic, None;
    "versioned_party_relationship_get" => Read, None, Demographic, None;
    "versioned_party_revision_history" => Read, Clinical, Demographic, None;
    "versioned_party_version_get_at_time" => Read, Clinical, Demographic, None;
    "versioned_party_version_get_by_id" => Read, Clinical, Demographic, None;
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
            if generated {
                row.is_none_or(Operation::is_write).to_string()
            } else {
                dash()
            },
            format!("{:?}", row.map(Operation::audit)),
            if generated || smart_read || smart_write {
                format!("{:?}", row.map_or(Permission::Read, Operation::permission))
            } else {
                dash()
            },
            smart_read.to_string(),
            smart_write.to_string(),
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

    /// The rows the policy-enforcement point selects by their fields are
    /// exactly the operations it named before.
    #[test]
    fn the_composition_gate_selections_name_their_operations() {
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
    }

    /// Every item-tag operation is audited under the class of its target's
    /// read (`GET`) or write (`PUT`, `DELETE`).
    #[test]
    fn an_item_tag_operation_is_audited_as_its_target() {
        let targets = [
            ("agent_tags_delete", "agent_update"),
            ("agent_tags_get", "agent_get"),
            ("agent_tags_update", "agent_update"),
            ("composition_tags_delete", "composition_update"),
            ("composition_tags_get", "composition_get"),
            ("composition_tags_update", "composition_update"),
            ("demographic_tags_get", "versioned_party_get"),
            ("ehr_status_tags_delete", "ehr_status_update"),
            ("ehr_status_tags_get", "ehr_status_get_at_time"),
            ("ehr_status_tags_update", "ehr_status_update"),
            ("ehr_tags_get", "ehr_get_by_id"),
            ("group_tags_delete", "group_update"),
            ("group_tags_get", "group_get"),
            ("group_tags_update", "group_update"),
            ("organisation_tags_delete", "organisation_update"),
            ("organisation_tags_get", "organisation_get"),
            ("organisation_tags_update", "organisation_update"),
            ("person_tags_delete", "person_update"),
            ("person_tags_get", "person_get"),
            ("person_tags_update", "person_update"),
            ("role_tags_delete", "role_update"),
            ("role_tags_get", "role_get"),
            ("role_tags_update", "role_update"),
        ];
        let tag_ops: Vec<&str> = OPERATIONS
            .iter()
            .map(|row| row.id)
            .filter(|id| id.contains("_tags_"))
            .collect();
        assert_eq!(
            tag_ops,
            targets.map(|(tag, _target)| tag),
            "every item-tag operation names its target"
        );
        for (tag, target) in targets {
            let (tag_row, target_row) = (operation(tag), operation(target));
            assert_eq!(
                tag_row.map(|row| row.audit),
                target_row.map(|row| row.audit),
                "{tag} is audited as {target}"
            );
        }
    }

    /// Rewrites the committed classification from the current code.
    #[test]
    #[ignore = "the classification writer: run it only for a deliberate change of a classification"]
    fn write_the_operation_classification() {
        let text = std::fs::read_to_string(golden_path()).expect("read the classification");
        let header =
            "# op\tgenerated\tclass\tis_write\taudit\tsmart_permission\tsmart_read\tsmart_write";
        let mut out = vec![header.to_owned()];
        out.extend(golden_ops(&text).into_iter().map(row));
        out.push(String::new());
        std::fs::write(golden_path(), out.join("\n")).expect("write the classification");
    }
}
