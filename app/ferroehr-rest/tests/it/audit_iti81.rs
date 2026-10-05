// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]
//! End-to-end tests of the RESTful-ATNA **ITI-81 Retrieve ATNA Audit Event**
//! surface (`GET /fhir/r4/AuditEvent`) over the real axum app + real
//! `FerroEhrService` + real `PostgreSQL` 18: the searchset Bundle over the
//! local Audit Record Repository, the supported filter subset, the
//! store-disabled `404`, and the RBAC admin gate (the node's security log is
//! an operator surface). No openEHR spec governs this — the retrieval
//! semantics are IHE's (`RESTful` ATNA supplement, ITI-81).

use std::sync::Arc;

use argon2::Argon2;
use argon2::password_hash::PasswordHasher;
use axum::Router;
use axum::body::Body;
use ferroehr::config::auth::{AuthConfig, BasicConfig, BasicUser};
use ferroehr::config::authz::AuthzConfig;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::event::{
    AuditEvent, EventActionCode, EventOutcome, EventType, ObjectClass,
};
use ferroehr::system_log::fhir;
use ferroehr::system_log::message::AuditContext;
use ferroehr::system_log::store::AuditStore;
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authz::{AuthzHandle, AuthzResolvers, ResolveError};
use http::{Request, StatusCode};
use jiff::Timestamp;
use serde_json::Value;
use tower::ServiceExt;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";

fn hash_pw(pw: &str) -> String {
    Argon2::default()
        .hash_password_with_salt(pw.as_bytes(), b"1234567890123456")
        .expect("hash")
        .to_string()
}

fn user(name: &str, roles: &[&str]) -> BasicUser {
    BasicUser {
        username: name.to_owned(),
        password_hash: ferroehr::config::secret::Secret::new(hash_pw("pw")),
        password_hash_file: None,
        roles: roles.iter().map(|r| (*r).to_owned()).collect(),
    }
}

fn rest_config() -> AppConfig {
    AppConfig {
        auth: AuthConfig {
            enabled: true,
            basic: Some(BasicConfig {
                users: vec![
                    user("root", &["ADMIN"]),
                    user("user", &["USER"]),
                    user("portal", &["PORTAL"]),
                ],
            }),
            ..AuthConfig::default()
        },
        ..Default::default()
    }
}

fn authz(enabled: bool) -> Option<Arc<AuthzHandle>> {
    let mut cfg = AuthzConfig::default();
    cfg.rbac.enabled = enabled;
    cfg.rbac.subject_audit_role = Some("PORTAL".to_owned());
    // RBAC-only: no engine, inert resolvers (nothing here consults ABAC).
    let resolvers = AuthzResolvers {
        subject: Arc::new(|_| Box::pin(async { Ok::<_, ResolveError>(None) })),
        template_of_version: Arc::new(|_, _| Box::pin(async { Ok::<_, ResolveError>(None) })),
    };
    AuthzHandle::build(&cfg, &rest_config().server.base_path, None, resolvers).map(Arc::new)
}

fn basic(name: &str) -> String {
    use base64::Engine;
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{name}:pw").as_bytes())
    )
}

fn ctx() -> AuditContext {
    AuditContext {
        source_id: "ferroehr".to_owned(),
        enterprise_site_id: "site-1".to_owned(),
        server_ip: "10.42.23.77".to_owned(),
        value_if_missing: "UNKNOWN".to_owned(),
    }
}

/// Seed one stored audit record directly through the store (the drain's
/// write path, minus the queue).
async fn seed(
    store: &AuditStore,
    at: &str,
    action: EventActionCode,
    principal: &str,
    patient: Option<&str>,
) {
    let mut e = AuditEvent::new(action, ObjectClass::Composition, EventOutcome::Success);
    principal.clone_into(&mut e.user_id);
    e.object_id = Some("8fa1::ferroehr::1".to_owned());
    e.event_type = Some(EventType::RestOperation("composition_get"));
    e.timestamp = at.parse::<Timestamp>().expect("ts");
    let rendered = fhir::to_fhir(&e, &ctx(), patient).expect("render");
    store.insert(&e, patient, &rendered).await.expect("seed");
}

/// The app + store over a fresh DB; `with_store` controls the ITI-81 gate.
async fn app(with_store: bool, rbac: bool) -> (testkit::TestDb, Router, AuditStore) {
    let (pg, pool) = common::migrated_pool().await;
    let store = AuditStore::new(pool.clone());
    let mut svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    if with_store {
        svc = svc.with_audit_store(store.clone());
    }
    let app = ferroehr_rest::build_full(
        rest_config(),
        Arc::new(svc),
        authz(rbac),
        ferroehr_rest::extensions::management::Observability::default(),
    )
    .expect("build app");
    (pg, app, store)
}

fn get(path: &str, auth: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(format!("{BASE}{path}"))
        .header("authorization", auth)
        .body(Body::empty())
        .expect("request")
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 1_048_576)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("json")
}

