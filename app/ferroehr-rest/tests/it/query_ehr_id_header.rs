// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `openehr-ehr-id` request header as the single-EHR query scope, end to
//! end through the assembled router over a **real** `FerroEhrService` on a real
//! `PostgreSQL` (the `common` fixture).
//!
//! Oracle — ITS-REST `docs/query/Request.md` §"About the `ehr_id` parameter":
//! "Depending on the needs, clients MAY supply it as a query parameter
//! `ehr_id` or alternatively as a request header named `openehr-ehr-id`."
//! §"Common Headers and Query Parameters" lists it under "Related request
//! headers ... used to execute the query within a single EHR context", opening
//! with "All query execution requests SHOULD support at least the following
//! parameters"; `docs/query/Query_types.md` §"Single EHR queries" repeats the
//! pair. None of them distinguishes HTTP methods or ad-hoc from stored
//! execution, so **every** execution operation must honour the header.
//!
//! Both-supplied precedence is spec-silent, so the handling is our own:
//! agreeing forms execute, conflicting forms are a `400`
//! (`docs/overview/Requests_and_responses.md` §"HTTP status codes", row `400`).
//!
//! Each scoping test seeds **two** EHRs with one composition each and runs an
//! AQL text that constrains no EHR itself (a population query,
//! `Query_types.md` §"Population queries"), so the header is the only thing
//! narrowing the result: an ignored header yields both compositions.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::missing_assert_message,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use ferroehr::config::auth::AuthConfig;
use ferroehr::config::server::ServerConfig;
use ferroehr_rest::config::AppConfig;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";
/// The `ehr_id` scope header (`Request.md` §Common Headers and Query
/// Parameters). `openEHR-EHR-id` is its deprecated spelling
/// (`Requests_and_responses.md` §Deprecated headers) and resolves to the same
/// field, HTTP field names being case-insensitive (RFC 9110 §5.1).
const H_EHR_ID: &str = "openehr-ehr-id";
/// A population query: the AQL constrains no EHR, so only the wire scope can
/// narrow it.
const POPULATION_AQL: &str = "SELECT c/uid/value AS uid FROM EHR e CONTAINS COMPOSITION c";
const STORED_NAME: &str = "org.openehr.test::ehr_id_header";

// The IPS OPT + its canonical composition — the pair driven end-to-end through
// the real `FerroEhrService` elsewhere in this suite, so they commit cleanly.
fn opt_xml() -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/openehr-its/tests/fixtures/sdk/ips.v0.opt"),
    )
    .expect("ips.v0.opt vendored in openehr-its")
}

fn canonical_composition() -> Value {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../crates/openehr-its/tests/vendor/openehr_sdk/composition/canonical_json/ips_canonical.json",
        ),
    )
    .expect("ips_canonical.json vendored in openehr-its");
    serde_json::from_str(&text).expect("valid canonical composition")
}

fn config() -> AppConfig {
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
        ..Default::default()
    }
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, String) {
    let resp = app.clone().oneshot(req).await.expect("response");
    let status = resp.status();
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn etag_uid(resp_headers: &header::HeaderMap) -> String {
    resp_headers
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag present")
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_owned()
}

async fn create_ehr(app: &Router) -> String {
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.expect("response");
    assert_eq!(resp.status(), StatusCode::CREATED);
    etag_uid(resp.headers())
}

async fn upload_opt(app: &Router) {
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/definition/template/adl1.4"))
        .header(header::CONTENT_TYPE, "application/xml")
        .body(Body::from(opt_xml()))
        .unwrap();
    let (status, body) = send(app, req).await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");
}

/// Commit the IPS composition into `ehr_id`; return the version uid.
async fn commit_composition(app: &Router, ehr_id: &str) -> String {
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(canonical_composition().to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.expect("response");
    assert_eq!(resp.status(), StatusCode::CREATED, "composition commit");
    etag_uid(resp.headers())
}

/// Store [`POPULATION_AQL`] as a named stored query; return its assigned
/// version (from the `Location` of the store response).
async fn store_query(app: &Router) -> String {
    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/definition/query/{STORED_NAME}"))
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(POPULATION_AQL))
        .unwrap();
    let resp = app.clone().oneshot(req).await.expect("response");
    assert_eq!(resp.status(), StatusCode::OK, "stored-query PUT");
    resp.headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|l| l.rsplit('/').next())
        .expect("Location carries the assigned version")
        .to_owned()
}

