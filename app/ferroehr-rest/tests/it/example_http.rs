// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the template-example endpoint
//! (`GET /definition/template/adl1.4/{template_id}/example`, a Release-1.1.0
//! operation — ITS-REST overview `Amendment_record` §Release-1.1.0, SPECITS-58).
//!
//! The Demo Vitals OPT is uploaded through the real wire
//! (`POST /definition/template/adl1.4`, `application/xml`); the real service
//! then generates the example COMPOSITION from the stored `WebTemplate`. The
//! assembled router is driven for each supported `Accept`:
//!
//! * default / `application/json` → canonical JSON COMPOSITION;
//! * `application/xml` → canonical XML;
//! * `application/openehr.wt.flat+json` / `…wt.structured+json` → the FLAT /
//!   STRUCTURED converters;
//! * an unsupported `Accept` → `406`;
//! * an unknown `template_id` → `404`;
//! * an invalid `detail_level` → `400`.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
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
const TEMPLATE_ID: &str = "Demo Vitals";
const JSON_MIME: &str = "application/json";
const XML_MIME: &str = "application/xml";
const FLAT_MIME: &str = "application/openehr.wt.flat+json";
const STRUCTURED_MIME: &str = "application/openehr.wt.structured+json";

fn flat_crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/openehr-sdt")
}

fn opt_xml() -> String {
    std::fs::read_to_string(flat_crate_dir().join("tests/fixtures/better/Demo Vitals.opt"))
        .expect("Demo Vitals.opt vendored in openehr-sdt")
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

/// A router over a fresh real service with the Demo Vitals OPT already uploaded.
async fn app_with_template() -> (testkit::TestDb, Router) {
    let (pg, service) = common::test_service().await;
    let app = common::router_with(config(), service);
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/definition/template/adl1.4"))
        .header(header::CONTENT_TYPE, "application/xml")
        .body(Body::from(opt_xml()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.expect("upload OPT");
    assert_eq!(
        resp.status(),
        StatusCode::CREATED,
        "Demo Vitals OPT uploads through the real wire"
    );
    (pg, app)
}

async fn get(
    app: &Router,
    uri: &str,
    accept: Option<&str>,
) -> (StatusCode, Option<String>, String) {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(a) = accept {
        builder = builder.header(header::ACCEPT, a);
    }
    let resp = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .expect("response");
    let status = resp.status();
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    (
        status,
        content_type,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn example_uri() -> String {
    // The template id carries a space; percent-encode it in the path segment.
    let id = TEMPLATE_ID.replace(' ', "%20");
    format!("{BASE}/definition/template/adl1.4/{id}/example")
}

#[tokio::test]
async fn example_default_is_canonical_json() {
    let (_pg, app) = app_with_template().await;
    let (status, content_type, body) = get(&app, &example_uri(), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type.as_deref(), Some(JSON_MIME));
    let comp: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        comp.get("_type").and_then(Value::as_str),
        Some("COMPOSITION")
    );
}

#[tokio::test]
async fn example_as_canonical_xml() {
    let (_pg, app) = app_with_template().await;
    let (status, content_type, body) = get(&app, &example_uri(), Some(XML_MIME)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type.as_deref(), Some(XML_MIME));
    assert!(body.contains("<composition"), "canonical XML root: {body}");
}

#[tokio::test]
async fn example_as_flat() {
    let (_pg, app) = app_with_template().await;
    let (status, content_type, body) = get(&app, &example_uri(), Some(FLAT_MIME)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type.as_deref(), Some(FLAT_MIME));
    let flat: serde_json::Map<String, Value> = serde_json::from_str(&body).unwrap();
    assert!(
        flat.contains_key("ctx/language"),
        "flat has ctx keys: {body}"
    );
}

#[tokio::test]
async fn example_as_structured() {
    let (_pg, app) = app_with_template().await;
    let (status, content_type, _body) = get(&app, &example_uri(), Some(STRUCTURED_MIME)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type.as_deref(), Some(STRUCTURED_MIME));
}

#[tokio::test]
async fn example_unsupported_accept_is_406() {
    let (_pg, app) = app_with_template().await;
    let (status, _content_type, _body) = get(&app, &example_uri(), Some("application/pdf")).await;
    assert_eq!(status, StatusCode::NOT_ACCEPTABLE);
}

#[tokio::test]
async fn example_unknown_template_is_404() {
    let (_pg, app) = app_with_template().await;
    let uri = format!("{BASE}/definition/template/adl1.4/nope.v0/example");
    let (status, _content_type, _body) = get(&app, &uri, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn example_invalid_detail_level_is_400() {
    let (_pg, app) = app_with_template().await;
    let uri = format!("{}?detail_level=exhaustive", example_uri());
    let (status, _content_type, _body) = get(&app, &uri, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// A present-but-empty value is out of the closed enums, not the default: the
// declared defaults apply to an ABSENT parameter only (ITS-REST
// parameters/query/example_detail_level.yaml, example_type.yaml).
#[tokio::test]
async fn example_empty_query_values_are_400() {
    let (_pg, app) = app_with_template().await;
    for query in ["?detail_level=", "?type="] {
        let uri = format!("{}{query}", example_uri());
        let (status, _content_type, _body) = get(&app, &uri, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query} must refuse");
    }
}
