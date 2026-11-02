// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `FhirTerminologyProvider` wire-contract tests,
//! driven by `wiremock` — a hermetic FHIR R4B terminology server: canned
//! `$validate-code`/`$expand`/`$subsumes`/`$lookup` responses + fault injection
//! (timeout, `5xx`, malformed). No network.
//!
//! Every operation arrives as a `POST` of a `Parameters` body
//! (`crate::terminology_wire`); the answers are read through the R4B
//! operation contracts of `ferrobridge-term`, so a body that does not fit them
//! is a fault, never a partial read.

use std::time::Duration;

use ferroehr::service::error::ServiceError;
use ferroehr::service::refusal::Missing;
use serde_json::json;
use wiremock::{MockServer, ResponseTemplate};

use crate::fixtures::provider;
use crate::terminology_wire::{Absent, operation, parameter};

/// The reference golden case: `ValueSet/surface`, code `B` = "Buccal".
const SURFACE_VS: &str = "http://hl7.org/fhir/ValueSet/surface";
const SURFACE_SYS: &str = "http://hl7.org/fhir/surface";
/// `ValueSet.expansion.timestamp` is 1..1 in FHIR R4B; every mock
/// `$expand` response carries it.
const EXPANSION_TIMESTAMP: &str = "2026-08-04T00:00:00Z";

#[tokio::test]
async fn validate_code_member_is_accepted() {
    let server = MockServer::start().await;
    operation("/ValueSet/$validate-code")
        .and(parameter("url", SURFACE_VS))
        .and(parameter("code", "B"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "result", "valueBoolean": true},
                {"name": "display", "valueString": "Buccal"}
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    let ok = p
        .value_set_validate(SURFACE_SYS, SURFACE_VS, "B", None)
        .await
        .expect("call");
    assert!(ok, "member code B is valid in ValueSet/surface");
}

#[tokio::test]
async fn validate_code_non_member_is_rejected() {
    let server = MockServer::start().await;
    operation("/ValueSet/$validate-code")
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "result", "valueBoolean": false}]
        })))
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    let ok = p
        .value_set_validate(SURFACE_SYS, SURFACE_VS, "Z", None)
        .await
        .expect("call");
    assert!(!ok, "non-member code Z is not valid");
}

#[tokio::test]
async fn validate_code_unknown_valueset_is_not_found() {
    let server = MockServer::start().await;
    operation("/ValueSet/$validate-code")
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    let err = p
        .value_set_validate(SURFACE_SYS, "http://x/ValueSet/nope", "B", None)
        .await
        .expect_err("404 → not found");
    assert!(
        matches!(&err, ServiceError::NotFound(r) if r.kind == Missing::VersionedObject),
        "got {err:?}"
    );
    // The 4xx body names what the CLIENT asked about, never WHICH configured
    // provider answered (#1819 — the operator-detail adjudication's 4xx arm).
    assert!(
        err.message().contains("http://x/ValueSet/nope") && !err.message().contains("test"),
        "the body names the asked-about id and not the provider: {err:?}"
    );
}

#[tokio::test]
async fn get_value_set_maps_expansion_to_extract() {
    let server = MockServer::start().await;
    operation("/ValueSet/$expand")
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "ValueSet",
            "status": "active",
            "expansion": {"timestamp": EXPANSION_TIMESTAMP, "contains": [
                {"system": SURFACE_SYS, "code": "B", "display": "Buccal"},
                {"system": SURFACE_SYS, "code": "O", "display": "Occlusal"}
            ]}
        })))
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    let extract = p
        .get_value_set(SURFACE_SYS, SURFACE_VS)
        .await
        .expect("call");
    let terms = extract.terms.expect("terms");
    assert_eq!(terms.len(), 2);
    assert!(terms.contains_key("B"));
    assert!(terms.contains_key("O"));
}

