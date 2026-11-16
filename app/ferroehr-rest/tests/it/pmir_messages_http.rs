// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The operator routes over the refused ITI-93 messages through the
//! assembled router (#5725): only a caller in the Admin class lists, retries
//! or settles, the routes answer `405` while the admin API is off, and every
//! act an administrator makes is an access record naming the administrator
//! and the message. The routes are our own design (PMIR 1.6.0
//! §2:3.93.4.1.3 leaves the Consumer's handling to its capabilities).

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use ferroehr::config::authz::AuthzConfig;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::identity_feed::apply::Applied;
use ferroehr::service::demographic::identity_feed::{FeedSender, Refusal};
use ferroehr::system_log::sender::{AuditHandle, start};
use ferroehr_rest::extensions::access::authz::AuthzHandle;
use ferroehr_rest::extensions::management::Observability;
use fhir_types::r4::patient::{Patient, PatientLink};
use fhir_types::r4::reference::Reference;
use http::{Request, StatusCode, header};
use ihe_iti::pmir::server::notification::{self, Addressing, Notice};
use serde_json::{Value, json};
use sqlx::PgPool;
use url::Url;

use crate::common;

const FEED: &str = "https://cdr.example.com/pmir/feed";
const PIX_A: &str = "0190a1bc-3a1e-72e4-b7c5-f78a24c0a001";
const PIX_B: &str = "0190a1bc-3a1e-72e4-b7c5-f78a24c0b002";
/// The message that merges B into A after A was merged into B, refused.
const REFUSED: &str = "m-into-merged";

/// The deprecated `Patient` `id`, merged into `survivor` (PMIR 1.6.0
/// §2:3.93.4.1.2.4).
fn merged(id: &str, survivor: &str) -> Patient {
    Patient {
        id: Some(id.to_owned()),
        active: Some(false.into()),
        link: vec![PatientLink {
            other: Reference {
                reference: Some(format!("Patient/{survivor}").as_str().into()),
                ..Reference::default()
            },
            r#type: "replaced-by".into(),
            ..PatientLink::default()
        }],
        ..Patient::default()
    }
}

/// Stores the ITI-93 message `id` merging `from` into `into` and applies it.
async fn merge(svc: &FerroEhrService, id: &str, from: &str, into: &str) -> Option<Applied> {
    let registry = Url::parse("https://pix.example/fhir").expect("a URL");
    let destination = Url::parse(FEED).expect("a URL");
    let history = format!("{id}-history");
    let body = serde_json::to_vec(&notification::message(
        &[Notice::Merged(Box::new(merged(from, into)))],
        &Addressing {
            message_id: id,
            history_id: &history,
            registry: &registry,
            destination: &destination,
        },
    ))
    .expect("the message writes");
    let sender = FeedSender {
        issuer: "https://as.example".to_owned(),
        client_id: "ferropix".to_owned(),
    };
    svc.pmir_receive(&sender, Some("application/fhir+json"), &body, FEED)
        .await
        .expect("the message is stored");
    Box::pin(svc.pmir_apply_next(uuid::Uuid::now_v7()))
        .await
        .expect("the apply runs")
}

/// An audited app with RBAC on, an administrator and a clinician, holding
/// one refused message, [`REFUSED`].
async fn app(admin: bool) -> (testkit::TestDb, PgPool, Arc<FerroEhrService>, Router) {
    let (db, pool) = common::migrated_pool().await;
    let (sender, _handle): (_, AuditHandle) =
        start(common::local_audit(), None, Some(pool.clone()))
            .await
            .expect("the audit sender");
    let svc = common::binding_service(&pool).with_audit(sender);
    assert!(matches!(
        merge(&svc, "m-merge", PIX_A, PIX_B).await,
        Some(Applied::Applied(_))
    ));
    assert_eq!(
        merge(&svc, REFUSED, PIX_B, PIX_A).await,
        Some(Applied::Refused(REFUSED.to_owned(), Refusal::Survivor))
    );
    let mut config = common::api_config(admin);
    common::with_basic_users(
        &mut config,
        &svc,
        &pool,
        &[("root", &["ADMIN"]), ("bob", &["USER"])],
    )
    .await;
    let mut authz = AuthzConfig::default();
    authz.rbac.enabled = true;
    let authz = AuthzHandle::build(&authz, &config.api.base_path).map(Arc::new);
    let svc = Arc::new(svc);
    let app = common::build_full(config, Arc::clone(&svc), authz, Observability::default()).await;
    (db, pool, svc, app)
}

