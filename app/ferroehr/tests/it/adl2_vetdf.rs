// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! ADL2 VETDF validation over the terminology-service resolver seam, end to
//! end (AM ADL2 `master03-archetype_package.adoc` §Validity Rules, VETDF:
//! "each external term used within the archetype definition must exist in the
//! relevant terminology (subject to tool accessibility; codes for inaccessible
//! terminologies should be flagged with a warning indicating that no
//! verification was possible)").
//!
//! The binding target `<http://snomedct.info/id/271649006>` is taken apart into
//! the FHIR pair the server is asked about (`system=http://snomed.info/sct`,
//! `code=271649006`), and the server's `OperationOutcome` decides the outcome:
//! a code system it does not serve leaves the binding unverified (accepted), a
//! code absent from a served system is VETDF (`422`), a resolving lookup is
//! accepted. The terminology backend here is a hermetic `wiremock` FHIR R4B
//! server, so every answer shape a real server can give is pinned, including
//! the bare `404` older servers answer with and the `CodeSystem?url=` probe
//! that resolves it; the same contract against a real `FerroTERM` is
//! `adl2_vetdf_ferroterm`. Real `PostgreSQL` 18 via the shared testkit harness.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::sync::Arc;

use ferroehr::service::FerroEhrService;
use ferroehr::service::status::CallStatusType;
use ferroehr::service::terminology::config::{FhirOperation, FhirProviderConfig, ProviderKind};
use ferroehr::service::terminology::fhir::FhirTerminologyProvider;
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const HRID: &str = "openEHR-EHR-OBSERVATION.vetdf.v1.0.0";
/// The FHIR code system the binding target's URI names (SNOMED CT URI Standard).
const SNOMED_SYSTEM: &str = "http://snomed.info/sct";
/// The concept the binding target's URI names.
const SNOMED_CODE: &str = "271649006";
const TX_ISSUE_TYPE: &str = "http://hl7.org/fhir/tools/CodeSystem/tx-issue-type";

/// A minimal spec-valid ADL2 archetype (the same shape the passing
/// `service_definition` upload test uses) whose root node `id1` carries an
/// external SNOMED CT term binding in the ADL2 spec's own URI form (the
/// `snomedct.info` host its `master07.13` example uses) — the VETDF subject.
fn archetype_with_external_binding() -> String {
    "\
archetype (adl_version=2.0.6; rm_release=1.1.0)
    openEHR-EHR-OBSERVATION.vetdf.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <\"published\">
    details = <
        [\"en\"] = <
            language = <[ISO_639-1::en]>
        >
    >

definition
    OBSERVATION[id1] matches { *}

terminology
    term_definitions = <
        [\"en\"] = <
            [\"id1\"] = <text = <\"Root\"> description = <\"Root.\">>
        >
    >
    term_bindings = <
        [\"SNOMED-CT\"] = <
            [\"id1\"] = <http://snomedct.info/id/271649006>
        >
    >
"
    .to_owned()
}

/// Build a provider pointing at `base` with the response cache off (each test
/// asserts a fresh remote answer).
fn provider(base: &str) -> FhirTerminologyProvider {
    let cfg = FhirProviderConfig {
        kind: ProviderKind::Fhir,
        url: base.to_owned(),
        // VETDF uses `CodeSystem/$lookup` regardless of the membership op.
        operation: FhirOperation::ValidateCode,
        connect_timeout_ms: 500,
        request_timeout_ms: 800,
        oauth2_client: None,
        client_cert_path: None,
        client_key_path: None,
        ca_bundle_path: None,
        cache_ttl_secs: 0,
        cache_capacity: 0,
    };
    FhirTerminologyProvider::new("test", &cfg).expect("build provider")
}

/// An `OperationOutcome` carrying one `tx-issue-type` coding.
fn outcome(issue_type: &str, text: &str) -> serde_json::Value {
    json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "severity": "error",
            "code": "not-found",
            "details": {
                "coding": [{"system": TX_ISSUE_TYPE, "code": issue_type}],
                "text": text
            }
        }]
    })
}

/// The `$lookup` mock for the decomposed binding: `system` is the SNOMED CT
/// code system URI and `code` the bare concept id, never the binding URI or
/// the archetype's `SNOMED-CT` key.
async fn mount_lookup(server: &MockServer, response: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path("/CodeSystem/$lookup"))
        .and(query_param("system", SNOMED_SYSTEM))
        .and(query_param("code", SNOMED_CODE))
        .respond_with(response)
        .mount(server)
        .await;
}

