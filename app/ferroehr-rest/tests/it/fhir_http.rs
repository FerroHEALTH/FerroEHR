// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the FHIR R4 connector API group (our own extension
//! — no openEHR spec governs this; E3): the config gate
//! (`AppConfig::fhir_api_enabled`), the starter-scope `501`, the inbound
//! `POST /fhir/r4/{resourceType}` outcomes, the `/admin/fhir_mapping` CRUD, and
//! the FHIR `OperationOutcome` error shape — driven through the assembled router
//! over the **real** `FerroEhrService` on a real Postgres database (the
//! scripted `Mock` is gone; the mapping CRUD persists to the real `fhir_mapping`
//! table, whose `template_id` is a foreign key into the template store).
//!
//! Design: FHIR↔openEHR mapping is spec-silent — our own extension; the surface
//! is our own, config-gated, dispatching to the `FhirConnectorAdapter`. Every
//! error is a FHIR `OperationOutcome`.
//!
//! Re-targets from the Mock: an empty DB has no mapping, so an inbound ingest is
//! a real `404 not-found` (was a scripted 201/422) and a search is a real empty
//! `searchset` Bundle (was a scripted one-entry Bundle). Creating a mapping
//! seeds the referenced OPT template first (the FK requires it).
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use ferroehr::config::auth::AuthConfig;
use ferroehr::config::server::ServerConfig;
use ferroehr_rest::config::AppConfig;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";
const MAPPINGS: &str = "/ferroehr/rest/openehr/v1/admin/fhir_mapping";
const INGEST_OBS: &str = "/ferroehr/rest/openehr/v1/fhir/r4/Observation";
/// The template the mapping FK references — a vendored OPT uploaded before a
/// mapping that names it can be created.
const TEMPLATE_ID: &str = "Demo Vitals";

fn config(enabled: bool) -> AppConfig {
    AppConfig {
        server: ServerConfig {
            bind: "127.0.0.1:0".to_owned(),
            base_path: BASE.to_owned(),
            max_in_flight: 1024,
            swagger_ui: ferroehr::config::management::AccessLevel::Off,
            cors_permissive: false,
            ..Default::default()
        },
        auth: AuthConfig {
            enabled: false,
            basic: None,
            oidc: None,
            ..AuthConfig::default()
        },
        fhir_api_enabled: enabled,
        ..Default::default()
    }
}

async fn app(enabled: bool) -> (testkit::TestDb, Router) {
    let (pg, service) = common::test_service().await;
    (
        pg,
        ferroehr_rest::build_with(config(enabled), service).expect("router builds"),
    )
}

/// Upload the vendored `Demo Vitals` OPT so a mapping referencing it satisfies
/// the `fhir_mapping.template_id` foreign key.
async fn upload_template(app: &Router) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/openehr-sdt/tests/fixtures/better/Demo Vitals.opt");
    let opt = std::fs::read_to_string(path).expect("Demo Vitals.opt vendored");
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/definition/template/adl1.4"))
        .header("content-type", "application/xml")
        .body(Body::from(opt))
        .unwrap();
    let resp = app.clone().oneshot(req).await.expect("upload template");
    assert_eq!(
        resp.status(),
        StatusCode::CREATED,
        "the Demo Vitals template uploads"
    );
}

async fn send(app: Router, req: Request<Body>) -> (StatusCode, Option<String>, Value) {
    let resp = app.oneshot(req).await.expect("response");
    let status = resp.status();
    let location = resp
        .headers()
        .get(http::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, location, body)
}

