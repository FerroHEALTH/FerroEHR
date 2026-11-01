// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Unit tests of the parent module.

use crate::service::error::ServiceError;
use crate::service::refusal::Invalid;
use serde_json::json;

use super::*;

/// A minimal spec-shaped `ORIGINAL_VERSION` wire value for the import parser
/// — carrying both mandatory `VERSION` attributes (`contribution` 1..1,
/// `commit_audit` 1..1; RM common `version.adoc` §Attributes).
fn original_version(type_field: Option<&str>) -> Value {
    let mut ov = json!({
        "uid": { "_type": "OBJECT_VERSION_ID",
                 "value": "018f4a5e-9df1-7d1e-8b6f-2b8c00000001::sysA.example.org::1" },
        "contribution": { "_type": "OBJECT_REF", "namespace": "local",
                          "type": "CONTRIBUTION",
                          "id": { "_type": "HIER_OBJECT_ID",
                                  "value": "3d2c1b0a-0000-4000-8000-000000000abc" } },
        "commit_audit": {
            "_type": "AUDIT_DETAILS",
            "system_id": "sysA.example.org",
            "time_committed": { "_type": "DV_DATE_TIME", "value": "2026-07-11T10:00:00Z" },
            "change_type": { "_type": "DV_CODED_TEXT", "value": "creation",
                             "defining_code": { "_type": "CODE_PHRASE", "code_string": "249",
                                                "terminology_id": { "_type": "TERMINOLOGY_ID",
                                                                    "value": "openehr" } } },
            "committer": { "_type": "PARTY_IDENTIFIED", "name": "Dr A" }
        },
        "lifecycle_state": { "_type": "DV_CODED_TEXT", "value": "complete",
                             "defining_code": { "_type": "CODE_PHRASE", "code_string": "532",
                                                "terminology_id": { "_type": "TERMINOLOGY_ID",
                                                                    "value": "openehr" } } },
        "data": { "_type": "EHR_STATUS" }
    });
    if let Some(t) = type_field {
        ov.as_object_mut().unwrap().insert("_type".into(), json!(t));
    }
    ov
}

/// `X_VERSIONED_OBJECT.versions` members must be `ORIGINAL_VERSION` (RM
/// `ehr_extract` master05); a foreign `_type` — e.g. an already-wrapped
/// `IMPORTED_VERSION` — is rejected, while an explicit or absent
/// `ORIGINAL_VERSION` tag parses. Regression for
/// rm-common-change-control-R20.
#[test]
fn imported_versions_member_type_is_enforced() {
    parse_imported_version(&original_version(None)).expect("absent _type defaults");
    parse_imported_version(&original_version(Some("ORIGINAL_VERSION")))
        .expect("explicit ORIGINAL_VERSION");
    for foreign in ["IMPORTED_VERSION", "VERSION", "ORIGINAL_VERSION2"] {
        let err = parse_imported_version(&original_version(Some(foreign)))
            .expect_err("foreign _type in versions[] must be rejected");
        assert!(
            err.message().contains("ORIGINAL_VERSION") && err.message().contains(foreign),
            "error should name the expected and offending types, got: {}",
            err.message()
        );
    }
}

/// The received original's OWN `contribution` and `commit_audit` are the
/// FOREIGN act, kept for the wrapped `ORIGINAL_VERSION` — RM common
/// master06 §Committal and Audits: "the knowledge of the original
/// Contribution and committal are retained inside the wrapped
/// `ORIGINAL_VERSION` instance". Regression for #1679, whose defect was
/// that the received `contribution` was dropped entirely and the foreign
/// `commit_audit` was written as the version row's own.
#[test]
fn wrapped_original_keeps_its_foreign_contribution_and_audit() {
    let (_, version) = parse_imported_version(&original_version(None)).expect("the fixture parses");
    assert_eq!(
        version
            .contribution
            .pointer("/id/value")
            .and_then(Value::as_str),
        Some("3d2c1b0a-0000-4000-8000-000000000abc"),
        "the received contribution reference must be preserved verbatim"
    );
    assert_eq!(
        version
            .commit_audit
            .pointer("/time_committed/value")
            .and_then(Value::as_str),
        Some("2026-07-11T10:00:00Z"),
        "the source commit instant must ride the wrapped original, not the row"
    );
    assert_eq!(
        version
            .commit_audit
            .get("system_id")
            .and_then(Value::as_str),
        Some("sysA.example.org"),
    );
}