/// The seeded world: the router, EHR **A** with its committed composition uid,
/// and EHR **B** with its own — two EHRs so an ignored scope is visible.
struct World {
    _pg: testkit::TestDb,
    app: Router,
    ehr_a: String,
    uid_a: String,
    ehr_b: String,
    uid_b: String,
}

async fn world() -> World {
    let (pg, service) = common::test_service().await;
    let app = common::router_with(config(), service);
    upload_opt(&app).await;
    let ehr_a = create_ehr(&app).await;
    let uid_a = commit_composition(&app, &ehr_a).await;
    let ehr_b = create_ehr(&app).await;
    let uid_b = commit_composition(&app, &ehr_b).await;
    assert_ne!(uid_a, uid_b, "each commit gets its own OBJECT_VERSION_ID");
    World {
        _pg: pg,
        app,
        ehr_a,
        uid_a,
        ehr_b,
        uid_b,
    }
}

/// The single projected column of a `RESULT_SET`, as strings.
fn uids(body: &str) -> Vec<String> {
    let v: Value = serde_json::from_str(body).expect("RESULT_SET json");
    v["rows"]
        .as_array()
        .expect("RESULT_SET.rows")
        .iter()
        .map(|row| row[0].as_str().expect("uid cell").to_owned())
        .collect()
}

fn q() -> String {
    urlencoding::encode(POPULATION_AQL).into_owned()
}