fn req(method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
    let builder = Request::builder().method(method).uri(uri);
    match body {
        Some(v) => builder
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&v).unwrap()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

/// Assert a body is a FHIR `OperationOutcome` with the given issue code.
fn assert_operation_outcome(body: &Value, code: &str) {
    assert_eq!(
        body["resourceType"], "OperationOutcome",
        "body is an OperationOutcome"
    );
    assert_eq!(body["issue"][0]["code"], code, "issue code");
    assert!(
        body["issue"][0]["diagnostics"].is_string(),
        "has diagnostics"
    );
}

/// A valid mapping definition referencing the seeded template.
fn mapping(name: &str) -> Value {
    json!({
        "name": name,
        "definition": {
            "resource_type": "Observation",
            "template_id": TEMPLATE_ID,
            "subject": { "reference_path": "subject.reference", "namespace": "fhir" },
            "entries": []
        }
    })
}

// ── config gate ─────────────────────────────────────────────────────────────
#[tokio::test]
async fn disabled_connector_is_404_operation_outcome() {
    // Mapping CRUD off.
    let (_pg, a) = app(false).await;
    let (status, _, body) = send(a, req("GET", MAPPINGS, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_operation_outcome(&body, "not-supported");
    // Inbound off.
    let obs = json!({ "resourceType": "Observation", "subject": { "reference": "Patient/x" } });
    let (_pg, a) = app(false).await;
    let (status, _, body) = send(a, req("POST", INGEST_OBS, Some(obs))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_operation_outcome(&body, "not-supported");
}

// ── starter scope ─────────────────────────────────────────────────────────────
#[tokio::test]
async fn unknown_resource_type_is_501_before_backend() {
    // MedicationRequest is outside the starter set → typed 501 at the protocol edge.
    let uri = format!("{BASE}/fhir/r4/MedicationRequest");
    let body = json!({ "resourceType": "MedicationRequest" });
    let (_pg, a) = app(true).await;
    let (status, _, oo) = send(a, req("POST", &uri, Some(body))).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert_operation_outcome(&oo, "not-supported");
}

// ── read façade ───────────────────────────────────────────────────────────────
#[tokio::test]
async fn search_returns_empty_searchset_on_empty_db() {
    // Re-targeted: with no mapping/data the real façade returns an empty
    // searchset Bundle (was a Mock-scripted one-entry Bundle).
    let uri = format!("{INGEST_OBS}?patient=p-1&_count=10");
    let (_pg, a) = app(true).await;
    let resp = a.oneshot(req("GET", &uri, None)).await.expect("response");
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/fhir+json"),
    );
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    let bundle: Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(bundle["resourceType"], "Bundle");
    assert_eq!(bundle["type"], "searchset");
    assert_eq!(bundle["total"], 0);
    assert_eq!(bundle["entry"].as_array().map_or(0, Vec::len), 0);
}

#[tokio::test]
async fn search_missing_patient_is_400() {
    // No patient param → 400 (explicit scope only; never generic Search).
    let (_pg, a) = app(true).await;
    let (status, _, oo) = send(a, req("GET", INGEST_OBS, None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_operation_outcome(&oo, "required");
}

#[tokio::test]
async fn search_unknown_type_is_501() {
    let uri = format!("{BASE}/fhir/r4/MedicationRequest?patient=p-1");
    let (_pg, a) = app(true).await;
    let (status, _, oo) = send(a, req("GET", &uri, None)).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert_operation_outcome(&oo, "not-supported");
}

#[tokio::test]
async fn search_disabled_is_404() {
    let uri = format!("{INGEST_OBS}?patient=p-1");
    let (_pg, a) = app(false).await;
    let (status, _, oo) = send(a, req("GET", &uri, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_operation_outcome(&oo, "not-supported");
}

// ── inbound ingest outcomes ───────────────────────────────────────────────────
#[tokio::test]
async fn ingest_no_mapping_is_404() {
    // No enabled mapping on an empty DB → 404 not-found (an in-scope resource
    // type with no mapping). Was Mock-scripted per resource type.
    let obs = json!({ "resourceType": "Observation", "subject": { "reference": "Patient/p" } });
    let (_pg, a) = app(true).await;
    let (status, _, oo) = send(a, req("POST", INGEST_OBS, Some(obs))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_operation_outcome(&oo, "not-found");
}

#[tokio::test]
async fn ingest_condition_no_mapping_is_404() {
    // Re-targeted from the old `unhooked → 501`: Condition is in scope but has no
    // mapping on an empty DB → real 404 not-found (never the trait-default 501).
    let uri = format!("{BASE}/fhir/r4/Condition");
    let body = json!({ "resourceType": "Condition", "subject": { "reference": "Patient/p" } });
    let (_pg, a) = app(true).await;
    let (status, _, oo) = send(a, req("POST", &uri, Some(body))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_operation_outcome(&oo, "not-found");
}

#[tokio::test]
#[ignore = "needs a verified FHIR-mapping→composition transform fixture: the Mock \
scripted a committed COMPOSITION; a real 201 requires an enabled mapping whose \
entry rules map a raw Observation onto a valid composition end-to-end (template \
+ mapping definition + transform), which is a separate integration fixture. \
Re-target once such a fixture exists."]
async fn ingest_success_is_201_with_location() {
    let obs = json!({
        "resourceType": "Observation",
        "id": "bp-1",
        "subject": { "reference": "Patient/p-1" },
        "component": [ { "valueQuantity": { "value": 120, "unit": "mm[Hg]" } } ]
    });
    let (_pg, a) = app(true).await;
    let (status, location, body) = send(a, req("POST", INGEST_OBS, Some(obs))).await;
    assert_eq!(status, StatusCode::CREATED);
    let loc = location.expect("Location header present");
    assert!(
        loc.contains("/composition/"),
        "location targets the composition: {loc}"
    );
    assert_operation_outcome(&body, "informational");
}

#[tokio::test]
#[ignore = "needs a verified FHIR-mapping→composition transform fixture that \
produces a validator-rejected composition (422). The Mock scripted the rejection; \
reproducing it requires a real mapping whose transform yields an invalid \
composition. Re-target once such a fixture exists."]
async fn ingest_validation_rejection_is_422_with_validator_message() {
    let obs = json!({
        "resourceType": "Observation",
        "subject": { "reference": "Patient/invalid" }
    });
    let (_pg, a) = app(true).await;
    let (status, _, body) = send(a, req("POST", INGEST_OBS, Some(obs))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_operation_outcome(&body, "invalid");
    assert!(
        body["issue"][0]["diagnostics"]
            .as_str()
            .unwrap()
            .contains("missing mandatory element 'systolic'"),
        "validator message surfaced verbatim"
    );
}

// ── mapping CRUD ──────────────────────────────────────────────────────────────
#[tokio::test]
async fn mapping_crud_round_trip() {
    let (_pg, app) = app(true).await;
    upload_template(&app).await;

    let (status, _, list) = send(app.clone(), req("GET", MAPPINGS, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 0);

    let (status, _, created) =
        send(app.clone(), req("POST", MAPPINGS, Some(mapping("obs-bp")))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["name"], "obs-bp");
    assert_eq!(created["resource_type"], "Observation");
    let id = created["id"].as_str().unwrap().to_owned();

    let (status, _, got) = send(app.clone(), req("GET", &format!("{MAPPINGS}/{id}"), None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got["id"], id);

    let (status, _, _) = send(
        app.clone(),
        req("DELETE", &format!("{MAPPINGS}/{id}"), None),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, oo) = send(app, req("GET", &format!("{MAPPINGS}/{id}"), None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_operation_outcome(&oo, "not-found");
}

#[tokio::test]
async fn mapping_duplicate_name_is_409() {
    let (_pg, app) = app(true).await;
    upload_template(&app).await;
    let (status, _, _) = send(app.clone(), req("POST", MAPPINGS, Some(mapping("dup")))).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, oo) = send(app, req("POST", MAPPINGS, Some(mapping("dup")))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_operation_outcome(&oo, "conflict");
}

#[tokio::test]
async fn mapping_malformed_id_is_400() {
    let (_pg, a) = app(true).await;
    let (status, _, oo) = send(a, req("GET", &format!("{MAPPINGS}/not-a-uuid"), None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_operation_outcome(&oo, "invalid");
}

#[tokio::test]
async fn mapping_list_enabled_is_200() {
    // Re-targeted from the old `unhooked → 501`: the mapping store is real, so an
    // enabled group lists (200), never the trait-default 501.
    let (_pg, a) = app(true).await;
    let (status, _, list) = send(a, req("GET", MAPPINGS, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(list.is_array());
}