/// `VERSION.contribution` is 1..1 (RM common `version.adoc` §Attributes);
/// an `ORIGINAL_VERSION` arriving without one cannot be wrapped without
/// losing the original Contribution master06 §Committal and Audits says
/// must be retained, so it is refused.
#[test]
fn a_received_original_without_a_contribution_is_refused() {
    let mut ov = original_version(None);
    ov.as_object_mut().expect("object").remove("contribution");
    let err = parse_imported_version(&ov).expect_err("contribution is mandatory");
    assert!(
        err.message().contains("contribution"),
        "error should name the missing attribute, got: {}",
        err.message()
    );
}

/// `AUDIT_DETAILS.Change_type_valid`: the commit audit's `change_type` is
/// coded from the openEHR `audit change type` group
/// (`UML/classes/org.openehr.rm.common.audit_details.adoc` §Invariants), so
/// a foreign code is refused rather than stored verbatim.
#[test]
fn a_received_commit_audit_with_an_unknown_change_type_is_refused() {
    let mut ov = original_version(None);
    *ov.pointer_mut("/commit_audit/change_type/defining_code/code_string")
        .expect("code_string") = json!("999999");
    let err = parse_imported_version(&ov).expect_err("change_type must be an openEHR code");
    assert!(
        err.message().contains("audit_change_type"),
        "error should name the violated invariant, got: {}",
        err.message()
    );
}

/// A malformed `other_input_version_uids` entry refuses the version, the
/// rule the CONTRIBUTION path applies (RM common `original_version`:
/// `Other_input_version_uids_valid`; master06 §Version Merging), and the
/// refusal names the entry; a valid list is kept in order.
#[test]
fn a_malformed_merge_input_is_refused_and_a_valid_one_kept() {
    let valid = "018f4a5e-9df1-7d1e-8b6f-2b8c00000002::sysB.example.org::3";
    let mut ov = original_version(None);
    ov.as_object_mut().expect("object").insert(
        "other_input_version_uids".into(),
        json!([{ "_type": "OBJECT_VERSION_ID", "value": valid }]),
    );
    let (_, version) = parse_imported_version(&ov).expect("a valid merge input parses");
    assert_eq!(version.other_input_version_uids, vec![valid.to_owned()]);

    ov.as_object_mut().expect("object").insert(
        "other_input_version_uids".into(),
        json!([
            { "_type": "OBJECT_VERSION_ID", "value": valid },
            { "_type": "OBJECT_VERSION_ID", "value": "not-a-version-id" }
        ]),
    );
    let err = parse_imported_version(&ov).expect_err("a malformed merge input is refused");
    assert!(matches!(
        &err,
        ServiceError::Unprocessable {
            kind: Invalid::Content,
            ..
        }
    ));
    assert!(
        err.message().contains("other_input_version_uids [1]")
            && err.message().contains("not-a-version-id"),
        "the refusal names the defective entry, got: {}",
        err.message()
    );
}

/// `ORIGINAL_VERSION.lifecycle_state` is 1..1 (RM common
/// `org.openehr.rm.common.original_version.adoc`), so a received original
/// without one is refused, never stored as `532|complete|`.
#[test]
fn a_received_original_without_a_lifecycle_state_is_refused() {
    let mut ov = original_version(None);
    ov.as_object_mut()
        .expect("object")
        .remove("lifecycle_state");
    let err = parse_imported_version(&ov).expect_err("lifecycle_state is mandatory");
    assert!(
        err.message().contains("lifecycle_state"),
        "error should name the missing attribute, got: {}",
        err.message()
    );
}