#[tokio::test]
async fn subsumes_true_only_on_strict_subsumption() {
    // outcome=subsumes → true.
    let server = MockServer::start().await;
    operation("/CodeSystem/$subsumes")
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "outcome", "valueCode": "subsumes"}]
        })))
        .mount(&server)
        .await;
    let p = provider(&server.uri());
    assert!(p.subsumes("sys", "parent", "child").await.expect("call"));

    // outcome=equivalent → false (strict subsumption excludes equivalence).
    let server2 = MockServer::start().await;
    operation("/CodeSystem/$subsumes")
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "outcome", "valueCode": "equivalent"}]
        })))
        .mount(&server2)
        .await;
    let p2 = provider(&server2.uri());
    assert!(!p2.subsumes("sys", "a", "a").await.expect("call"));
}

#[tokio::test]
async fn lookup_drives_has_term_and_get_term() {
    let server = MockServer::start().await;
    operation("/CodeSystem/$lookup")
        .and(parameter("code", "B"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "name", "valueString": "surface"},
                {"name": "display", "valueString": "Buccal"}
            ]
        })))
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    assert!(p.has_term(SURFACE_SYS, "B", None).await.expect("has_term"));

    let extract = p
        .get_term(SURFACE_SYS, "B", None, None)
        .await
        .expect("get_term");
    let terms = extract.terms.expect("terms");
    let entry = terms.get("B").expect("B");
    match entry {
        ferroehr::service::terminology::types::TermEntry::Defined(d) => {
            assert_eq!(d.text, "Buccal");
        }
        ferroehr::service::terminology::types::TermEntry::Bare(_) => {
            panic!("expected a defined term");
        }
    }
}

#[tokio::test]
async fn has_term_false_when_lookup_404() {
    let server = MockServer::start().await;
    operation("/CodeSystem/$lookup")
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    assert!(
        !p.has_term(SURFACE_SYS, "nope", None)
            .await
            .expect("has_term")
    );
}

// ─── fault injection ─────────────────────────────────────────────────────────

#[tokio::test]
async fn server_5xx_is_an_exception() {
    let server = MockServer::start().await;
    operation("/ValueSet/$validate-code")
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    let err = p
        .value_set_validate(SURFACE_SYS, SURFACE_VS, "B", None)
        .await
        .expect_err("5xx → exception");
    assert!(matches!(&err, ServiceError::Fault(_)), "got {err:?}");
    // The OPERATOR detail stays off the wire: a 500 body names neither the
    // configured provider, nor its URL, nor the upstream status — a tenant's
    // clients cannot read the deployment's terminology configuration out of a
    // failure. (The operator gets all of it on the trace record.)
    assert!(
        !err.message().contains("test"),
        "the configured provider name must not reach the body, got {err:?}"
    );
    assert!(
        !err.message().contains(&server.uri()) && !err.message().contains("503"),
        "the upstream URL/status must not reach the body, got {err:?}"
    );
}

#[tokio::test]
async fn malformed_body_is_an_exception() {
    let server = MockServer::start().await;
    operation("/ValueSet/$validate-code")
        .respond_with(ResponseTemplate::new(200).set_body_string("this is not FHIR json"))
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    let err = p
        .value_set_validate(SURFACE_SYS, SURFACE_VS, "B", None)
        .await
        .expect_err("malformed → exception");
    assert!(matches!(&err, ServiceError::Fault(_)), "got {err:?}");
}

#[tokio::test]
async fn missing_result_field_is_an_exception() {
    let server = MockServer::start().await;
    operation("/ValueSet/$validate-code")
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "message", "valueString": "no result here"}]
        })))
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    let err = p
        .value_set_validate(SURFACE_SYS, SURFACE_VS, "B", None)
        .await
        .expect_err("no result → exception");
    assert!(matches!(&err, ServiceError::Fault(_)), "got {err:?}");
}

#[tokio::test]
async fn timeout_is_an_exception() {
    let server = MockServer::start().await;
    operation("/ValueSet/$validate-code")
        // Delay well past the provider's 800ms request timeout.
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(3))
                .set_body_json(json!({
                    "resourceType": "Parameters",
                    "parameter": [{"name": "result", "valueBoolean": true}]
                })),
        )
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    let err = p
        .value_set_validate(SURFACE_SYS, SURFACE_VS, "B", None)
        .await
        .expect_err("timeout → exception");
    assert!(matches!(&err, ServiceError::Fault(_)), "got {err:?}");
}

