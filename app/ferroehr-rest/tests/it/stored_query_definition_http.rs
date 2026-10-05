// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the stored-query DEFINITION wire (group-11 audit,
//! `I_DEFINITION_QUERY`): the version-less `PUT /definition/query/{name}`
//! (whose `Location` names exactly the version the store wrote — never a
//! neighbouring one, `headers/Location_Query.yaml`) and the versioned
//! `PUT …/{name}/{version}` (exact `major.minor.patch` required — the
//! `{major}`/`{major}.{minor}` prefix forms of `parameters/path/version.yaml`
//! are READ-side resolution patterns, and a non-numeric segment would poison
//! the surface's SEMVER ordering; register-adjudicated `400`).
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;

use ferroehr::config::auth::AuthConfig;
use ferroehr::config::server::ServerConfig;
use ferroehr_rest::config::AppConfig;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";
const AQL: &str = "SELECT e/ehr_id/value FROM EHR e";

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

async fn app() -> (testkit::TestDb, Router) {
    let (pg, service) = common::test_service().await;
    (pg, common::router_with(config(), service))
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, header::HeaderMap, String) {
    let resp = app.clone().oneshot(req).await.expect("response");
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn put(uri: String) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(uri)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(AQL))
        .unwrap()
}

fn location(h: &header::HeaderMap) -> &str {
    h.get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .expect("Location header")
}

// ── the version-less store's Location names the WRITTEN version ─────────────

#[tokio::test]
async fn versionless_store_location_names_the_default_slot() {
    let (_pg, app) = app().await;
    let (status, headers, body) =
        send(&app, put(format!("{BASE}/definition/query/org.test::a"))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        location(&headers),
        format!("{BASE}/definition/query/org.test::a/1.0.0"),
        "the version-less store writes the 1.0.0 default slot and Location \
         names exactly that resource (headers/Location_Query.yaml)"
    );
}

#[tokio::test]
async fn versionless_store_location_ignores_higher_stored_versions() {
    let (_pg, app) = app().await;
    // A higher version exists first…
    let (status, _h, body) = send(
        &app,
        put(format!("{BASE}/definition/query/org.test::b/2.0.0")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // …then the version-less store writes 1.0.0: Location must name 1.0.0,
    // never the neighbouring 2.0.0 this PUT did not touch.
    let (status, headers, body) =
        send(&app, put(format!("{BASE}/definition/query/org.test::b"))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        location(&headers),
        format!("{BASE}/definition/query/org.test::b/1.0.0"),
        "Location indicates the URL of the Stored Query resource this PUT \
         stored (headers/Location_Query.yaml), not the highest version"
    );
}

#[tokio::test]
async fn versionless_store_updates_in_place() {
    let (_pg, app) = app().await;
    let uri = format!("{BASE}/definition/query/org.test::c");
    let (status, _h, body) = send(&app, put(uri.clone())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // "Stores a new query, or updates an existing query on the system"
    // (definition_query_store.yaml) — the second PUT is an update, not a 409.
    let (status, headers, body) = send(&app, put(uri)).await;
    assert_eq!(status, StatusCode::OK, "update-in-place: {body}");
    assert!(location(&headers).ends_with("/1.0.0"));
}

// ── the versioned store requires an exact numeric SEMVER triple ─────────────

#[tokio::test]
async fn versioned_store_accepts_exact_semver_and_conflicts_on_duplicate() {
    let (_pg, app) = app().await;
    let uri = format!("{BASE}/definition/query/org.test::d/1.2.3");
    let (status, headers, body) = send(&app, put(uri.clone())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        location(&headers),
        format!("{BASE}/definition/query/org.test::d/1.2.3")
    );
    // "409 Conflict is returned when a query with the given
    // qualified_query_name and version already exists on the server"
    // (responses/409_StoredQuery_version.yaml).
    let (status, _h, body) = send(&app, put(uri)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

#[tokio::test]
async fn versioned_store_rejects_non_exact_versions() {
    let (_pg, app) = app().await;
    // The prefix forms are read-side resolution patterns
    // (parameters/path/version.yaml) that resolve to nothing writable on a
    // store, and the released identifier format is three numeric parts
    // ("SEMVER style (i.e. `major.minor.patch`)",
    // docs/query/Qualified_query_name.md). Every other token is the 400 the
    // docs text assigns to a client error no other 4xx fits
    // (docs/overview/Requests_and_responses.md §"HTTP status codes").
    for bad in ["1", "1.0", "1.0.0-rc.1", "latest", "1.0.x", "1..0"] {
        let (status, _h, body) = send(
            &app,
            put(format!("{BASE}/definition/query/org.test::e/{bad}")),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "`{bad}` is not an exact major.minor.patch: {body}"
        );
    }
}

#[tokio::test]
async fn version_get_resolves_prefixes_to_highest() {
    let (_pg, app) = app().await;
    for v in ["1.0.0", "1.0.9", "1.2.0"] {
        let (status, _h, body) = send(
            &app,
            put(format!("{BASE}/definition/query/org.test::f/{v}")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    // "a pattern as partial prefix … the highest (latest) version matching
    // the prefix will be considered" (parameters/path/version.yaml).
    for (sel, expect) in [("1", "1.2.0"), ("1.0", "1.0.9"), ("1.0.0", "1.0.0")] {
        let get = Request::builder()
            .method("GET")
            .uri(format!("{BASE}/definition/query/org.test::f/{sel}"))
            .header(header::ACCEPT, "application/json")
            .body(Body::empty())
            .unwrap();
        let (status, _h, body) = send(&app, get).await;
        assert_eq!(status, StatusCode::OK, "{sel}: {body}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            v.get("version").and_then(|x| x.as_str()),
            Some(expect),
            "selector `{sel}` resolves to `{expect}`: {body}"
        );
    }
}

// ── a non-text/plain declared body type is 415, never a parse-time 400 ──────

#[tokio::test]
async fn stores_refuse_a_declared_foreign_body_type_with_415() {
    let (_pg, app) = app().await;
    // Both store forms declare text/plain as the single body type; a payload
    // declaring another media type cannot be processed as it (`Resources.md`
    // §format rules: the service "MUST respond with HTTP status code 415
    // Unsupported Media Type").
    for uri in [
        format!("{BASE}/definition/query/org.test::g"),
        format!("{BASE}/definition/query/org.test::g/1.0.0"),
    ] {
        let req = Request::builder()
            .method("PUT")
            .uri(uri.clone())
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(AQL))
            .unwrap();
        let (status, _h, body) = send(&app, req).await;
        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{uri}: {body}");
    }
}

#[tokio::test]
async fn stores_accept_an_absent_content_type() {
    let (_pg, app) = app().await;
    // An absent Content-Type declares nothing to refuse (the header is a
    // client MAY) — the operation's own body type applies.
    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/definition/query/org.test::h/1.0.0"))
        .body(Body::from(AQL))
        .unwrap();
    let (status, _h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}
