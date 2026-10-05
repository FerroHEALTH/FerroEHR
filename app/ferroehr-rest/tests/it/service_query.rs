// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the DEFINITION stored-query store surface against a
//! real `PostgreSQL` 18 (the shared `testkit` harness), driven through the
//! assembled `ferroehr-rest` router (auth disabled) with `tower`'s `oneshot`.
//!
//! Covers the spec-mandated write semantics fixed in the W1-C audit wave:
//! - a versioned store returns `200 OK` + a `Location` header
//!   (`responses/200_StoredQuery_stored.yaml` + `headers/Location_Query.yaml`),
//!   not `204`;
//! - re-storing an existing `(name, version)` returns `409 Conflict`
//!   (`responses/409_StoredQuery_version.yaml`), never a silent overwrite;
//! - the no-version store path upserts (spec: "stores a new query, or updates
//!   an existing query", `operations/definition_query_store.yaml`, no `409`).
#![expect(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tower::ServiceExt;

use ferroehr::service::FerroEhrService;
use ferroehr_rest::config::AppConfig;

const BASE: &str = "/ferroehr/rest/openehr/v1";
const AQL: &str = "SELECT c FROM EHR e CONTAINS COMPOSITION c";

/// The router backed by the DB service, with authentication disabled.
fn app(pool: &PgPool) -> Router {
    let mut config = AppConfig::default();
    config.auth.enabled = false;
    ferroehr_rest::build_with(
        config,
        std::sync::Arc::new(FerroEhrService::new(
            &ferroehr::db::domain::DomainPools::from_shared(pool),
        )),
    )
    .expect("router builds")
}

async fn put(app: Router, uri: &str, body: &str) -> (StatusCode, http::HeaderMap, String) {
    let req = Request::builder()
        .method("PUT")
        .uri(uri)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(body.to_owned()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

async fn get(app: Router, uri: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn post_json(app: Router, uri: &str, body: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// The QUERY API is wired end to end through the router: `POST /query/aql`
/// and `GET /query/aql` execute an ad-hoc query and return a `RESULT_SET`
/// (`schemas/query/ResultSet`: `columns` + `rows`, `_schema_version` 1.0.3).
#[tokio::test]
async fn adhoc_aql_over_http_returns_result_set() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();

    // Empty store → COUNT(*) over compositions is 0, but the endpoint, dispatch,
    // engine, and RESULT_SET assembly are all exercised.
    let (status, body) = post_json(
        app(&pool.clone()),
        &format!("{BASE}/query/aql"),
        r#"{"q":"SELECT COUNT(*) FROM EHR e CONTAINS COMPOSITION c"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "adhoc POST /query/aql: {body}");
    let json: serde_json::Value = serde_json::from_str(&body).expect("result set json");
    assert_eq!(
        json["meta"]["_schema_version"],
        serde_json::json!(ferroehr::telemetry::provenance::ITS_REST),
        "RESULT_SET carries the implemented ITS-REST release as its schema version"
    );
    assert_eq!(
        json["rows"],
        serde_json::json!([[0]]),
        "COUNT(*) = 0 on an empty store"
    );

    // The GET form takes `q` as a query parameter (URL-encoded).
    let (status, body) = get(
        app(&pool),
        &format!(
            "{BASE}/query/aql?q=SELECT%20COUNT(*)%20FROM%20EHR%20e%20CONTAINS%20COMPOSITION%20c"
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "adhoc GET /query/aql: {body}");
    let json: serde_json::Value = serde_json::from_str(&body).expect("result set json");
    assert_eq!(json["rows"], serde_json::json!([[0]]));
}

/// A malformed AQL query is a `400 Bad Request` (ITS-REST `400_QUERY.yaml`), not
/// a `500` — the parse failure is surfaced as a client error.
#[tokio::test]
async fn malformed_adhoc_aql_is_400() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let (status, _body) = post_json(
        app(&pool),
        &format!("{BASE}/query/aql"),
        r#"{"q":"SELECT FROM WHERE not aql"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "malformed AQL → 400");
}

#[tokio::test]
async fn versioned_store_returns_200_with_location_and_409_on_duplicate() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let name = "org.example::test_query";
    let uri = format!("{BASE}/definition/query/{name}/1.0.0");

    // First store: 200 OK + Location pointing at the stored resource.
    let (status, headers, _body) = put(app(&pool.clone()), &uri, AQL).await;
    assert_eq!(status, StatusCode::OK, "store success is 200, not 204");
    let location = headers
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .expect("Location header present");
    assert_eq!(
        location,
        format!("{BASE}/definition/query/{name}/1.0.0"),
        "Location points at the stored query version"
    );

    // Re-storing the same name+version is a 409 Conflict (immutable version).
    let (status, _headers, _body) = put(app(&pool.clone()), &uri, AQL).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "re-storing an existing (name, version) conflicts"
    );

    // The stored query is retrievable and untouched.
    let (status, body) = get(app(&pool), &format!("{BASE}/definition/query/{name}/1.0.0")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(AQL), "stored AQL retrievable: {body}");
}

#[tokio::test]
async fn no_version_store_upserts_and_returns_200() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let name = "org.example::auto_query";
    let uri = format!("{BASE}/definition/query/{name}");

    // No-version store: 200 OK (auto-assigned version).
    let (status, _headers, _body) = put(app(&pool.clone()), &uri, AQL).await;
    assert_eq!(status, StatusCode::OK, "no-version store success is 200");

    // Re-storing the no-version path updates rather than conflicting (spec:
    // "stores a new query, or updates an existing query").
    let updated = "SELECT c FROM EHR e CONTAINS COMPOSITION c WHERE c/name/value = 'x'";
    let (status, _headers, _body) = put(app(&pool.clone()), &uri, updated).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "no-version re-store upserts, no 409"
    );

    // The latest text is what is retrieved.
    let (status, body) = get(app(&pool), &format!("{BASE}/definition/query/{name}/1.0.0")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(updated),
        "no-version store updated text: {body}"
    );
}

/// A SELECT mixing an aggregate with a plain projection is a typed `400`, not
/// the database's ungrouped-column error surfacing as a `500` (#3054; QUERY
/// master03 §Aggregate functions defines no grouping).
#[tokio::test]
async fn mixed_aggregate_projection_is_a_typed_400() {
    let db = testkit::db().await.expect("testkit database");
    let (status, body) = post_json(
        app(&db.pool().clone()),
        &format!("{BASE}/query/aql"),
        r#"{"q":"SELECT e/ehr_id/value, COUNT(c/uid/value) FROM EHR e CONTAINS COMPOSITION c"}"#,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "mixed aggregate → 400: {body}"
    );
    assert!(
        body.contains("aggregate"),
        "the refusal names the rule, not a database error: {body}"
    );
}
