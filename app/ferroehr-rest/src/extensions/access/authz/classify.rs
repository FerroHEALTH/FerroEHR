// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The coarse authorization class of an operation, as the operation table
//! (`crate::api::operation`) records it per id.
//!
//! Every generated operation is explicitly classified; one without a row fails
//! the total-coverage guard tests below.
//!
//! Only the two `admin_*` operations are [`OperationClass::Admin`]; every other
//! generated route is [`OperationClass::Clinical`], requiring any authenticated
//! principal with a role. [`OperationClass::Public`] classifies the non-generated
//! surface, which has no class in the table, and the management surface is
//! classified by its own per-endpoint guard against `[management.endpoints]`.

/// The coarse authorization class of an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationClass {
    /// No authorization check (root/status/health per the router).
    Public,
    /// Any authenticated principal with at least one role.
    Clinical,
    /// Administrative operations — require `rbac.admin_role`. The management
    /// surface reaches this class through its own per-endpoint guard, which
    /// maps an `admin_only` endpoint level onto it; `[management]` owns that
    /// decision, and there is no second RBAC key spelling it.
    Admin,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::operation::{Operation, operation};

    fn class_of(op: &str) -> Option<OperationClass> {
        operation(op).and_then(|o| o.class)
    }

    /// The write class the read-only gate applies: an operation without a row
    /// is a write.
    fn is_write(op: &str) -> bool {
        operation(op).is_none_or(Operation::is_write)
    }

    /// The write class of an operation with a row, `None` without one.
    fn write_verb(op: &str) -> Option<bool> {
        operation(op).map(Operation::is_write)
    }

    /// The generated ITS-REST route tables, all groups.
    fn all_route_ops() -> Vec<&'static str> {
        let mut ops = Vec::new();
        for table in [
            openehr_its::rest::generated::ehr::ROUTES,
            openehr_its::rest::generated::definition::ROUTES,
            openehr_its::rest::generated::demographic::ROUTES,
            openehr_its::rest::generated::query::ROUTES,
            openehr_its::rest::generated::admin::ROUTES,
        ] {
            for (_method, _path, op) in table {
                ops.push(*op);
            }
        }
        ops
    }

    /// Every generated operation id has an [`OperationClass`]. A new,
    /// unclassified generated operation fails this test.
    #[test]
    fn every_operation_is_classified() {
        let mut unclassified = Vec::new();
        for op in all_route_ops() {
            if class_of(op).is_none() {
                unclassified.push(op);
            }
        }
        assert!(
            unclassified.is_empty(),
            "unclassified ITS-REST operations (add a row to app/ferroehr-rest/src/api/operation.rs): {unclassified:?}"
        );
    }

    /// The only Admin-class generated routes are the two admin-API deletes;
    /// everything else generated is Clinical.
    #[test]
    fn admin_routes_are_the_only_admin_class() {
        let mut admin: Vec<&str> = all_route_ops()
            .into_iter()
            .filter(|op| class_of(op) == Some(OperationClass::Admin))
            .collect();
        admin.sort_unstable();
        admin.dedup();
        assert_eq!(admin, vec!["admin_ehr_delete", "admin_ehr_delete_all"]);
    }

    #[test]
    fn known_mappings() {
        assert_eq!(class_of("ehr_create"), Some(OperationClass::Clinical));
        assert_eq!(class_of("admin_ehr_delete"), Some(OperationClass::Admin));
        assert_eq!(
            class_of("definition_template_adl1.4_upload"),
            Some(OperationClass::Clinical)
        );
        assert_eq!(class_of("person_get"), Some(OperationClass::Clinical));
        assert_eq!(class_of("no_such_operation"), None);
    }

    /// Every generated operation id classifies write/read via a positive verb
    /// rule — no op falls through to [`is_write`]'s fail-safe default. A new,
    /// unrecognized generated op fails this test until [`write_verb`] handles it.
    #[test]
    fn every_operation_has_a_write_class() {
        let mut unclassified = Vec::new();
        for op in all_route_ops() {
            if write_verb(op).is_none() {
                unclassified.push(op);
            }
        }
        assert!(
            unclassified.is_empty(),
            "ops with no write/read verb rule (add a row to app/ferroehr-rest/src/api/operation.rs): {unclassified:?}"
        );
    }

    #[test]
    fn write_classification_exemplars() {
        // Writes: create / update / delete / upload / store, tag mutations, admin.
        assert!(is_write("ehr_create"));
        assert!(is_write("ehr_create_with_id"));
        assert!(is_write("composition_create"));
        assert!(is_write("composition_update"));
        assert!(is_write("composition_delete"));
        assert!(is_write("ehr_status_update"));
        assert!(is_write("contribution_create"));
        assert!(is_write("directory_create"));
        assert!(is_write("definition_template_adl1.4_upload"));
        assert!(is_write("definition_template_adl2_upload"));
        assert!(is_write("definition_query_store.yaml"));
        assert!(is_write("definition_query_version_store.yaml"));
        assert!(is_write("composition_tags_update"));
        assert!(is_write("composition_tags_delete"));
        assert!(is_write("ehr_status_tags_update"));
        assert!(is_write("person_create"));
        assert!(is_write("agent_tags_delete"));
        assert!(is_write("admin_ehr_delete"));
        assert!(is_write("admin_ehr_delete_all"));
        // Reads: GET-semantics ops, plus AQL execution (POST-but-read).
        assert!(!is_write("ehr_get_by_id"));
        assert!(!is_write("ehr_get_by_subject"));
        assert!(!is_write("composition_get"));
        assert!(!is_write("versioned_composition_get"));
        assert!(!is_write("versioned_composition_revision_history"));
        assert!(!is_write("versioned_composition_version_get_at_time"));
        assert!(!is_write("ehr_status_get_at_time"));
        assert!(!is_write("directory_get_by_version_id"));
        assert!(!is_write("contribution_get"));
        assert!(!is_write("ehr_tags_get"));
        assert!(!is_write("composition_tags_get"));
        assert!(!is_write("definition_template_adl1.4_list"));
        assert!(!is_write("definition_template_adl2_version_get"));
        assert!(!is_write("definition_query_list"));
        assert!(!is_write("query_execute_adhoc_query"));
        assert!(!is_write("query_execute_adhoc_query_body"));
        assert!(!is_write("query_execute_stored_query_version_body"));
        // The cohort extension selects; it never commits.
        assert!(!is_write("query_execute_cohort"));
        assert!(!is_write("person_get"));
        assert!(!is_write("versioned_party_revision_history"));
        // Fail-safe: an unrecognized op is treated as a write.
        assert!(is_write("no_such_operation"));
    }
}