/// Sends `method path` as `user`, or anonymously, with an optional JSON body.
async fn send(
    app: &Router,
    method: &str,
    path: &str,
    user: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, String) {
    let mut req = Request::builder()
        .method(method)
        .uri(common::full(path))
        .header(header::ACCEPT, "application/json");
    if let Some(user) = user {
        req = req.header(header::AUTHORIZATION, common::basic_credential(user));
    }
    let req = match body {
        Some(body) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string())),
        None => req.body(Body::empty()),
    }
    .expect("request");
    common::send_body(app, req).await
}

fn retry_path(id: &str) -> String {
    format!("/admin/pmir/messages/{id}/retry")
}

fn settle_path(id: &str) -> String {
    format!("/admin/pmir/messages/{id}/settle")
}

/// The `(action, principal)` of every stored access record on `resource`,
/// polled until `expected` have landed.
async fn trail(pool: &PgPool, resource: &str, expected: usize) -> Vec<(String, String)> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT action, principal FROM audit.access_trail \
             WHERE resource_id = $1 ORDER BY chain_seq",
        )
        .bind(resource)
        .fetch_all(pool)
        .await
        .expect("read the trail");
        if rows.len() >= expected {
            return rows
                .into_iter()
                .map(|(action, principal)| (action, principal.unwrap_or_default()))
                .collect();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "only {} of {expected} records on {resource} landed",
            rows.len()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn only_an_administrator_lists_retries_or_settles() {
    let (_db, _pool, _svc, app) = Box::pin(app(true)).await;
    let settlement = json!({ "decision": "discard", "reason": "a test" });
    assert_eq!(
        send(&app, "GET", "/admin/pmir/messages", None, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    for (method, path, body) in [
        ("GET", "/admin/pmir/messages".to_owned(), None),
        ("POST", retry_path(REFUSED), None),
        ("POST", settle_path(REFUSED), Some(settlement.clone())),
    ] {
        let (status, answer) = send(&app, method, &path, Some("bob"), body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {answer}");
    }
    let (status, answer) = send(&app, "GET", "/admin/pmir/messages", Some("root"), None).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let listed: Value = serde_json::from_str(&answer).expect("a JSON answer");
    assert_eq!(listed["messages"][0]["message_id"], json!(REFUSED));
    assert_eq!(
        listed["messages"][0]["reason"],
        json!(Refusal::Survivor.code())
    );
}

#[tokio::test]
async fn the_routes_answer_405_while_the_admin_api_is_off() {
    let (_db, _pool, _svc, app) = Box::pin(app(false)).await;
    let (status, answer) = send(&app, "GET", "/admin/pmir/messages", Some("root"), None).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{answer}");
}

#[tokio::test]
async fn an_administrator_retries_and_settles_and_each_act_is_recorded() {
    let (_db, pool, svc, app) = Box::pin(app(true)).await;
    assert_eq!(
        send(&app, "GET", "/admin/pmir/messages", Some("root"), None)
            .await
            .0,
        StatusCode::OK
    );
    let (status, answer) = send(&app, "POST", &retry_path(REFUSED), Some("root"), None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{answer}");
    assert_eq!(
        Box::pin(svc.pmir_apply_next(uuid::Uuid::now_v7()))
            .await
            .expect("the apply runs"),
        Some(Applied::Refused(REFUSED.to_owned(), Refusal::Survivor)),
        "the retried message applies again and is refused again"
    );

    let (status, answer) = send(
        &app,
        "POST",
        &settle_path(REFUSED),
        Some("root"),
        Some(json!({ "decision": "apply", "reason": "no" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    let (status, answer) = send(
        &app,
        "POST",
        &settle_path("no-such"),
        Some("root"),
        Some(json!({ "decision": "discard", "reason": "gone" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{answer}");
    let (status, answer) = send(
        &app,
        "POST",
        &settle_path(REFUSED),
        Some("root"),
        Some(json!({ "decision": "discard", "reason": "the Registry is corrected" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let settled: Value = serde_json::from_str(&answer).expect("a JSON answer");
    assert_eq!(settled["decision"], json!("discard"));
    let (status, answer) = send(&app, "POST", &retry_path(REFUSED), Some("root"), None).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a settled message is not retried: {answer}"
    );

    assert_eq!(
        trail(&pool, &format!("pmir-message:{REFUSED}"), 3).await,
        [
            ("R".to_owned(), "root".to_owned()),
            ("U".to_owned(), "root".to_owned()),
            ("U".to_owned(), "root".to_owned()),
        ]
    );
}
