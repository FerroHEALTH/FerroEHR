// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The typed FHIR R4B terminology decoders
//! (`ferroehr_ext::fhir::terminology`): what the platform's terminology
//! provider gets back from a server's `Parameters` or `ValueSet` body, and
//! what it refuses. The wire, the routing and the SM error mapping are the
//! platform's and are exercised by `app/ferroehr/tests/it/terminology_*.rs`
//! against wiremock; this module drives the decoders directly with bodies,
//! valid and malformed, that those journeys never send.
//!
//! No openEHR spec governs FHIR resource representation — our own
//! design/extension; the shapes are HL7 FHIR R4B
//! (<https://hl7.org/fhir/R4B/parameters.html>,
//! <https://hl7.org/fhir/R4B/valueset.html>).

use ferroehr_ext::fhir::terminology::{
    ExpansionMember, TerminologyDecodeError, TranslateMatch, decode_expansion, decode_parameters,
};
use serde_json::json;

fn bytes(value: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("fixture serializes")
}

#[test]
fn parameters_scalars_are_read_by_name_and_kind() {
    let body = bytes(&json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "result", "valueBoolean": true},
            {"name": "outcome", "valueCode": "subsumes"},
            {"name": "display", "valueString": "Buccal"},
            {"name": "message", "valueString": "matched"},
            // A kind the view does not carry is ignored, never an error.
            {"name": "version", "valueInteger": 3}
        ]
    }));
    let view = decode_parameters(&body).expect("a valid Parameters body decodes");
    assert_eq!(view.booleans.get("result"), Some(&true));
    assert_eq!(
        view.codes.get("outcome").map(String::as_str),
        Some("subsumes")
    );
    assert_eq!(
        view.strings.get("display").map(String::as_str),
        Some("Buccal")
    );
    assert_eq!(
        view.strings.get("message").map(String::as_str),
        Some("matched")
    );
    assert!(view.matches.is_empty());
    assert_eq!(
        view.booleans.len() + view.codes.len() + view.strings.len(),
        4
    );
}

#[test]
fn an_empty_parameters_body_is_an_empty_view_not_a_refusal() {
    let body = bytes(&json!({"resourceType": "Parameters"}));
    let view = decode_parameters(&body).expect("a Parameters without parameters is valid");
    assert!(view.booleans.is_empty() && view.codes.is_empty() && view.strings.is_empty());
    assert!(view.matches.is_empty());
}

#[test]
fn translate_matches_keep_their_order_and_their_concepts() {
    let body = bytes(&json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "result", "valueBoolean": true},
            {"name": "match", "part": [
                {"name": "equivalence", "valueCode": "equivalent"},
                {"name": "concept", "valueCoding": {
                    "system": "http://snomed.info/sct", "code": "271649006",
                    "display": "Systolic blood pressure"
                }}
            ]},
            {"name": "match", "part": [
                {"name": "equivalence", "valueCode": "wider"},
                {"name": "concept", "valueCoding": {"system": "http://loinc.org", "code": "8480-6"}}
            ]},
            // A match with no recognisable parts is still a match, all fields absent.
            {"name": "match", "part": [{"name": "source", "valueUri": "http://example.org/map"}]}
        ]
    }));
    let view = decode_parameters(&body).expect("a $translate response decodes");
    assert_eq!(view.matches.len(), 3);
    assert_eq!(view.matches[0].equivalence.as_deref(), Some("equivalent"));
    assert_eq!(
        view.matches[0].system.as_deref(),
        Some("http://snomed.info/sct")
    );
    assert_eq!(view.matches[0].code.as_deref(), Some("271649006"));
    assert_eq!(
        view.matches[0].display.as_deref(),
        Some("Systolic blood pressure")
    );
    assert_eq!(view.matches[1].equivalence.as_deref(), Some("wider"));
    assert_eq!(view.matches[1].code.as_deref(), Some("8480-6"));
    assert_eq!(view.matches[1].display, None);
    assert_eq!(view.matches[2], TranslateMatch::default());
    // The `result` beside the matches is still read.
    assert_eq!(view.booleans.get("result"), Some(&true));
}

