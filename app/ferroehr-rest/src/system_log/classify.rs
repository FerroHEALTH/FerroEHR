// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Operation → DICOM audit classification for the IHE ATNA system log.
//!
//! ATNA records, per audited access, who did what to which resource with what
//! outcome, as a DICOM Audit Message (PS3.15 §A.5). The operation's row in
//! `crate::api::operation` supplies the what-and-which half for one operation:
//! its `EventActionCode` (§A.5.1) and the resource `ObjectClass` driving the
//! participant-object rendering in the platform emitter. openEHR is silent on
//! audit-record shape, so the mapping is our own design over those external
//! standards.
//!
//! Every row names its audit class, and every mounted route's operation has a
//! row: the completeness tests here and the route-coverage test
//! (`tests/it/audit_route_coverage.rs`) fail the build on an operation without
//! one. There is no fallback class: the dispatcher refuses an operation without
//! a row before its handler runs, and the audit middleware refuses the response
//! of one that reaches it.

use ferroehr::system_log::event::{EventActionCode, ObjectClass};

use crate::api::operation::{Operation, operation};

/// Returns the `(action, object)` the operation `op` is audited under, or
/// `None` for an operation without a row in the operation table.
#[must_use]
pub fn audit_for(op: &str) -> Option<(EventActionCode, ObjectClass)> {
    operation(op).map(Operation::audit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use EventActionCode::{Create, Delete, Execute, Read, Update};
    use ObjectClass::{Composition, Contribution, Demographic, Directory, Ehr, Query, Template};
    use ferroehr::system_log::event::AccessDomain;

    /// Every operation id in every generated ITS-REST `ROUTES` table.
    fn all_route_ops() -> Vec<&'static str> {
        let mut ops = Vec::new();
        for table in [
            openehr_its::rest::generated::ehr::ROUTES,
            openehr_its::rest::generated::definition::ROUTES,
            openehr_its::rest::generated::demographic::ROUTES,
            openehr_its::rest::generated::query::ROUTES,
            openehr_its::rest::generated::admin::ROUTES,
            openehr_its::rest::generated::system::ROUTES,
        ] {
            for (_method, _path, op) in table {
                ops.push(*op);
            }
        }
        ops
    }

    /// The op ids the released OAS deliberately reuses across group bundles,
    /// each adjudicated to ONE family-invariant classification (see the
    /// CONTRIBUTION row note in [`crate::api::operation`]). A NEW cross-group duplicate fails
    /// [`adjudicated_shared_ids`] until it is adjudicated here (#1707).
    const ADJUDICATED_SHARED: &[&str] = &["contribution_create", "contribution_get"];

    /// Collision gate (#1707): an op id in more than one generated group
    /// table is only legal when its shared classification is adjudicated —
    /// the classifiers are op-id-keyed, so an unadjudicated duplicate could
    /// silently classify one family as the other.
    #[test]
    fn adjudicated_shared_ids() {
        use std::collections::BTreeMap;
        let mut owners: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (group, table) in [
            ("ehr", openehr_its::rest::generated::ehr::ROUTES),
            (
                "definition",
                openehr_its::rest::generated::definition::ROUTES,
            ),
            (
                "demographic",
                openehr_its::rest::generated::demographic::ROUTES,
            ),
            ("query", openehr_its::rest::generated::query::ROUTES),
            ("admin", openehr_its::rest::generated::admin::ROUTES),
            ("system", openehr_its::rest::generated::system::ROUTES),
        ] {
            for (_m, _p, op) in table {
                let groups = owners.entry(op).or_default();
                if !groups.contains(&group) {
                    groups.push(group);
                }
            }
        }
        let unadjudicated: Vec<(&str, &Vec<&str>)> = owners
            .iter()
            .filter(|(op, groups)| groups.len() > 1 && !ADJUDICATED_SHARED.contains(*op))
            .map(|(op, groups)| (*op, groups))
            .collect();
        assert!(
            unadjudicated.is_empty(),
            "op ids reused across generated groups without an adjudicated shared \
             classification (#1707 — adjudicate in the classifier and extend \
             ADJUDICATED_SHARED): {unadjudicated:?}"
        );
        // …and the adjudicated list itself never rots: every entry is still a
        // real cross-group duplicate.
        for op in ADJUDICATED_SHARED {
            assert!(
                owners.get(op).is_some_and(|g| g.len() > 1),
                "{op} is no longer shared across groups — remove it from ADJUDICATED_SHARED"
            );
        }
    }

    /// Completeness guard: every generated operation id has a row, so an
    /// audit class. A newly generated, unclassified operation fails this test.
    #[test]
    fn every_generated_operation_is_explicit() {
        let mut missing = Vec::new();
        for op in all_route_ops() {
            if audit_for(op).is_none() {
                missing.push(op);
            }
        }
        assert!(
            missing.is_empty(),
            "generated ITS-REST operations with no explicit audit classification \
             (add them to app/ferroehr-rest/src/api/operation.rs): {missing:?}"
        );
    }

    /// Every generated read of patient data emits an access record, in its own
    /// pseudonymisation domain.
    #[test]
    fn every_read_operation_produces_an_access_record() {
        // NEN 7513 requires per-access logging of who read which record, EHDS
        // Art. 9 requires logging of access to health data for primary use, and
        // the Wabvpz gives the patient the right to know who consulted the
        // record. A retrieval that emits nothing is unanswerable to all three,
        // so the property is asserted over the GENERATED route tables of the
        // three APIs that serve patient data — a newly generated GET reaches
        // this test before it reaches a deployment.
        let mut silent = Vec::new();
        let mut misclassified = Vec::new();
        for (group, table, expected) in [
            (
                "ehr",
                openehr_its::rest::generated::ehr::ROUTES,
                AccessDomain::Ehr,
            ),
            (
                "query",
                openehr_its::rest::generated::query::ROUTES,
                AccessDomain::Ehr,
            ),
            (
                "demographic",
                openehr_its::rest::generated::demographic::ROUTES,
                AccessDomain::Demographic,
            ),
        ] {
            for (method, path, op) in table {
                if !method.eq_ignore_ascii_case("get") {
                    continue;
                }
                let Some((action, object)) = audit_for(op) else {
                    silent.push(format!("{group} {method} {path} ({op})"));
                    continue;
                };
                // A GET is a Read; the one sanctioned exception is a query
                // execution, which the DICOM vocabulary classes as an Execute
                // on the Query object (PS3.15 §A.5.1).
                let read_shaped = action == Read || (action == Execute && object == Query);
                // An operation id the released OAS reuses across bundles cannot
                // name its own domain: `contribution_get` is the same id in the
                // EHR and the DEMOGRAPHIC group. Those carry a handler-set
                // override (`AuditObject::domain`) this static table cannot
                // see, and the integration suite is where the override is
                // proven; `adjudicated_shared_ids` keeps the list honest.
                let domain_is_static = !ADJUDICATED_SHARED.contains(op);
                if !read_shaped || (domain_is_static && AccessDomain::of(object) != expected) {
                    misclassified.push(format!(
                        "{group} {method} {path} ({op}) → {action:?}/{object:?} \
                         in domain {:?}, expected a read in {expected:?}",
                        AccessDomain::of(object)
                    ));
                }
            }
        }
        assert!(
            silent.is_empty(),
            "read operations that emit no access record: {silent:#?}"
        );
        assert!(
            misclassified.is_empty(),
            "read operations classified outside their own domain: {misclassified:#?}"
        );
    }

    /// An operation without a row has no audit class: there is no fallback
    /// class it could be recorded under.
    #[test]
    fn an_operation_without_a_row_has_no_audit_class() {
        assert_eq!(audit_for("no_such_operation"), None);
        assert_eq!(audit_for("some_future_extension_op"), None);
    }

    #[test]
    fn representative_mappings() {
        assert_eq!(audit_for("ehr_create"), Some((Create, Ehr)));
        assert_eq!(audit_for("ehr_status_update"), Some((Update, Ehr)));
        assert_eq!(audit_for("composition_delete"), Some((Delete, Composition)));
        assert_eq!(audit_for("admin_party_delete"), Some((Delete, Demographic)));
        assert_eq!(audit_for("directory_create"), Some((Create, Directory)));
        assert_eq!(audit_for("contribution_get"), Some((Read, Contribution)));
        assert_eq!(
            audit_for("query_execute_adhoc_query"),
            Some((Execute, Query))
        );
        assert_eq!(
            audit_for("definition_query_store.yaml"),
            Some((Create, Query))
        );
        assert_eq!(audit_for("admin_ehr_delete_all"), Some((Delete, Ehr)));
        assert_eq!(
            audit_for("definition_template_adl2_upload"),
            Some((Create, Template))
        );
        assert_eq!(
            audit_for("definition_template_adl1.4_get"),
            Some((Read, Template))
        );
        assert_eq!(
            audit_for("definition_template_adl14_convert"),
            Some((Read, Template))
        );
        assert_eq!(audit_for("person_create"), Some((Create, Demographic)));
        assert_eq!(audit_for("role_tags_delete"), Some((Delete, Demographic)));
        assert_eq!(audit_for("ehr_status_tags_get"), Some((Read, Ehr)));
    }
}
