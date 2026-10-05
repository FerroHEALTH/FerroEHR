// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Operation → DICOM audit classification for the IHE ATNA system log.
//!
//! ATNA records, per audited access, who did what to which resource with what
//! outcome, as a DICOM Audit Message (PS3.15 §A.5). This table supplies the
//! what-and-which half for one ITS-REST operation: its `EventActionCode`
//! (§A.5.1) and the resource `ObjectClass` driving the participant-object
//! rendering in the platform emitter. openEHR is silent on audit-record shape —
//! the only normative line is "System Log | IHE ATNA-compliant system log" (SM
//! `master02-overview.adoc` §openEHR Platform Model) — so the mapping here is
//! our own design over those external standards.
//!
//! Every operation id in every generated `ROUTES` table is explicitly
//! classified, and the completeness test fails the build the moment a newly
//! generated one is not. An id absent from the table — an extension route or a
//! future operation — resolves through the conservative [`DEFAULT`], so it is
//! still recorded, attributing the caller and outcome without asserting a false
//! resource class. An operation is left unaudited only by an explicit
//! [`Classification::Unaudited`] entry.

use ferroehr::system_log::event::{EventActionCode, ObjectClass};

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
    Classification::audited(Execute, ObjectClass::ApplicationActivity);

use EventActionCode::{Create, Delete, Execute, Read, Update};
use ObjectClass::{Composition, Contribution, Demographic, Directory, Ehr, Query, Template};

/// The management surface's operation ids (#3244): reads of the node's own
/// state are application-activity Executes; a runtime filter change is an
/// Update and its reset a Delete, so the trail says which act it recorded.
fn management_lookup(op: &str) -> Option<Classification> {
    let c = match op {
        "management"
        | "management_info"
        | "management_prometheus"
        | "management_metrics"
        | "management_env"
        | "management_flamegraph"
        | "management_loggers_get" => {
            Classification::audited(Execute, ObjectClass::ApplicationActivity)
        }
        "management_loggers_set" => {
            Classification::audited(Update, ObjectClass::ApplicationActivity)
        }
        "management_loggers_reset" => {
            Classification::audited(Delete, ObjectClass::ApplicationActivity)
        }
        _ => return None,
    };
    Some(c)
}

