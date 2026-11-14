// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The erasure of an access-log entry a law orders, end to end over the
//! assembled router (#5128).
//!
//! The tests assert that an administrator's erasure replaces the entry by a
//! tombstone naming the erasure's grounds, that the chain still verifies, that
//! the act is an access record naming the administrator, and that it is
//! refused without the admin role, for an unknown uid, for a tombstone and
//! without grounds. No openEHR specification defines an access log: our own
//! design.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use ferroehr::access_log::store::AccessLogStore;
use ferroehr::config::authz::AuthzConfig;
use ferroehr::system_log::sender::{AuditHandle, start};
use ferroehr::system_log::store::{AuditStore, VerifyScope};
use ferroehr_rest::extensions::access::authz::AuthzHandle;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode, header};
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common;

/// The uid of the entry the tests erase.
const UID: &str = "0193f1a2-7c1e-7b4a-9f00-2c6d1e8b5a01";

fn basic(name: &str) -> String {
    use base64::Engine;
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{name}:pw").as_bytes())
    )
}

/// A `DV_CODED_TEXT` of `terminology`.
fn coded(terminology: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "terminology_id": { "value": terminology }, "code_string": code } })
}

/// An access-log entry of a read of one composition, kept until 2031.
fn entry() -> Value {
    json!({
        "_type": "ACCESS_LOG_ENTRY",
        "uid": { "_type": "HIER_OBJECT_ID", "value": UID },
        "system_id": "cdr.example.org",
        "time_recorded": { "_type": "DV_DATE_TIME", "value": "2026-11-07T09:14:03.120Z" },
        "action": coded("openehr", "read"),
        "outcome": { "_type": "ACCESS_OUTCOME", "result": coded("openehr", "granted"),
            "delivered": 1 },
        "accessor": { "_type": "ACCESSOR", "kind": coded("openehr", "person"),
            "party": { "_type": "PARTY_IDENTIFIED", "name": "A. Clinician" } },
        "ehr_id": { "_type": "HIER_OBJECT_ID", "value": "7d44b88c-4199-4bad-97dc-d78268e01398" },
        "is_emergency": false,
        "retention": { "_type": "ACCESS_RETENTION",
            "retain_until": { "_type": "DV_DATE", "value": "2031-11-07" },
            "grounds": [coded("ehds-logging", "eu-2025-327-art-9-2")] },
        "link": { "_type": "ACCESS_LOG_LINK", "sequence": 1, "algorithm": "SHA-256",
            "previous_digest": "0".repeat(64), "content_digest": "0".repeat(64),
            "digest": "0".repeat(64) }
    })
}

/// An audited app with RBAC on, an administrator and a clinician, over an
/// access log that holds [`entry`].
async fn app() -> (testkit::TestDb, PgPool, Router) {
    let (db, pool) = common::migrated_pool().await;
    let (sender, _handle): (_, AuditHandle) =
        start(common::local_audit(), None, Some(pool.clone()))
            .await
            .expect("the audit sender");
    let svc = common::binding_service(&pool)
        .with_audit(sender)
        .with_audit_store(AuditStore::new(pool.clone()));
    AccessLogStore::new(pool.clone())
        .append(vec![
            serde_json::from_value(entry()).expect("the entry reads"),
        ])
        .await
        .expect("the entry appends");

    let mut config = common::api_config(true);
    common::with_basic_users(
        &mut config,
        &svc,
        &pool,
        &[("root", &["ADMIN"]), ("bob", &["USER"])],
    )
    .await;
    let mut authz = AuthzConfig::default();
    authz.rbac.enabled = true;
    let authz = AuthzHandle::build(&authz, &config.api.base_path, None).map(Arc::new);
    let app = common::build_full(config, Arc::new(svc), authz, Observability::default()).await;
    (db, pool, app)
}

/// `user` asks for the erasure that `body` describes.
async fn erase(app: &Router, user: &str, body: &Value) -> (StatusCode, String) {
    let req = Request::builder()
        .method("POST")
        .uri(common::full("/admin/access-log/erase"))
        .header(header::AUTHORIZATION, basic(user))
        .header(header::ACCEPT, "application/json")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    common::send_body(app, req).await
}

/// The grounds of an erasure a court orders.
fn ordered() -> Value {
    json!([coded("local", "court-order")])
}

/// The `(action, principal)` of the access records on `resource`, polled
/// until one has landed.
async fn trail(pool: &PgPool, resource: &str) -> Vec<(String, String)> {
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
        if !rows.is_empty() {
            return rows
                .into_iter()
                .map(|(action, principal)| (action, principal.unwrap_or_default()))
                .collect();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no record on {resource} landed"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The administrator's erasure answers the tombstone, which names the
/// erasure's grounds and keeps the entry's link, the chain still verifies,
/// and the act is an access record naming the administrator.
#[tokio::test]
async fn an_administrator_erases_an_entry_by_its_tombstone() {
    let (_db, pool, app) = Box::pin(app()).await;

    let (status, body) = erase(&app, "root", &json!({ "uid": UID, "grounds": ordered() })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let tombstone: Value = serde_json::from_str(&body).expect("a JSON answer");
    assert_eq!(
        tombstone["_type"],
        json!("ACCESS_LOG_TOMBSTONE"),
        "{tombstone}"
    );
    assert_eq!(tombstone["uid"]["value"], json!(UID));
    assert_eq!(
        tombstone["grounds"][0]["defining_code"]["code_string"],
        json!("court-order"),
        "{tombstone}"
    );
    assert!(tombstone.get("ehr_id").is_none(), "{tombstone}");

    assert!(matches!(
        AccessLogStore::new(pool.clone())
            .read(UID)
            .await
            .expect("read"),
        Some(AccessLogRecord::AccessLogTombstone(_))
    ));
    let trail = trail(&pool, &format!("access-log-erasure:{UID}")).await;
    assert_eq!(trail, [("D".to_owned(), "root".to_owned())]);
    let findings = AuditStore::new(pool.clone())
        .verify_chain_advancing(VerifyScope::Full)
        .await
        .expect("verify");
    assert!(findings.is_empty(), "{findings:?}");

    let (status, body) = erase(&app, "root", &json!({ "uid": UID, "grounds": ordered() })).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a tombstone is not erased again: {body}"
    );
}

/// The erasure is refused without the admin role, for an unknown uid, and
/// without grounds, and each refusal leaves the entry in place.
#[tokio::test]
async fn an_erasure_is_refused_without_the_role_a_record_or_grounds() {
    let (_db, pool, app) = Box::pin(app()).await;

    let (status, _) = erase(&app, "bob", &json!({ "uid": UID, "grounds": ordered() })).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let unknown = "0193f1a2-7c1e-7b4a-9f00-000000000000";
    let (status, _) = erase(
        &app,
        "root",
        &json!({ "uid": unknown, "grounds": ordered() }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for body in [
        json!({ "uid": UID }),
        json!({ "uid": UID, "grounds": [] }),
        json!({ "uid": UID, "grounds": ["court-order"] }),
        json!({ "grounds": ordered() }),
    ] {
        let (status, answer) = erase(&app, "root", &body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {answer}");
    }

    assert!(matches!(
        AccessLogStore::new(pool.clone())
            .read(UID)
            .await
            .expect("read"),
        Some(AccessLogRecord::AccessLogEntry(_))
    ));
}
