// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the terminology extension API group: the config gate
//! (`AppConfig::terminology_api_enabled`), the `200`/`404`/`400` wire outcomes
//! for `get_terminology_ids` / `get_terminology_description` / `get_term` /
//! `subsumes` / `get_value_set` / `value_set_validate`, and the JSON body shapes
//! — driven through the assembled router over the **real** `FerroEhrService`
//! (the scripted `Mock` is gone). The terminology service is the
//! in-process openEHR bundle (TERM 3.1.0), so no DB seeding is needed and every
//! assertion observes the real bundle.
//!
//! An unknown terminology, term or value set surfaces as
//! `versioned_object_does_not_exist` (the
//! bundle provider's convention), which the adapter maps to HTTP `404`.
#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use axum::Router;
use axum::body::Body;
use base64::Engine;
use http::{Request, StatusCode};
use serde_json::Value;

use ferroehr::config::rest::RestConfig;
use ferroehr_rest::config::AppConfig;
use ferrohealth_smart::config::AuthSection;

use crate::common;
use crate::common::EXT;
use crate::common::{get, send_body};

const BASE: &str = "/ferroehr/rest/openehr/v1";
/// A terminology the bundle knows; anything else is `versioned_object_does_not_exist`.
const KNOWN: &str = "openehr";
const UNKNOWN: &str = "no-such-terminology";
/// The canonical openEHR bundle URI (`OPENEHR_TERM_URI`), returned by the real
/// terminology description.
const OPENEHR_TERM_URI: &str = "https://github.com/openEHR/terminology";

fn config(terminology_enabled: bool) -> AppConfig {
    AppConfig {
        server: common::server(),
        rest: RestConfig {
            base_path: BASE.to_owned(),
            swagger_ui: ferroehr::config::management::AccessLevel::Off,
            cors_permissive: false,
            ..Default::default()
        },
        auth: AuthSection {
            enabled: false,
            basic: None,
            ..AuthSection::default()
        },
        terminology_api_enabled: terminology_enabled,
        ..Default::default()
    }
}

/// The assembled router over the real service with the terminology group toggled.
async fn app(terminology_enabled: bool) -> (testkit::TestDb, Router) {
    let (pg, service) = common::test_service().await;
    (
        pg,
        ferroehr_rest::build_with(config(terminology_enabled), service)
            .await
            .expect("router builds"),
    )
}

