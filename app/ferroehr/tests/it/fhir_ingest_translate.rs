// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FHIR-ingest cross-terminology translation seam (no openEHR spec
//! governs FHIR conversion — our own design/extension): a `translate`
//! mapping entry drives `ConceptMap/$translate` through the terminology
//! router before the FLAT build, and a deployment with no provider for the
//! route fails CLOSED — never a silent pass-through of the untranslated
//! code.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use std::sync::Arc;

use ferroehr::service::FerroEhrService;
use ferroehr::service::status::CallStatusType;
use ferroehr::service::terminology::config::{FhirOperation, FhirProviderConfig, ProviderKind};
use ferroehr::service::terminology::fhir::FhirTerminologyProvider;
use ferroehr::service::terminology::router::TerminologyRouter;
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const OPT_REL: &str = "knowledge/opt/minimal_evaluation.opt";
const TEMPLATE_ID: &str = "minimal_evaluation.en.v1";

fn opt_xml() -> String {
    let path = format!(
        "{}/../../corpus/fixtures/service/{OPT_REL}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).expect("read the OPT fixture")
}

/// A mapping whose one coded entry requests LOINC → SNOMED CT translation.
fn translate_mapping() -> Value {
    json!({
        "name": "translate-probe",
        "definition": {
            "resource_type": "Observation",
            "template_id": TEMPLATE_ID,
            "subject": { "reference_path": "subject.reference", "namespace": "fhir",
                         "strip_prefix": "Patient/" },
            "context": { "ctx/language": "en", "ctx/territory": "US",
                         "ctx/composer_name": "fhir-connector",
                         "ctx/time": "2026-02-03T04:05:06Z" },
            "entries": [
                { "openehr_path": "minimal/minimal:0/coded_probe",
                  "fhir_path": "code.coding[0].code",
                  "transform": { "kind": "coded",
                    "system_path": "code.coding[0].system",
                    "translate": { "target_system": "http://snomed.info/sct" } },
                  "code_map": { "http://snomed.info/sct": "SNOMED-CT" } }
            ]
        }
    })
}

fn loinc_observation() -> Value {
    json!({
        "resourceType": "Observation",
        "id": "obs-translate-1",
        "status": "final",
        "subject": { "reference": "Patient/p-77" },
        "code": { "coding": [{ "system": "http://loinc.org", "code": "8480-6" }] }
    })
}

fn provider(base: &str) -> FhirTerminologyProvider {
    let cfg = FhirProviderConfig {
        kind: ProviderKind::Fhir,
        url: base.to_owned(),
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

#[tokio::test]
async fn translate_mapping_without_a_provider_fails_closed() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    svc.template_adl14_upload(opt_xml())
        .await
        .expect("ingest OPT");
    svc.fhir_mapping_create(translate_mapping())
        .await
        .expect("store mapping");

    let err = svc
        .fhir_ingest("Observation".to_owned(), None, loinc_observation())
        .await
        .expect_err("no terminology provider is configured");
    assert_eq!(
        err.status,
        CallStatusType::Exception,
        "a translate mapping on a deployment without a terminology provider is a \
         configuration fault, never a silent pass-through: {err:?}"
    );
}

#[tokio::test]
async fn translate_mapping_drives_the_terminology_seam_before_the_build() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ConceptMap/$translate"))
        .and(query_param("system", "http://loinc.org"))
        .and(query_param("code", "8480-6"))
        .and(query_param("targetsystem", "http://snomed.info/sct"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "result", "valueBoolean": true},
                {"name": "match", "part": [
                    {"name": "equivalence", "valueCode": "equivalent"},
                    {"name": "concept", "valueCoding": {
                        "system": "http://snomed.info/sct", "code": "271649006"}}
                ]}
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_terminology_router(Arc::new(TerminologyRouter::single(Arc::new(provider(
            &server.uri(),
        )))));
    svc.template_adl14_upload(opt_xml())
        .await
        .expect("ingest OPT");
    svc.fhir_mapping_create(translate_mapping())
        .await
        .expect("store mapping");

    // The template has no coded_probe node, so the FLAT build refuses AFTER
    // the seam ran — wiremock's expect(1) pins that the $translate call
    // happened, with the request's own coding and the entry's target system.
    let err = svc
        .fhir_ingest("Observation".to_owned(), None, loinc_observation())
        .await
        .expect_err("the coded_probe FLAT path is not in the template");
    assert_eq!(err.status, CallStatusType::ContentInvalid, "{err:?}");
    server.verify().await;
}