/// A `GET`/`POST` execution request with an optional `openehr-ehr-id` header.
fn execute(
    method: &str,
    uri: String,
    ehr_id_header: Option<&str>,
    body: Option<&str>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(value) = ehr_id_header {
        builder = builder.header(H_EHR_ID, value);
    }
    match body {
        Some(json) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json.to_owned()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

/// The unscoped baseline: with no `ehr_id` in either form the population query
/// sees BOTH EHRs — so every scoped assertion below is a real narrowing, not a
/// query that could only ever return one row.
#[tokio::test]
async fn unscoped_population_query_sees_both_ehrs() {
    let w = world().await;
    let (status, body) = send(
        &w.app,
        execute("GET", format!("{BASE}/query/aql?q={}", q()), None, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = uids(&body);
    assert!(
        rows.contains(&w.uid_a) && rows.contains(&w.uid_b),
        "{rows:?}"
    );
}

// ── the header alone scopes the execution (Request.md §About the ehr_id parameter)

#[tokio::test]
async fn get_adhoc_is_scoped_by_the_header_alone() {
    let w = world().await;
    let (status, body) = send(
        &w.app,
        execute(
            "GET",
            format!("{BASE}/query/aql?q={}", q()),
            Some(&w.ehr_a),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(uids(&body), vec![w.uid_a.clone()], "only EHR A's rows");
}

#[tokio::test]
async fn get_adhoc_accepts_the_deprecated_header_spelling() {
    // Requests_and_responses.md §Deprecated headers pairs `openEHR-EHR-id` with
    // `openehr-ehr-id` and keeps the deprecated form "available for backward
    // compatibility"; RFC 9110 §5.1 makes field names case-insensitive.
    let w = world().await;
    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/query/aql?q={}", q()))
        .header("openEHR-EHR-id", &w.ehr_a)
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(&w.app, req).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(uids(&body), vec![w.uid_a.clone()]);
}

#[tokio::test]
async fn get_stored_query_is_scoped_by_the_header_alone() {
    let w = world().await;
    store_query(&w.app).await;
    let (status, body) = send(
        &w.app,
        execute(
            "GET",
            format!("{BASE}/query/{STORED_NAME}"),
            Some(&w.ehr_b),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(uids(&body), vec![w.uid_b.clone()], "only EHR B's rows");
}

#[tokio::test]
async fn get_stored_query_version_is_scoped_by_the_header_alone() {
    let w = world().await;
    let version = store_query(&w.app).await;
    let (status, body) = send(
        &w.app,
        execute(
            "GET",
            format!("{BASE}/query/{STORED_NAME}/{version}"),
            Some(&w.ehr_a),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(uids(&body), vec![w.uid_a.clone()]);
}

#[tokio::test]
async fn post_adhoc_is_scoped_by_the_header_alone() {
    // The POST paths already honoured the header; pinned here so GET and POST
    // stay one behaviour.
    let w = world().await;
    let body = serde_json::json!({ "q": POPULATION_AQL }).to_string();
    let (status, out) = send(
        &w.app,
        execute(
            "POST",
            format!("{BASE}/query/aql"),
            Some(&w.ehr_a),
            Some(&body),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(uids(&out), vec![w.uid_a.clone()]);
}

#[tokio::test]
async fn post_stored_query_is_scoped_by_the_header_alone() {
    let w = world().await;
    store_query(&w.app).await;
    let body = serde_json::json!({ "offset": 0, "fetch": 100, "query_parameters": {} }).to_string();
    let (status, out) = send(
        &w.app,
        execute(
            "POST",
            format!("{BASE}/query/{STORED_NAME}"),
            Some(&w.ehr_b),
            Some(&body),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(uids(&out), vec![w.uid_b.clone()]);
}

// ── both forms supplied (spec-silent — our own handling) ─────────────────────

#[tokio::test]
async fn matching_parameter_and_header_execute_normally() {
    // The request names ONE EHR twice — nothing to arbitrate, so it executes.
    let w = world().await;
    let (status, body) = send(
        &w.app,
        execute(
            "GET",
            format!("{BASE}/query/aql?q={}&ehr_id={}", q(), w.ehr_a),
            Some(&w.ehr_a),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(uids(&body), vec![w.uid_a.clone()]);
}

#[tokio::test]
async fn conflicting_parameter_and_header_are_bad_request() {
    // A request naming two DIFFERENT EHRs is self-contradictory and no released
    // rule picks a winner → 400 (Requests_and_responses.md §HTTP status codes,
    // row 400: "the service cannot or will not process the request due to
    // something that is perceived to be a client error").
    // Every execution operation, both methods.
    let w = world().await;
    let version = store_query(&w.app).await;
    let adhoc_body = serde_json::json!({ "q": POPULATION_AQL }).to_string();
    let stored_body =
        serde_json::json!({ "offset": 0, "fetch": 100, "query_parameters": {} }).to_string();

    let cases: Vec<(&str, String, Option<&str>)> = vec![
        (
            "GET",
            format!("{BASE}/query/aql?q={}&ehr_id={}", q(), w.ehr_b),
            None,
        ),
        (
            "POST",
            format!("{BASE}/query/aql?ehr_id={}", w.ehr_b),
            Some(adhoc_body.as_str()),
        ),
        (
            "GET",
            format!("{BASE}/query/{STORED_NAME}?ehr_id={}", w.ehr_b),
            None,
        ),
        (
            "POST",
            format!("{BASE}/query/{STORED_NAME}?ehr_id={}", w.ehr_b),
            Some(stored_body.as_str()),
        ),
        (
            "GET",
            format!("{BASE}/query/{STORED_NAME}/{version}?ehr_id={}", w.ehr_b),
            None,
        ),
    ];
    for (method, uri, body) in cases {
        // The header names EHR A while the query parameter names EHR B.
        let (status, out) = send(&w.app, execute(method, uri.clone(), Some(&w.ehr_a), body)).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{method} {uri} must reject a conflicting EHR scope: {out}"
        );
    }
}

#[tokio::test]
async fn an_empty_header_value_does_not_conflict() {
    // RFC 9110 §5.5 permits empty field values; an empty `openehr-ehr-id`
    // carries no EHR identifier, so the query parameter still scopes.
    let w = world().await;
    let (status, body) = send(
        &w.app,
        execute(
            "GET",
            format!("{BASE}/query/aql?q={}&ehr_id={}", q(), w.ehr_a),
            Some(""),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(uids(&body), vec![w.uid_a.clone()]);
}

/// A well-formed `ehr_id` that names no EHR is a `404` whichever form carries
/// it — the header must not silently degrade the scope to a population query.
#[tokio::test]
async fn header_scope_naming_no_ehr_is_not_found() {
    let w = world().await;
    let absent = "3fa85f64-5717-4562-b3fc-2c963f66afa6";
    let (status, body) = send(
        &w.app,
        execute(
            "GET",
            format!("{BASE}/query/aql?q={}", q()),
            Some(absent),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

/// A malformed `ehr_id` in the header is a `400`, matching the query-parameter
/// form.
#[tokio::test]
async fn malformed_header_scope_is_bad_request() {
    let w = world().await;
    let (status, body) = send(
        &w.app,
        execute(
            "GET",
            format!("{BASE}/query/aql?q={}", q()),
            Some("not-a-uuid"),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}
