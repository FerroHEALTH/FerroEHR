// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the event-subscription admin extension API group:
//! the config gate (`AppConfig::events_admin_api`), the `200`/`201`/`204`/`404`/
//! `400`/`409` wire outcomes for the CRUD verbs, and the JSON body shapes —
//! driven through the assembled router over the **real** `FerroEhrService` on a
//! real Postgres database (the scripted `Mock` is gone; the CRUD
//! persists to the real `event_subscription` table).
//!
//! Design: event/subscription semantics are spec-silent; the surface is our own,
//! config-gated like the terminology group, mounted under `/admin/` and
//! dispatching to the `EventSubscriptionAdapter` extension.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use ferroehr::config::auth::AuthConfig;
use ferroehr::config::server::ServerConfig;
use ferroehr_rest::config::AppConfig;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";
const GROUP: &str = "/ferroehr/rest/openehr/v1/admin/event_subscription";

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
        events_admin_api: enabled,
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

async fn send(app: Router, req: Request<Body>) -> (StatusCode, Value) {
    let resp = app.oneshot(req).await.expect("response");
    let status = resp.status();
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, body)
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

#[tokio::test]
async fn disabled_group_is_404() {
    let (_pg, a) = app(false).await;
    let (status, _) = send(a, req("GET", GROUP, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn crud_round_trip() {
    let (_pg, app) = app(true).await;

    // Empty list (no seed row for subscriptions).
    let (status, body) = send(app.clone(), req("GET", GROUP, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 0);

    // Create — 201 with the stored record + a generated id.
    let create_body =
        json!({ "name": "vitals", "kind": "COMPOSITION", "template_id": "vitals.v2" });
    let (status, created) = send(app.clone(), req("POST", GROUP, Some(create_body))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["name"], "vitals");
    assert_eq!(created["kind"], "COMPOSITION");
    assert_eq!(created["template_id"], "vitals.v2");
    // Unset predicates are wildcards (null).
    assert_eq!(created["change_type"], Value::Null);
    assert_eq!(created["enabled"], true);
    let id = created["id"].as_str().unwrap().to_owned();

    // Get by id — 200.
    let (status, got) = send(app.clone(), req("GET", &format!("{GROUP}/{id}"), None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got["id"], id);

    // Update — 200, predicates replaced, name immutable.
    let update_body = json!({ "name": "ignored", "kind": "EHR_STATUS", "enabled": false });
    let (status, updated) = send(
        app.clone(),
        req("PUT", &format!("{GROUP}/{id}"), Some(update_body)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["name"], "vitals", "name is immutable");
    assert_eq!(updated["kind"], "EHR_STATUS");
    assert_eq!(updated["enabled"], false);

    // List now has one.
    let (_status, list) = send(app.clone(), req("GET", GROUP, None)).await;
    assert_eq!(list.as_array().unwrap().len(), 1);

    // Delete — 204.
    let (status, _) = send(app.clone(), req("DELETE", &format!("{GROUP}/{id}"), None)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Gone — 404.
    let (status, _) = send(app, req("GET", &format!("{GROUP}/{id}"), None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_without_name_is_400() {
    let (_pg, a) = app(true).await;
    let (status, _) = send(
        a,
        req("POST", GROUP, Some(json!({ "kind": "COMPOSITION" }))),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn duplicate_name_is_409() {
    let (_pg, app) = app(true).await;
    let body = json!({ "name": "dup" });
    let (status, _) = send(app.clone(), req("POST", GROUP, Some(body.clone()))).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = send(app, req("POST", GROUP, Some(body))).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn unknown_id_is_404() {
    let (_pg, app) = app(true).await;
    let id = Uuid::now_v7();
    let (status, _) = send(app.clone(), req("GET", &format!("{GROUP}/{id}"), None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(app, req("DELETE", &format!("{GROUP}/{id}"), None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn malformed_id_is_400() {
    let (_pg, a) = app(true).await;
    let (status, _) = send(a, req("GET", &format!("{GROUP}/not-a-uuid"), None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn enabled_group_lists_real_store() {
    // Re-targeted from the old `unhooked → 501` Mock-scaffolding case: with the
    // concrete service the CRUD persists to the real store, so an enabled group
    // answers 200 (never the trait-default 501).
    let (_pg, a) = app(true).await;
    let (status, body) = send(a, req("GET", GROUP, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_array());
}

/// The update is a full replace whose intent must be complete (#2598): a PUT
/// omitting `enabled` is refused — measured pre-fix, it silently re-enabled a
/// deliberately disabled subscription — and the disabled state survives.
#[tokio::test]
async fn a_put_omitting_enabled_is_refused_and_cannot_reenable() {
    let (_pg, a) = app(true).await;
    let (status, created) = send(
        a.clone(),
        req(
            "POST",
            GROUP,
            Some(json!({ "name": "replace-honesty", "enabled": true })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().expect("id").to_owned();

    // Disable explicitly.
    let (status, disabled) = send(
        a.clone(),
        req(
            "PUT",
            &format!("{GROUP}/{id}"),
            Some(json!({ "enabled": false })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{disabled}");
    assert_eq!(disabled["enabled"], false);

    // A partial replace omitting `enabled` is the caller's incomplete intent.
    let (status, refused) = send(
        a.clone(),
        req(
            "PUT",
            &format!("{GROUP}/{id}"),
            Some(json!({ "kind": "COMPOSITION" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|m| m.contains("enabled")),
        "the refusal names the field: {refused}"
    );

    // The deliberate disable survived the refused replace.
    let (_s, read) = send(a, req("GET", &format!("{GROUP}/{id}"), None)).await;
    assert_eq!(read["enabled"], false, "{read}");
}

/// An unknown body key is refused loudly (#2599's removal follow-through): a
/// client still sending the removed `archetype` predicate gets a 400, never a
/// silent drop — while echoed read-only members from a prior GET stay
/// tolerated (the crud round trip pins that).
#[tokio::test]
async fn an_unknown_predicate_key_is_refused_never_dropped() {
    let (_pg, a) = app(true).await;
    let (status, body) = send(
        a,
        req(
            "POST",
            GROUP,
            Some(json!({ "name": "stale-client", "archetype": "openEHR-EHR-COMPOSITION.x.v1" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|m| m.contains("archetype")),
        "the refusal names the unknown key: {body}"
    );
}
