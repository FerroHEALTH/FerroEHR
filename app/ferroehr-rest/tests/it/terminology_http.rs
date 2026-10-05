// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the terminology extension API group (SM
//! `I_TERMINOLOGY_SERVICE`): the config gate
//! (`AppConfig::terminology_api_enabled`), the `200`/`404`/`400` wire outcomes
//! for `get_terminology_ids` / `get_terminology_description` / `get_term` /
//! `subsumes` / `get_value_set` / `value_set_validate`, and the JSON body shapes
//! — driven through the assembled router over the **real** `FerroEhrService`
//! (the scripted `Mock` is gone). The terminology service is the
//! in-process openEHR bundle (TERM 3.1.0), so no DB seeding is needed and every
//! assertion observes the real bundle.
//!
//! Spec grounding: `docs/specs/openehr/SM/docs/UML/classes/
//! i_terminology_service.adoc` (the nine calls + `Pre_has_*` preconditions).
//! A failed precondition surfaces as `versioned_object_does_not_exist` (the
//! bundle provider's convention), which the adapter maps to HTTP `404`.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use argon2::Argon2;
use argon2::password_hash::PasswordHasher;
use axum::Router;
use axum::body::Body;
use base64::Engine;
use http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use ferroehr::config::auth::{AuthConfig, BasicConfig, BasicUser};
use ferroehr::config::server::ServerConfig;
use ferroehr_rest::config::AppConfig;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";
/// A terminology the bundle knows; anything else is `versioned_object_does_not_exist`.
const KNOWN: &str = "openehr";
const UNKNOWN: &str = "no-such-terminology";
/// The canonical openEHR bundle URI (`OPENEHR_TERM_URI`), returned by the real
/// terminology description.
const OPENEHR_TERM_URI: &str = "https://github.com/openEHR/terminology";

fn config(terminology_enabled: bool) -> AppConfig {
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
        terminology_api_enabled: terminology_enabled,
        ..Default::default()
    }
}

/// The assembled router over the real service with the terminology group toggled.
async fn app(terminology_enabled: bool) -> (testkit::TestDb, Router) {
    let (pg, service) = common::test_service().await;
    (
        pg,
        ferroehr_rest::build_with(config(terminology_enabled), service).expect("router builds"),
    )
}

async fn send(app: Router, req: Request<Body>) -> (StatusCode, String) {
    let resp = app.oneshot(req).await.expect("response");
    let status = resp.status();
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn get(uri: String) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn disabled_terminology_is_404() {
    // Gate off → 404 before any backend work (the config gate short-circuits;
    // the real bundle is never consulted).
    let (_pg, app) = app(false).await;
    let (status, _) = send(app, get(format!("{BASE}/terminology"))).await;
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
    cfg.auth = AuthConfig {
        enabled: true,
        basic: Some(BasicConfig {
            users: vec![BasicUser {
                username: "alice".to_owned(),
                password_hash: ferroehr::config::secret::Secret::new(argon2_hash("pw")),
                password_hash_file: None,
                roles: vec!["USER".to_owned()],
            }],
        }),
        oidc: None,
        ..AuthConfig::default()
    };
    let (_pg, service) = common::test_service().await;
    let app = ferroehr_rest::build_with(cfg, service).expect("router builds");

    let (status, _) = send(app, get(format!("{BASE}/terminology"))).await;
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
    cfg.auth = AuthConfig {
        enabled: true,
        basic: Some(BasicConfig {
            users: vec![BasicUser {
                username: "alice".to_owned(),
                password_hash: ferroehr::config::secret::Secret::new(argon2_hash("pw")),
                password_hash_file: None,
                roles: vec!["USER".to_owned()],
            }],
        }),
        oidc: None,
        ..AuthConfig::default()
    };
    let (_pg, service) = common::test_service().await;
    let app = ferroehr_rest::build_with(cfg, service).expect("router builds");

    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/terminology"))
        .header(
            http::header::AUTHORIZATION,
            format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode("alice:pw")
            ),
        )
        .body(Body::empty())
        .unwrap();
    let (status, _) = send(app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Argon2 hash of a test password (the Basic-auth user store stores hashes).
fn argon2_hash(pw: &str) -> String {
    Argon2::default()
        .hash_password_with_salt(pw.as_bytes(), b"1234567890123456")
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn terminology_ids_returns_list() {
    let (_pg, app) = app(true).await;
    let (status, body) = send(app, get(format!("{BASE}/terminology"))).await;
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
    let (status, body) = send(app, get(format!("{BASE}/terminology/{KNOWN}"))).await;
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
    let (status, _) = send(app, get(format!("{BASE}/terminology/{UNKNOWN}"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn get_term_lookup_returns_extract() {
    let (_pg, app) = app(true).await;
    // 249 is `creation` in the audit_change_type group (a real openEHR code).
    // `at_date` is accepted but a no-op for the single-version bundle.
    let (status, body) = send(
        app,
        get(format!(
            "{BASE}/terminology/{KNOWN}/term/249?at_date=2024-01-01"
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
    let (status, body) = send(
        app,
        get(format!(
            "{BASE}/terminology/{KNOWN}/subsumes?ref_code=249&candidate=249"
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
    let (status, _) = send(
        app,
        get(format!("{BASE}/terminology/{KNOWN}/subsumes?ref_code=249")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn get_value_set_expand_returns_extract() {
    let (_pg, app) = app(true).await;
    // `audit_change_type` is a real openEHR group addressable as a value set.
    let (status, body) = send(
        app,
        get(format!(
            "{BASE}/terminology/{KNOWN}/value_set/audit_change_type"
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
    let (status, _) = send(
        app,
        get(format!(
            "{BASE}/terminology/{KNOWN}/value_set/no_such_group"
        )),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn value_set_validate_returns_valid() {
    let (_pg, app) = app(true).await;
    // 249 is a member of audit_change_type → valid.
    let (status, body) = send(
        app,
        get(format!(
            "{BASE}/terminology/{KNOWN}/value_set/audit_change_type/validate?candidate_code=249"
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
    let (status, _) = send(
        app,
        get(format!(
            "{BASE}/terminology/{KNOWN}/value_set/audit_change_type/validate"
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
    let (status, _) = send(app, get(format!("{BASE}/terminology"))).await;
    assert_eq!(status, StatusCode::OK);
}