// ─── the request side ────────────────────────────────────────────────────────

/// The `tx-issue-type` code system the terminology-service specifications use
/// to say why an operation failed
/// (<https://build.fhir.org/ig/HL7/fhir-tools-ig/CodeSystem-tx-issue-type.html>).
const TX_ISSUE_TYPE: &str = "http://hl7.org/fhir/tools/CodeSystem/tx-issue-type";

/// A `$lookup` answer carrying the two out parameters FHIR R4B
/// `OperationDefinition-CodeSystem-lookup` declares 1..1, `name` and `display`.
fn lookup_found(display: &str) -> serde_json::Value {
    json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "name", "valueString": "surface"},
            {"name": "display", "valueString": display}
        ]
    })
}

/// The `at_date` reaches `$lookup` as its FHIR `date` input
/// (<https://hl7.org/fhir/R4B/codesystem-operation-lookup.html>).
#[tokio::test]
async fn at_date_is_sent_as_the_lookup_date() {
    let server = MockServer::start().await;
    operation("/CodeSystem/$lookup")
        .and(parameter("system", SURFACE_SYS))
        .and(parameter("code", "B"))
        .and(parameter("date", "2026-01-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(lookup_found("Buccal")))
        .expect(1)
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    assert!(
        p.has_term(SURFACE_SYS, "B", Some("2026-01-01".to_owned()))
            .await
            .expect("has_term")
    );
}

/// Without an `at_date` the request names no `date`, so the server answers for
/// the current state of the code system.
#[tokio::test]
async fn no_date_is_sent_without_at_date() {
    let server = MockServer::start().await;
    operation("/CodeSystem/$lookup")
        .and(Absent("date"))
        .respond_with(ResponseTemplate::new(200).set_body_json(lookup_found("Buccal")))
        .expect(1)
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    assert!(p.has_term(SURFACE_SYS, "B", None).await.expect("has_term"));
}

/// The `at_date` reaches `$validate-code` as its FHIR `date` input
/// (<https://hl7.org/fhir/R4B/valueset-operation-validate-code.html>).
#[tokio::test]
async fn at_date_is_sent_as_the_validate_code_date() {
    let server = MockServer::start().await;
    operation("/ValueSet/$validate-code")
        .and(parameter("url", SURFACE_VS))
        .and(parameter("system", SURFACE_SYS))
        .and(parameter("code", "B"))
        .and(parameter("date", "2026-01-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "result", "valueBoolean": true}]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    assert!(
        p.value_set_validate(SURFACE_SYS, SURFACE_VS, "B", Some("2026-01-01".to_owned()))
            .await
            .expect("call")
    );
}

/// `$subsumes` carries the two codes as `codeA` and `codeB`
/// (<https://hl7.org/fhir/R4B/codesystem-operation-subsumes.html>).
#[tokio::test]
async fn subsumes_sends_code_a_and_code_b() {
    let server = MockServer::start().await;
    operation("/CodeSystem/$subsumes")
        .and(parameter("system", "sys"))
        .and(parameter("codeA", "parent"))
        .and(parameter("codeB", "child"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "outcome", "valueCode": "subsumes"}]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    assert!(p.subsumes("sys", "parent", "child").await.expect("call"));
}

/// A `5xx` is answered once: the request timeout bounds the whole call on the
/// commit path, so the client does not retry.
#[tokio::test]
async fn a_server_failure_is_not_retried() {
    let server = MockServer::start().await;
    operation("/ValueSet/$validate-code")
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    let err = p
        .value_set_validate(SURFACE_SYS, SURFACE_VS, "B", None)
        .await
        .expect_err("5xx → exception");
    assert!(matches!(&err, ServiceError::Fault(_)), "got {err:?}");
}

// ─── reading the answer ──────────────────────────────────────────────────────

/// A `404` whose `OperationOutcome` carries no `tx-issue-type` coding still
/// says the code is unknown.
#[tokio::test]
async fn a_404_outcome_without_details_is_an_unknown_term() {
    let server = MockServer::start().await;
    operation("/CodeSystem/$lookup")
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "resourceType": "OperationOutcome",
            "issue": [{"severity": "error", "code": "not-found", "diagnostics": "Unknown code"}]
        })))
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    assert!(
        !p.has_term(SURFACE_SYS, "nope", None)
            .await
            .expect("has_term")
    );
}