/// The `CodeSystem?url=…&_summary=count` probe an unclassified `404` triggers.
async fn mount_system_probe(server: &MockServer, total: u32) {
    Mock::given(method("GET"))
        .and(path("/CodeSystem"))
        .and(query_param("url", SNOMED_SYSTEM))
        .and(query_param("_summary", "count"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Bundle",
            "type": "searchset",
            "total": total
        })))
        .mount(server)
        .await;
}

async fn service_at(server: &MockServer) -> (testkit::TestDb, FerroEhrService) {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_external_terminology(Arc::new(provider(&server.uri())));
    (db, svc)
}

async fn assert_vetdf_refusal(svc: &FerroEhrService) {
    let err = svc
        .upload_artefact(archetype_with_external_binding())
        .await
        .expect_err("a term the served code system does not contain must be refused");
    assert_eq!(
        err.status,
        CallStatusType::ContentInvalid,
        "VETDF is a content-invalid (422) failure"
    );
    assert!(
        err.message.contains("VETDF"),
        "the rejection carries the VETDF rule code: {}",
        err.message
    );
    assert!(
        !svc.has_artefact(HRID.to_owned())
            .await
            .expect("has_artefact answers"),
        "a rejected artefact is not stored"
    );
}

async fn assert_accepted(svc: &FerroEhrService) {
    svc.upload_artefact(archetype_with_external_binding())
        .await
        .expect("the archetype is accepted");
    assert!(
        svc.has_artefact(HRID.to_owned())
            .await
            .expect("has_artefact answers"),
        "the accepted artefact is stored"
    );
}

#[tokio::test]
async fn a_code_absent_from_a_served_system_is_rejected_with_vetdf() {
    let server = MockServer::start().await;
    // FerroTERM's shape: 400 with an `invalid-code` issue.
    mount_lookup(
        &server,
        ResponseTemplate::new(400).set_body_json(outcome(
            "invalid-code",
            "code `271649006` is not in code system `http://snomed.info/sct`",
        )),
    )
    .await;
    let (_db, svc) = service_at(&server).await;
    assert_vetdf_refusal(&svc).await;
}

#[tokio::test]
async fn a_code_system_the_server_does_not_serve_leaves_the_binding_unverified() {
    let server = MockServer::start().await;
    // FerroTERM's shape: 404 with a `not-found` issue. The spec's VETDF makes
    // this a warning, not a refusal: no verification was possible.
    mount_lookup(
        &server,
        ResponseTemplate::new(404).set_body_json(outcome(
            "not-found",
            "code system `http://snomed.info/sct` is not served",
        )),
    )
    .await;
    let (_db, svc) = service_at(&server).await;
    assert_accepted(&svc).await;
}

#[tokio::test]
async fn a_resolving_lookup_is_accepted() {
    let server = MockServer::start().await;
    mount_lookup(
        &server,
        ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "display", "valueString": "Entire lung"}]
        })),
    )
    .await;
    let (_db, svc) = service_at(&server).await;
    assert_accepted(&svc).await;
}

#[tokio::test]
async fn a_bare_404_on_a_served_system_is_rejected_after_the_probe() {
    let server = MockServer::start().await;
    // An older server answers every miss with a bare 404. The probe finds the
    // code system served, so the miss means the code is absent.
    mount_lookup(&server, ResponseTemplate::new(404)).await;
    mount_system_probe(&server, 1).await;
    let (_db, svc) = service_at(&server).await;
    assert_vetdf_refusal(&svc).await;
}

#[tokio::test]
async fn a_bare_404_on_an_unserved_system_leaves_the_binding_unverified() {
    let server = MockServer::start().await;
    mount_lookup(&server, ResponseTemplate::new(404)).await;
    mount_system_probe(&server, 0).await;
    let (_db, svc) = service_at(&server).await;
    assert_accepted(&svc).await;
}

#[tokio::test]
async fn a_bare_404_the_probe_cannot_resolve_leaves_the_binding_unverified() {
    let server = MockServer::start().await;
    // No `CodeSystem` search at all: the server cannot say whether it serves
    // the system, so nothing was verified and nothing is refused.
    mount_lookup(&server, ResponseTemplate::new(404)).await;
    let (_db, svc) = service_at(&server).await;
    assert_accepted(&svc).await;
}