/// A `Parameters` has no mandatory member, so without the `resourceType`
/// check any JSON object would read as an empty view — a `$validate-code`
/// answered with a `ValueSet` (or an `OperationOutcome`) would look like
/// "no result" instead of the wrong answer it is.
#[test]
fn a_body_of_another_resource_type_is_refused_by_name() {
    for (body, found) in [
        (
            bytes(&json!({"resourceType": "ValueSet", "status": "active"})),
            "ValueSet",
        ),
        (
            bytes(&json!({"resourceType": "OperationOutcome", "issue": []})),
            "OperationOutcome",
        ),
        (bytes(&json!({"parameter": []})), "(none)"),
    ] {
        let err = decode_parameters(&body).expect_err("another resource is refused");
        match &err {
            TerminologyDecodeError::WrongResource {
                expected,
                found: got,
            } => {
                assert_eq!(*expected, "Parameters");
                assert_eq!(got, found);
            }
            TerminologyDecodeError::Malformed(other) => {
                panic!("expected a wrong-resource refusal, got a malformed-body one: {other}")
            }
        }
        assert_eq!(
            err.to_string(),
            format!("unexpected FHIR resource: expected Parameters, got {found}")
        );
    }
}

#[test]
fn a_body_that_is_not_a_parameters_resource_is_refused_not_partially_read() {
    for body in [
        // Not JSON at all.
        b"<Parameters/>".to_vec(),
        // A parameter without its mandatory name.
        bytes(&json!({"resourceType": "Parameters", "parameter": [{"valueBoolean": true}]})),
        b"".to_vec(),
    ] {
        let err = decode_parameters(&body).expect_err("a malformed Parameters body is refused");
        assert!(
            matches!(err, TerminologyDecodeError::Malformed(_)),
            "got {err:?}"
        );
        assert!(
            err.to_string().starts_with("malformed FHIR response: "),
            "{err}"
        );
    }
}

#[test]
fn an_expansion_is_read_with_its_nesting() {
    let body = bytes(&json!({
        "resourceType": "ValueSet",
        "status": "active",
        "expansion": {
            "timestamp": "2026-01-01T00:00:00Z",
            "contains": [
                {"system": "http://example.org/surface", "code": "B", "display": "Buccal"},
                {"system": "http://example.org/surface", "code": "L", "display": "Lingual",
                 "contains": [
                    {"system": "http://example.org/surface", "code": "LD", "display": "Lingual distal"}
                 ]},
                // An abstract grouper carries no code; it still holds its members.
                {"abstract": true, "display": "Occlusal group", "contains": [
                    {"system": "http://example.org/surface", "code": "O"}
                ]}
            ]
        }
    }));
    let members = decode_expansion(&body).expect("a valid ValueSet expansion decodes");
    assert_eq!(members.len(), 3);
    assert_eq!(
        members[0],
        ExpansionMember {
            code: Some("B".to_owned()),
            display: Some("Buccal".to_owned()),
            children: Vec::new(),
        }
    );
    assert_eq!(members[1].children.len(), 1);
    assert_eq!(members[1].children[0].code.as_deref(), Some("LD"));
    assert_eq!(members[2].code, None);
    assert_eq!(members[2].children[0].code.as_deref(), Some("O"));
    assert_eq!(members[2].children[0].display, None);
}

#[test]
fn a_value_set_without_an_expansion_has_no_members() {
    let body = bytes(&json!({"resourceType": "ValueSet", "status": "active"}));
    assert!(decode_expansion(&body).expect("valid").is_empty());
    let body = bytes(&json!({
        "resourceType": "ValueSet", "status": "active",
        "expansion": {"timestamp": "2026-01-01T00:00:00Z"}
    }));
    assert!(decode_expansion(&body).expect("valid").is_empty());
}

#[test]
fn a_value_set_the_r4b_model_rejects_is_refused() {
    for body in [
        // `status` is mandatory on ValueSet.
        bytes(
            &json!({"resourceType": "ValueSet", "expansion": {"timestamp": "2026-01-01T00:00:00Z"}}),
        ),
        // `expansion.timestamp` is mandatory.
        bytes(
            &json!({"resourceType": "ValueSet", "status": "active", "expansion": {"contains": []}}),
        ),
    ] {
        let err = decode_expansion(&body).expect_err("a malformed ValueSet body is refused");
        assert!(
            matches!(err, TerminologyDecodeError::Malformed(_)),
            "got {err:?}"
        );
    }
    // The wrong resource is named, not read as a ValueSet with no expansion.
    let err = decode_expansion(&bytes(&json!({"resourceType": "Parameters"})))
        .expect_err("a Parameters body is not an expansion");
    assert!(
        matches!(
            &err,
            TerminologyDecodeError::WrongResource { expected: "ValueSet", found } if found == "Parameters"
        ),
        "got {err:?}"
    );
}
