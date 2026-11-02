// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Operation → DICOM audit classification for the IHE ATNA system log.
//!
//! ATNA records, per audited access, who did what to which resource with what
//! outcome, as a DICOM Audit Message (PS3.15 §A.5). The operation's row in
//! `crate::api::operation` supplies the what-and-which half for one ITS-REST
//! operation: its `EventActionCode` (§A.5.1) and the resource `ObjectClass`
//! driving the participant-object rendering in the platform emitter. openEHR is
//! silent on audit-record shape, so the mapping is our own design over those
//! external standards.
//!
//! Every operation id in every generated `ROUTES` table is explicitly
//! classified, and the completeness test fails the build the moment a newly
//! generated one is not. An id without an audit class (an extension route or a
//! future operation) resolves through the conservative [`DEFAULT`], so it is
//! still recorded, attributing the caller and outcome without asserting a false
//! resource class. An operation is left unaudited only by an explicit
//! [`Classification::Unaudited`] entry.

use ferroehr::system_log::event::{EventActionCode, ObjectClass};

use crate::api::operation::{Operation, operation};

/// The audit classification of an ITS-REST operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// The operation emits an audit record with this DICOM action + resource class.
    Audited {
        /// The DICOM `EventActionCode` (PS3.15 §A.5.1): `C`/`R`/`U`/`D`/`E`.
        action: EventActionCode,
        /// The touched resource class (drives `EventID` + participant objects).
        object: ObjectClass,
    },
    /// The operation is deliberately not audited: a reviewed non-clinical
    /// opt-out, distinct from an unrecognised operation, which fails closed to
    /// [`DEFAULT`]. No generated operation uses this, and the completeness test
    /// asserts the allowlist is empty; it is the seam a future non-clinical
    /// route would use.
    Unaudited,
}

impl Classification {
    /// Convenience constructor for an audited entry.
    const fn audited(action: EventActionCode, object: ObjectClass) -> Self {
        Classification::Audited { action, object }
    }
}

/// The fail-closed classification for an operation id absent from the table.
///
/// Such an operation is audited as an `Execute` on the generic
/// `ApplicationActivity` class rather than silently dropped; extension routes
/// and future operations land here until given an explicit entry.
pub const DEFAULT: Classification =
    Classification::audited(EventActionCode::Execute, ObjectClass::ApplicationActivity);

/// Classifies an operation id for the request path: its explicit audit class,
/// or the fail-closed [`DEFAULT`] when the id has none.
///
/// Never yields "unknown": an unrecognised operation is audited under the
/// generic class rather than dropped.
#[must_use]
pub fn classify(op: &str) -> Classification {
    operation(op).and_then(Operation::audit).unwrap_or(DEFAULT)
}

/// Returns the `(action, object)` an operation is audited under, or `None` for
/// an explicit [`Classification::Unaudited`] opt-out.
///
/// The entry point the audit middleware calls: an unrecognised id resolves
/// through [`DEFAULT`], so only a deliberate opt-out suppresses the record.
#[must_use]
pub fn audit_for(op: &str) -> Option<(EventActionCode, ObjectClass)> {
    match classify(op) {
        Classification::Audited { action, object } => Some((action, object)),
        Classification::Unaudited => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use EventActionCode::{Create, Delete, Execute, Read, Update};
    use ObjectClass::{Composition, Contribution, Demographic, Directory, Ehr, Query, Template};
    use ferroehr::system_log::event::AccessDomain;

    /// The explicit audit class of `op`, or `None` when its row names none.
    fn lookup(op: &str) -> Option<Classification> {
        operation(op).and_then(Operation::audit)
    }

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

    /// Completeness guard: every generated operation id has an **explicit**
    /// table entry (not merely the fail-closed default). A newly generated,
    /// unclassified operation fails this test — the coverage discipline.
    #[test]
    fn every_generated_operation_is_explicit() {
        let mut missing = Vec::new();
        for op in all_route_ops() {
            if lookup(op).is_none() {
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

    #[test]
    /// Coverage is total: every generated route entry is `Audited` and the
    /// explicit `Unaudited` allowlist is empty.
    /// Coverage is total: every generated route entry is `Audited` and the
    /// explicit `Unaudited` allowlist is empty.
    fn generated_coverage_is_total_and_audited() {
        let ops = all_route_ops();
        let mut unaudited = Vec::new();
        for op in &ops {
            if lookup(op) == Some(Classification::Unaudited) {
                unaudited.push(*op);
            }
        }
        assert!(
            unaudited.is_empty(),
            "generated ops explicitly opted out of auditing: {unaudited:?}"
        );
        assert!(
            ops.iter().all(|op| audit_for(op).is_some()),
            "every generated route entry must produce an audit record"
        );
    }

    /// An unrecognised operation id (extension route / future op) fails closed
    /// to the documented default — it is audited, never silently dropped.
    #[test]
    fn unrecognised_operation_fails_closed_to_default() {
        assert_eq!(classify("terminology_expand_value_set"), DEFAULT);
        assert_eq!(classify("some_future_extension_op"), DEFAULT);
        assert_eq!(
            audit_for("no_such_operation"),
            Some((Execute, ObjectClass::ApplicationActivity))
        );
        // lookup, in contrast, reports the raw table miss.
        assert_eq!(lookup("no_such_operation"), None);
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