/// Only `issue.details.coding` entries of the `tx-issue-type` system classify a
/// refusal; the same code under another system leaves a `400` a fault.
#[tokio::test]
async fn a_tx_code_under_another_system_does_not_classify() {
    let server = MockServer::start().await;
    operation("/CodeSystem/$lookup")
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "error",
                "code": "code-invalid",
                "details": {"coding": [{"system": "http://example.org/other", "code": "invalid-code"}]}
            }]
        })))
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    let err = p
        .has_term(SURFACE_SYS, "B", None)
        .await
        .expect_err("an unclassified 400 is a fault");
    assert!(matches!(&err, ServiceError::Fault(_)), "got {err:?}");
}

/// A `tx-issue-type` `invalid-code` refusal is an unknown term, whatever the
/// status (`FerroTERM` answers it with `400`).
#[tokio::test]
async fn an_invalid_code_refusal_is_an_unknown_term() {
    let server = MockServer::start().await;
    operation("/CodeSystem/$lookup")
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "error",
                "code": "code-invalid",
                "details": {"coding": [{"system": TX_ISSUE_TYPE, "code": "invalid-code"}]}
            }]
        })))
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    let err = p
        .get_term(SURFACE_SYS, "nope", None, None)
        .await
        .expect_err("an unknown term");
    assert!(
        matches!(&err, ServiceError::NotFound(r) if r.kind == Missing::VersionedObject),
        "got {err:?}"
    );
}

/// A `410 Gone` value set is unknown.
#[tokio::test]
async fn a_gone_value_set_is_unknown() {
    let server = MockServer::start().await;
    operation("/ValueSet/$expand")
        .respond_with(ResponseTemplate::new(410))
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    assert!(
        !p.has_value_set(SURFACE_SYS, SURFACE_VS)
            .await
            .expect("has_value_set")
    );
}

/// The nested `contains` tree of an `$expand` answer survives as the extract's
/// membership, an abstract grouper without a code included.
#[tokio::test]
async fn get_value_set_keeps_the_nested_members() {
    let server = MockServer::start().await;
    operation("/ValueSet/$expand")
        .and(parameter("url", SURFACE_VS))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "ValueSet",
            "status": "active",
            "expansion": {"timestamp": EXPANSION_TIMESTAMP, "contains": [
                {"system": SURFACE_SYS, "code": "L", "display": "Lingual", "contains": [
                    {"system": SURFACE_SYS, "code": "LD", "display": "Lingual distal"}
                ]},
                {"abstract": true, "display": "Occlusal group", "contains": [
                    {"system": SURFACE_SYS, "code": "O"}
                ]}
            ]}
        })))
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    let extract = p
        .get_value_set(SURFACE_SYS, SURFACE_VS)
        .await
        .expect("call");
    let terms = extract.terms.expect("terms");
    assert_eq!(
        terms.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["L", "LD", "O"]
    );
    let relationships = extract.relationships.expect("relationships");
    assert_eq!(relationships.len(), 1);
    assert_eq!(relationships[0].origin_code, "L");
}

/// A `$validate-code` answered with another resource is the wrong answer, a
/// fault, never "no result".
#[tokio::test]
async fn a_validate_code_answered_with_another_resource_is_a_fault() {
    let server = MockServer::start().await;
    operation("/ValueSet/$validate-code")
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"resourceType": "ValueSet", "status": "active"})),
        )
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    let err = p
        .value_set_validate(SURFACE_SYS, SURFACE_VS, "B", None)
        .await
        .expect_err("a ValueSet is not a $validate-code answer");
    assert!(matches!(&err, ServiceError::Fault(_)), "got {err:?}");
}

