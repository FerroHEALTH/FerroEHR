// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `FhirTerminologyProvider` wire-contract tests,
//! driven by `wiremock` — a hermetic FHIR R4B terminology server: canned
//! `$validate-code`/`$expand`/`$subsumes`/`$lookup` responses + fault injection
//! (timeout, `5xx`, malformed). No network.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::time::Duration;

use ferroehr::service::status::CallStatusType;
use ferroehr::service::terminology::config::TerminologyServerConfig;
use ferroehr::service::terminology::fhir::FhirTerminologyProvider;
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Build a client pointing at `base` with a short, test-friendly timeout.
fn provider(base: &str) -> FhirTerminologyProvider {
    let cfg = TerminologyServerConfig {
        enabled: true,
        url: ferroehr::config::secret::SecretUrl::new(base),
        request_timeout_ms: 800,
        ..TerminologyServerConfig::default()
    };
    FhirTerminologyProvider::new(&cfg).expect("build provider")
}

/// The reference golden case: `ValueSet/surface`, code `B` = "Buccal".
const SURFACE_VS: &str = "http://hl7.org/fhir/ValueSet/surface";
const SURFACE_SYS: &str = "http://hl7.org/fhir/surface";
/// `ValueSet.expansion.timestamp` is 1..1 in FHIR R4B; every mock
/// `$expand` response carries it.
const EXPANSION_TIMESTAMP: &str = "2026-08-04T00:00:00Z";

#[tokio::test]
async fn validate_code_member_is_accepted() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ValueSet/$validate-code"))
        .and(query_param("url", SURFACE_VS))
        .and(query_param("code", "B"))
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
    Mock::given(method("GET"))
        .and(path("/ValueSet/$validate-code"))
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
    Mock::given(method("GET"))
        .and(path("/ValueSet/$validate-code"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    let err = p
        .value_set_validate(SURFACE_SYS, "http://x/ValueSet/nope", "B", None)
        .await
        .expect_err("404 → not found");
    assert_eq!(err.status, CallStatusType::VersionedObjectDoesNotExist);
    // The 4xx body names what the CLIENT asked about, never WHICH configured
    // provider answered (#1819 — the operator-detail adjudication's 4xx arm).
    assert!(
        err.message.contains("http://x/ValueSet/nope") && !err.message.contains("test"),
        "the body names the asked-about id and not the provider: {err:?}"
    );
}

#[tokio::test]
async fn get_value_set_maps_expansion_to_extract() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ValueSet/$expand"))
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
    Mock::given(method("GET"))
        .and(path("/CodeSystem/$subsumes"))
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
    Mock::given(method("GET"))
        .and(path("/CodeSystem/$subsumes"))
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
    Mock::given(method("GET"))
        .and(path("/CodeSystem/$lookup"))
        .and(query_param("code", "B"))
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
    Mock::given(method("GET"))
        .and(path("/CodeSystem/$lookup"))
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
    Mock::given(method("GET"))
        .and(path("/ValueSet/$validate-code"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    let err = p
        .value_set_validate(SURFACE_SYS, SURFACE_VS, "B", None)
        .await
        .expect_err("5xx → exception");
    assert_eq!(err.status, CallStatusType::Exception);
    // The OPERATOR detail stays off the wire: a 500 body names neither the
    // configured provider, nor its URL, nor the upstream status — a tenant's
    // clients cannot read the deployment's terminology configuration out of a
    // failure. (The operator gets all of it on the trace record.)
    assert!(
        !err.message.contains("test"),
        "the configured provider name must not reach the body, got {err:?}"
    );
    assert!(
        !err.message.contains(&server.uri()) && !err.message.contains("503"),
        "the upstream URL/status must not reach the body, got {err:?}"
    );
}

#[tokio::test]
async fn malformed_body_is_an_exception() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ValueSet/$validate-code"))
        .respond_with(ResponseTemplate::new(200).set_body_string("this is not FHIR json"))
        .mount(&server)
        .await;

    let p = provider(&server.uri());
    let err = p
        .value_set_validate(SURFACE_SYS, SURFACE_VS, "B", None)
        .await
        .expect_err("malformed → exception");
    assert_eq!(err.status, CallStatusType::Exception);
}

#[tokio::test]
async fn missing_result_field_is_an_exception() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ValueSet/$validate-code"))
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
    assert_eq!(err.status, CallStatusType::Exception);
}

#[tokio::test]
async fn timeout_is_an_exception() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ValueSet/$validate-code"))
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
    assert_eq!(err.status, CallStatusType::Exception);
}