#[tokio::test]
async fn searchset_bundle_with_filters_and_paging() {
    let (_pg, app, store) = app(true, false).await;
    seed(
        &store,
        "2026-07-10T08:00:00Z",
        EventActionCode::Read,
        "alice",
        Some("patient-1"),
    )
    .await;
    seed(
        &store,
        "2026-07-12T09:00:00Z",
        EventActionCode::Create,
        "bob",
        Some("patient-2"),
    )
    .await;
    seed(
        &store,
        "2026-07-14T10:00:00Z",
        EventActionCode::Read,
        "alice",
        Some("patient-1"),
    )
    .await;

    // Unfiltered: all three, newest first.
    let resp = app
        .clone()
        .oneshot(get("/fhir/r4/AuditEvent", &basic("root")))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/fhir+json"
    );
    let bundle = body_json(resp).await;
    assert_eq!(bundle["resourceType"], "Bundle");
    assert_eq!(bundle["type"], "searchset");
    assert_eq!(bundle["total"], 3);
    assert_eq!(bundle["entry"][0]["resource"]["resourceType"], "AuditEvent");

    // patient + agent filters.
    let resp = app
        .clone()
        .oneshot(get(
            "/fhir/r4/AuditEvent?patient=patient-1&agent=alice",
            &basic("root"),
        ))
        .await
        .expect("resp");
    let bundle = body_json(resp).await;
    assert_eq!(bundle["total"], 2);

    // date range narrows to the middle record.
    let resp = app
        .clone()
        .oneshot(get(
            "/fhir/r4/AuditEvent?date=ge2026-07-11T00:00:00Z&date=le2026-07-13T00:00:00Z",
            &basic("root"),
        ))
        .await
        .expect("resp");
    let bundle = body_json(resp).await;
    assert_eq!(bundle["total"], 1);
    assert_eq!(bundle["entry"][0]["resource"]["action"], "C");

    // action filter + paging.
    let resp = app
        .clone()
        .oneshot(get(
            "/fhir/r4/AuditEvent?action=R&_count=1&_offset=1",
            &basic("root"),
        ))
        .await
        .expect("resp");
    let bundle = body_json(resp).await;
    assert_eq!(bundle["total"], 2, "total counts all matches");
    assert_eq!(
        bundle["entry"].as_array().map(Vec::len),
        Some(1),
        "one page entry"
    );

    // A malformed supported parameter is a 400 OperationOutcome.
    let resp = app
        .clone()
        .oneshot(get("/fhir/r4/AuditEvent?date=2026-07-11", &basic("root")))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let outcome = body_json(resp).await;
    assert_eq!(outcome["resourceType"], "OperationOutcome");
}

#[tokio::test]
async fn disabled_store_answers_404() {
    let (_pg, app, _store) = app(false, false).await;
    let resp = app
        .oneshot(get("/fhir/r4/AuditEvent", &basic("root")))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let outcome = body_json(resp).await;
    assert_eq!(outcome["resourceType"], "OperationOutcome");
}

#[tokio::test]
async fn rbac_gates_the_audit_surface_to_admins() {
    let (_pg, app, store) = app(true, true).await;
    seed(
        &store,
        "2026-07-10T08:00:00Z",
        EventActionCode::Read,
        "alice",
        Some("patient-1"),
    )
    .await;

    // A plain USER is forbidden (the audit trail is an operator surface).
    let resp = app
        .clone()
        .oneshot(get("/fhir/r4/AuditEvent", &basic("user")))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // An ADMIN retrieves.
    let resp = app
        .oneshot(get("/fhir/r4/AuditEvent", &basic("root")))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);
    let bundle = body_json(resp).await;
    assert_eq!(bundle["total"], 1);
}

/// The subject-scoped grant (#3240): a caller holding `subject_audit_role`
/// reads the access log for the one subject it names and nothing else — the
/// grant a patient portal holds instead of an admin credential over every
/// patient's log (GDPR Art. 15 with Recital 63, EHDS Art. 9).
#[tokio::test]
async fn the_subject_audit_role_reads_one_subjects_log_and_nothing_else() {
    let (_pg, app, store) = app(true, true).await;
    seed(
        &store,
        "2026-07-10T08:00:00Z",
        EventActionCode::Read,
        "alice",
        Some("patient-1"),
    )
    .await;
    seed(
        &store,
        "2026-07-10T08:01:00Z",
        EventActionCode::Read,
        "bob",
        Some("patient-2"),
    )
    .await;

    // Scoped to one subject: the portal reads that subject's records only.
    let resp = app
        .clone()
        .oneshot(get(
            "/fhir/r4/AuditEvent?patient=patient-1",
            &basic("portal"),
        ))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);
    let bundle = body_json(resp).await;
    assert_eq!(bundle["total"], 1, "{bundle}");
    assert_eq!(
        bundle["entry"][0]["resource"]["agent"][0]["who"]["identifier"]["value"],
        "alice"
    );

    // Unscoped: the same role is refused, with the reason naming the parameter.
    let resp = app
        .clone()
        .oneshot(get("/fhir/r4/AuditEvent", &basic("portal")))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let outcome = body_json(resp).await;
    assert!(
        outcome["issue"][0]["diagnostics"]
            .as_str()
            .is_some_and(|d| d.contains("`patient` parameter is required")),
        "{outcome}"
    );

    // A role that is neither admin nor the subject role is still refused, even
    // when it names a patient.
    let resp = app
        .clone()
        .oneshot(get("/fhir/r4/AuditEvent?patient=patient-1", &basic("user")))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // The admin keeps the unscoped retrieval.
    let resp = app
        .oneshot(get("/fhir/r4/AuditEvent", &basic("root")))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["total"], 2);
}