/// An `$expand` answered with a `Parameters` resource is a fault, never an
/// empty expansion.
#[tokio::test]
async fn an_expand_answered_with_another_resource_is_a_fault() {
    let server = MockServer::start().await;
    operation("/ValueSet/$expand")
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"resourceType": "Parameters"})),
        )
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    let err = p
        .get_value_set(SURFACE_SYS, SURFACE_VS)
        .await
        .expect_err("a Parameters is not an expansion");
    assert!(matches!(&err, ServiceError::Fault(_)), "got {err:?}");
}

/// A `parameter` entry without its mandatory `name` (FHIR R4B `Parameters`,
/// `parameter.name` 1..1) is a malformed answer.
#[tokio::test]
async fn a_parameter_without_a_name_is_a_fault() {
    let server = MockServer::start().await;
    operation("/ValueSet/$validate-code")
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [{"valueBoolean": true}]
        })))
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    let err = p
        .value_set_validate(SURFACE_SYS, SURFACE_VS, "B", None)
        .await
        .expect_err("a nameless parameter is malformed");
    assert!(matches!(&err, ServiceError::Fault(_)), "got {err:?}");
}

/// A `ValueSet` the R4B model refuses (`ValueSet.status` 1..1,
/// `ValueSet.expansion.timestamp` 1..1) is a fault, never a partial read.
#[tokio::test]
async fn a_value_set_the_r4b_model_refuses_is_a_fault() {
    for body in [
        json!({"resourceType": "ValueSet", "expansion": {"timestamp": EXPANSION_TIMESTAMP}}),
        json!({"resourceType": "ValueSet", "status": "active", "expansion": {"contains": []}}),
    ] {
        let server = MockServer::start().await;
        operation("/ValueSet/$expand")
            .respond_with(ResponseTemplate::new(200).set_body_json(body.clone()))
            .mount(&server)
            .await;

        let p = provider(&server.uri());
        let err = p
            .get_value_set(SURFACE_SYS, SURFACE_VS)
            .await
            .expect_err("a ValueSet the R4B model refuses");
        assert!(
            matches!(&err, ServiceError::Fault(_)),
            "{body}: got {err:?}"
        );
    }
}

/// `$expand` returns "the result of the expansion" as its one `return` out
/// parameter (FHIR R4B `OperationDefinition-ValueSet-expand`), so a `ValueSet`
/// without an `expansion` is a fault, never a value set without members.
#[tokio::test]
async fn an_expand_answer_without_an_expansion_is_a_fault() {
    let server = MockServer::start().await;
    operation("/ValueSet/$expand")
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"resourceType": "ValueSet", "status": "active"})),
        )
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    let err = p
        .get_value_set(SURFACE_SYS, SURFACE_VS)
        .await
        .expect_err("a ValueSet without an expansion");
    assert!(matches!(&err, ServiceError::Fault(_)), "got {err:?}");
}

/// FHIR R4B `OperationDefinition-CodeSystem-lookup` declares `display` 1..1,
/// so a resolving answer without it is a fault, never a term whose text is
/// made up.
#[tokio::test]
async fn a_lookup_answer_without_a_display_is_a_fault() {
    let server = MockServer::start().await;
    operation("/CodeSystem/$lookup")
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "name", "valueString": "surface"}]
        })))
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    let err = p
        .get_term(SURFACE_SYS, "B", None, None)
        .await
        .expect_err("a lookup without its display");
    assert!(matches!(&err, ServiceError::Fault(_)), "got {err:?}");
}

/// The `$subsumes` `outcome` is bound `required` to `ConceptSubsumptionOutcome`
/// (FHIR R4B `OperationDefinition-CodeSystem-subsumes`), so a code outside it
/// is a fault, never "not subsumed".
#[tokio::test]
async fn a_subsumes_outcome_outside_its_value_set_is_a_fault() {
    let server = MockServer::start().await;
    operation("/CodeSystem/$subsumes")
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "outcome", "valueCode": "related"}]
        })))
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    let err = p
        .subsumes("sys", "a", "b")
        .await
        .expect_err("an outcome outside the value set");
    assert!(matches!(&err, ServiceError::Fault(_)), "got {err:?}");
}
