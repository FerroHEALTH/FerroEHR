// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the SMART App Launch surface (group-15 audit,
//! #391): the discovery document's absolute `baseUrl`s (master04 §Services:
//! "Absolute URL to the root of the API `*(required)*`"), the honest
//! `capabilities` (`openehr-permission-v1` only in fail-closed mode), and the
//! fail-closed template/AQL scope enforcement (master08 §Resource Scopes —
//! the previously-inert two of the three resource families).
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
use serde_json::Value;
use tower::ServiceExt;

use ferroehr::config::auth::AuthConfig;
use ferroehr::config::server::ServerConfig;
use ferroehr_rest::config::AppConfig;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";

fn config(smart_enabled: bool, fail_closed: bool) -> AppConfig {
    let mut cfg = AppConfig {
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
    };
    cfg.smart.enabled = smart_enabled;
    cfg.smart.require_smart_scopes = fail_closed;
    cfg.smart.public_base_url = Some("https://cdr.example.com".to_owned());
    cfg.smart.endpoints.authorization_endpoint = Some("https://as.example/authorize".to_owned());
    cfg.smart.endpoints.token_endpoint = Some("https://as.example/token".to_owned());
    cfg
}

async fn app(smart_enabled: bool, fail_closed: bool) -> (testkit::TestDb, Router) {
    let (pg, service) = common::test_service().await;
    (
        pg,
        common::router_with(config(smart_enabled, fail_closed), service),
    )
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, String) {
    let resp = app.clone().oneshot(req).await.expect("response");
    let status = resp.status();
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header(header::ACCEPT, "application/json")
        .body(Body::empty())
        .unwrap()
}

// ── discovery ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn discovery_serves_absolute_base_urls() {
    let (_pg, app) = app(true, false).await;
    let (status, body) = send(&app, get("/ferroehr/rest/.well-known/smart-configuration")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let doc: Value = serde_json::from_str(&body).unwrap();
    let base = doc["services"]["org.openehr.rest"]["baseUrl"]
        .as_str()
        .expect("baseUrl");
    assert!(
        base.starts_with("https://cdr.example.com/"),
        "master04 §Services: baseUrl is an Absolute URL — got {base}"
    );
}

#[tokio::test]
async fn discovery_absent_when_disabled() {
    let (_pg, app) = app(false, false).await;
    let (status, _b) = send(&app, get("/ferroehr/rest/.well-known/smart-configuration")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "zero wire drift when off");
}

#[tokio::test]
async fn permission_capability_only_in_fail_closed_mode() {
    let (_pg, advisory) = app(true, false).await;
    let (_s, body) = send(
        &advisory,
        get("/ferroehr/rest/.well-known/smart-configuration"),
    )
    .await;
    let doc: Value = serde_json::from_str(&body).unwrap();
    let caps: Vec<&str> = doc["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        !caps.contains(&"openehr-permission-v1"),
        "advisory mode must not claim fine-grained enforcement: {caps:?}"
    );

    let (_pg2, strict) = app(true, true).await;
    let (_s, body) = send(
        &strict,
        get("/ferroehr/rest/.well-known/smart-configuration"),
    )
    .await;
    let doc: Value = serde_json::from_str(&body).unwrap();
    let caps: Vec<&str> = doc["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        caps.contains(&"openehr-permission-v1"),
        "fail-closed mode advertises the enforcement it performs: {caps:?}"
    );
}

// ── the template + AQL families enforce in fail-closed mode ─────────────────

#[tokio::test]
async fn fail_closed_denies_scopeless_template_access() {
    let (_pg, app) = app(true, true).await;
    // Auth is disabled, so the caller holds no SMART resource scopes at all —
    // fail-closed mode (master08 §Scopes ¶2: "The Platform must validate
    // requested scopes…") denies the template family.
    let (status, body) = send(&app, get(&format!("{BASE}/definition/template/adl1.4"))).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a scope-less caller is denied the template family: {body}"
    );
}

#[tokio::test]
async fn fail_closed_denies_scopeless_query_access() {
    let (_pg, app) = app(true, true).await;
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/query/aql"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"q": "SELECT e/ehr_id/value FROM EHR e"}"#))
        .unwrap();
    let (status, body) = send(&app, req).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a scope-less caller is denied the AQL family: {body}"
    );
}

#[tokio::test]
async fn advisory_mode_defers_for_scopeless_callers() {
    let (_pg, app) = app(true, false).await;
    // Advisory mode: no SMART scope for the family → the gate defers to
    // RBAC/ABAC; the list serves normally.
    let (status, body) = send(&app, get(&format!("{BASE}/definition/template/adl1.4"))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// With no authenticated principal there is no token, and therefore no SMART
/// scopes — a state the configuration already answers. Advisory mode defers to
/// the RBAC/ABAC tiers; fail-closed mode refuses.
///
/// This is the distinction that keeps `auth.enabled = false` usable: there the
/// absence of a principal is the operator's explicit choice, not a missing
/// credential, so a gate that refused unconditionally would brick the
/// development posture.
#[tokio::test]
async fn scopeless_caller_defers_in_advisory_mode_and_is_refused_when_required() {
    let path = format!("{BASE}/definition/template/adl1.4");

    let (_pg, advisory) = app(true, false).await;
    let (status, body) = send(&advisory, get(&path)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "advisory mode must defer for a scopeless caller: {body}"
    );

    let (_pg2, fail_closed) = app(true, true).await;
    let (status, body) = send(&fail_closed, get(&path)).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "fail-closed mode must refuse a caller carrying no SMART scopes: {body}"
    );
}

#[tokio::test]
async fn smart_disabled_leaves_families_ungated() {
    let (_pg, app) = app(false, false).await;
    let (status, body) = send(&app, get(&format!("{BASE}/definition/template/adl1.4"))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}