/// Look up the **explicit** classification for an operation id, or `None`
/// when the id is not in the table.
///
/// This is the raw table probe used by the coverage guard; the request path
/// uses [`classify`], which applies the fail-closed [`DEFAULT`] to a `None`.
#[must_use]
#[expect(
    clippy::match_same_arms,
    reason = "the arms are grouped by audited resource, so naming every operation \
              explicitly is the point — merging equal arms would hide which \
              operations belong to which DICOM EventID"
)]
pub fn lookup(op: &str) -> Option<Classification> {
    let c = match op {
        // ── SYSTEM (the OPTIONS-and-Conformance manifest) ────────────────────
        // A conformance probe touches no clinical resource: audited as an
        // application-activity Execute (the same class the fail-closed
        // default uses, made EXPLICIT so the completeness gate covers it).
        "options" => Classification::audited(Execute, ObjectClass::ApplicationActivity),

        // ── EHR / Patient Record (DICOM EventID 110110) ──────────────────────
        "ehr_create" | "ehr_create_with_id" => Classification::audited(Create, Ehr),
        "ehr_get_by_subject" | "ehr_get_by_id" => Classification::audited(Read, Ehr),

        // EHR_STATUS is a facet of the EHR / Patient Record.
        "ehr_status_get_by_version_id"
        | "ehr_status_get_at_time"
        | "versioned_ehr_status_get"
        | "versioned_ehr_status_revision_history"
        | "versioned_ehr_status_version_get_at_time"
        | "versioned_ehr_status_version_get_by_id" => Classification::audited(Read, Ehr),
        "ehr_status_update" => Classification::audited(Update, Ehr),

        // ── COMPOSITION ──────────────────────────────────────────────────────
        "composition_create" => Classification::audited(Create, Composition),
        "composition_update" => Classification::audited(Update, Composition),
        "composition_delete" => Classification::audited(Delete, Composition),
        "composition_get"
        | "versioned_composition_get"
        | "versioned_composition_revision_history"
        | "versioned_composition_version_get_at_time"
        | "versioned_composition_version_get_by_id" => Classification::audited(Read, Composition),

        // ── DIRECTORY (FOLDER) ───────────────────────────────────────────────
        "directory_create" => Classification::audited(Create, Directory),
        "directory_update" => Classification::audited(Update, Directory),
        "directory_delete" => Classification::audited(Delete, Directory),
        "directory_get_at_time" | "directory_get_by_version_id" => {
            Classification::audited(Read, Directory)
        }

        // The released OAS reuses these CONTRIBUTION `operationId`s in both
        // bundles, and one classification is correct for both: RM common
        // master06 change control governs every VERSIONED_OBJECT, demographic
        // content included. The `adjudicated_shared_ids` gate below fails on any
        // new cross-group duplicate.
        "contribution_create" => Classification::audited(Create, Contribution),
        "contribution_get" => Classification::audited(Read, Contribution),

        // NOTE: RM ehr `master04-ehr_package.adoc` §Tags puts ITEM_TAGs outside
        // change control, so auditing them at all is our own design/extension:
        // each is audited under its parent's DICOM class.
        "ehr_tags_get" | "ehr_status_tags_get" => Classification::audited(Read, Ehr),
        "ehr_status_tags_update" => Classification::audited(Update, Ehr),
        "ehr_status_tags_delete" => Classification::audited(Delete, Ehr),
        "composition_tags_get" => Classification::audited(Read, Composition),
        "composition_tags_update" => Classification::audited(Update, Composition),
        "composition_tags_delete" => Classification::audited(Delete, Composition),

        // ── DEFINITION: stored queries ───────────────────────────────────────
        "definition_query_list" | "definition_query_version_get" => {
            Classification::audited(Read, Query)
        }
        // The ".yaml" suffix is the generated operationId verbatim.
        "definition_query_store.yaml" | "definition_query_version_store.yaml" => {
            Classification::audited(Create, Query)
        }

        // ── QUERY execution (ad-hoc AQL + stored-query invocation) ───────────
        "query_execute_adhoc_query"
        | "query_execute_adhoc_query_body"
        | "query_execute_stored_query"
        | "query_execute_stored_query_body"
        | "query_execute_stored_query_version"
        | "query_execute_stored_query_version_body"
        // The cohort extension executes AQL like the six released operations;
        // the service layer records the boundary crossing separately.
        | "query_execute_cohort" => Classification::audited(Execute, Query),

        // ── DEFINITION: operational templates (OPT provisioning) ─────────────
        "definition_template_adl1.4_upload" | "definition_template_adl2_upload" => {
            Classification::audited(Create, Template)
        }
        "definition_template_adl1.4_list"
        | "definition_template_adl1.4_get"
        | "definition_template_adl1.4_example_get"
        | "definition_template_adl2_list"
        | "definition_template_adl2_get"
        | "definition_template_adl2_example_get"
        | "definition_template_adl2_version_get" => Classification::audited(Read, Template),

        // ── ADMIN (physical deletion of EHRs; bulk delete-all) ───────────────
        "admin_ehr_delete" | "admin_ehr_delete_all" => Classification::audited(Delete, Ehr),

        // ── DEMOGRAPHIC (person-identifiable; audited under the Patient-Record
        //    EventID family per our ATNA design) ───────────────────────────────
        "agent_create"
        | "group_create"
        | "organisation_create"
        | "person_create"
        | "role_create" => Classification::audited(Create, Demographic),
        "agent_get"
        | "group_get"
        | "organisation_get"
        | "person_get"
        | "role_get"
        | "versioned_party_get"
        | "versioned_party_revision_history"
        | "versioned_party_version_get_at_time"
        | "versioned_party_version_get_by_id"
        | "demographic_tags_get"
        | "agent_tags_get"
        | "group_tags_get"
        | "organisation_tags_get"
        | "person_tags_get"
        | "role_tags_get" => Classification::audited(Read, Demographic),
        "agent_update"
        | "group_update"
        | "organisation_update"
        | "person_update"
        | "role_update"
        | "agent_tags_update"
        | "group_tags_update"
        | "organisation_tags_update"
        | "person_tags_update"
        | "role_tags_update" => Classification::audited(Update, Demographic),
        "agent_delete"
        | "group_delete"
        | "organisation_delete"
        | "person_delete"
        | "role_delete"
        | "agent_tags_delete"
        | "group_tags_delete"
        | "organisation_tags_delete"
        | "person_tags_delete"
        | "role_tags_delete" => Classification::audited(Delete, Demographic),

        // The management surface's ids live in their own table (#3244).
        _ => return management_lookup(op),
    };
    Some(c)
}

/// Classifies an operation id for the request path: its explicit [`lookup`]
/// entry, or the fail-closed [`DEFAULT`] when the id is unrecognised.
///
/// Never yields "unknown": an unrecognised operation is audited under the
/// generic class rather than dropped.
#[must_use]
pub fn classify(op: &str) -> Classification {
    lookup(op).unwrap_or(DEFAULT)
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
    /// CONTRIBUTION block in [`lookup`]). A NEW cross-group duplicate fails
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
             (add them to app/ferroehr-rest/src/system_log/classify.rs): {missing:?}"
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
        assert_eq!(audit_for("person_create"), Some((Create, Demographic)));
        assert_eq!(audit_for("role_tags_delete"), Some((Delete, Demographic)));
        assert_eq!(audit_for("ehr_status_tags_get"), Some((Read, Ehr)));
    }
}