#[tokio::test]
async fn disabled_terminology_is_404() {
    // Gate off → 404 before any backend work (the config gate short-circuits;
    // the real bundle is never consulted).
    let (_pg, app) = app(false).await;
    let (status, _) = send_body(&app, get(&format!("{EXT}/terminology"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// With authentication ENABLED, an unauthenticated caller hitting a **disabled**
/// in-API extension group is answered `401`, not the `404` the group's own gate
/// would produce: the group gate is in-handler, and the authentication layer
/// wraps the whole API subtree, so authn runs first. That ordering is the
/// ITS-REST discipline — an unauthenticated request is `401`
/// (`docs/specs/openehr/ITS-REST/specifications/docs/overview/
/// Requests_and_responses.md` §Authentication and authorization) — and it also
/// keeps the group's enabled/disabled state from leaking to an anonymous
/// prober. The served declarations document exactly this (each disabled-group
/// `404` says a `401` comes first); this test pins the wire so document and
/// behaviour cannot drift apart.
#[tokio::test]
async fn disabled_group_answers_401_before_404_when_unauthenticated() {
    let mut cfg = config(false);
    common::with_basic_users(&mut cfg, &[("alice", &["USER"])]);
    let (_pg, service) = common::test_service().await;
    let app = ferroehr_rest::build_with(cfg, service)
        .await
        .expect("router builds");

    let (status, _) = send_body(&app, get(&format!("{EXT}/terminology"))).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "authentication runs before the in-handler group gate, so an \
         unauthenticated request to a disabled group is 401, never 404"
    );
}

/// An AUTHENTICATED caller gets the group gate's own `404` — the disabled-group
/// outcome the declarations document, once authentication is satisfied.
#[tokio::test]
async fn disabled_group_answers_404_when_authenticated() {
    let mut cfg = config(false);
    common::with_basic_users(&mut cfg, &[("alice", &["USER"])]);
    let (_pg, service) = common::test_service().await;
    let app = ferroehr_rest::build_with(cfg, service)
        .await
        .expect("router builds");

    let req = Request::builder()
        .method("GET")
        .uri(format!("{EXT}/terminology"))
        .header(
            http::header::AUTHORIZATION,
            format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode("alice:pw")
            ),
        )
        .body(Body::empty())
        .unwrap();
    let (status, _) = send_body(&app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn terminology_ids_returns_list() {
    let (_pg, app) = app(true).await;
    let (status, body) = send_body(&app, get(&format!("{EXT}/terminology"))).await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).expect("json body");
    let ids = v["terminology_ids"].as_array().expect("terminology_ids");
    // The real bundle exposes "openehr" plus the external code sets.
    assert!(ids.iter().any(|id| id == KNOWN), "ids contain openehr");
    assert!(
        ids.iter().any(|id| id == "ISO_639-1"),
        "ids contain ISO_639-1"
    );
}

#[tokio::test]
async fn terminology_description_found_is_200() {
    let (_pg, app) = app(true).await;
    let (status, body) = send_body(&app, get(&format!("{EXT}/terminology/{KNOWN}"))).await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).expect("json body");
    // Real bundle values (was Mock-scripted "openEHR"/"http://openehr.org/...").
    assert_eq!(v["publisher"], "openEHR Foundation");
    assert_eq!(v["uri"], OPENEHR_TERM_URI);
}

#[tokio::test]
async fn terminology_description_unknown_maps_to_404() {
    let (_pg, app) = app(true).await;
    // The bundle's `versioned_object_does_not_exist` surfaces as HTTP 404.
    let (status, _) = send_body(&app, get(&format!("{EXT}/terminology/{UNKNOWN}"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn get_term_lookup_returns_extract() {
    let (_pg, app) = app(true).await;
    // 249 is `creation` in the audit_change_type group (a real openEHR code).
    // `at_date` is accepted but a no-op for the single-version bundle.
    let (status, body) = send_body(
        &app,
        get(&format!(
            "{EXT}/terminology/{KNOWN}/term/249?at_date=2024-01-01"
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).expect("json body");
    assert_eq!(v["terminology_id"], KNOWN);
    assert_eq!(v["terms"]["249"]["text"], "creation");
}

#[tokio::test]
async fn subsumes_returns_bool() {
    let (_pg, app) = app(true).await;
    // The openEHR bundle is flat and `subsumes` is strict, so subsumption is
    // uniformly false — even the identity case (was Mock-scripted identity→true).
    let (status, body) = send_body(
        &app,
        get(&format!(
            "{EXT}/terminology/{KNOWN}/subsumes?ref_code=249&candidate=249"
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).expect("json body");
    assert_eq!(v["subsumes"], false);
}

#[tokio::test]
async fn subsumes_missing_required_query_is_400() {
    let (_pg, app) = app(true).await;
    // `candidate` absent → 400 before the backend is consulted.
    let (status, _) = send_body(
        &app,
        get(&format!("{EXT}/terminology/{KNOWN}/subsumes?ref_code=249")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn get_value_set_expand_returns_extract() {
    let (_pg, app) = app(true).await;
    // `audit_change_type` is a real openEHR group addressable as a value set.
    let (status, body) = send_body(
        &app,
        get(&format!(
            "{EXT}/terminology/{KNOWN}/value_set/audit_change_type"
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).expect("json body");
    assert_eq!(v["terminology_id"], KNOWN);
    assert!(v["terms"].get("249").is_some());
}

#[tokio::test]
async fn get_value_set_unknown_maps_to_404() {
    let (_pg, app) = app(true).await;
    let (status, _) = send_body(
        &app,
        get(&format!(
            "{EXT}/terminology/{KNOWN}/value_set/no_such_group"
        )),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn value_set_validate_returns_valid() {
    let (_pg, app) = app(true).await;
    // 249 is a member of audit_change_type → valid.
    let (status, body) = send_body(
        &app,
        get(&format!(
            "{EXT}/terminology/{KNOWN}/value_set/audit_change_type/validate?candidate_code=249"
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).expect("json body");
    assert_eq!(v["valid"], true);
}

#[tokio::test]
async fn value_set_validate_missing_candidate_is_400() {
    let (_pg, app) = app(true).await;
    let (status, _) = send_body(
        &app,
        get(&format!(
            "{EXT}/terminology/{KNOWN}/value_set/audit_change_type/validate"
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn enabled_terminology_group_serves_real_bundle() {
    // Re-targeted from the old `unhooked → 501` Mock-scaffolding case: with the
    // concrete service the terminology group is always implemented, so an enabled
    // group serves the real bundle (200), never the trait-default 501.
    let (_pg, app) = app(true).await;
    let (status, _) = send_body(&app, get(&format!("{EXT}/terminology"))).await;
    assert_eq!(status, StatusCode::OK);
}

/// The router over a service whose terminology server is the mock at `base`.
async fn app_with_server(base: &str) -> (testkit::TestDb, Router) {
    let cfg = ferroehr::service::terminology::config::TerminologyServerConfig {
        enabled: true,
        url: ferrohealth_secret::secret_url::SecretUrl::new(base),
        request_timeout_ms: 800,
        ..Default::default()
    };
    let provider = ferroehr::service::terminology::fhir::FhirTerminologyProvider::new(&cfg)
        .expect("build provider");
    let pg = common::test_db().await;
    let service = ferroehr::service::FerroEhrService::new(
        &ferroehr::db::domain::DomainPools::from_shared(&pg.pool()),
    )
    .with_terminology_server(std::sync::Arc::new(provider));
    let app = ferroehr_rest::build_with(config(true), std::sync::Arc::new(service))
        .await
        .expect("router builds");
    (pg, app)
}

/// A value set the terminology server does not know is a `404`, and a server
/// fault is a `500` whose body names nothing of the upstream answer.
#[tokio::test]
async fn terminology_server_refusals_keep_their_wire_status() {
    use wiremock::matchers::path;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(path("/ValueSet/$expand"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let (_pg, app) = app_with_server(&server.uri()).await;
    let (status, body) = send_body(
        &app,
        get(&format!(
            "{EXT}/terminology/urn:test:system/value_set/urn:test:vs"
        )),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let server = MockServer::start().await;
    Mock::given(path("/ValueSet/$expand"))
        .respond_with(ResponseTemplate::new(500).set_body_string("upstream-secret-detail"))
        .mount(&server)
        .await;
    let (_pg, app) = app_with_server(&server.uri()).await;
    let (status, body) = send_body(
        &app,
        get(&format!(
            "{EXT}/terminology/urn:test:system/value_set/urn:test:vs"
        )),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(!body.contains("upstream-secret-detail"), "{body}");
}
