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
use ferroehr::system_log::config::{AuditConfig, FhirFeedConfig, SyslogConfig};
use ferroehr::system_log::sender::{AuditHandle, start};
use ferroehr::system_log::store::AuditStore;
use ferroehr::system_log::store::chains::VerifyScope;
use ferroehr_rest::extensions::access::authz::AuthzHandle;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode, header};
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;
use serde_json::{Value, json};
use sqlx::PgPool;
use testkit::syslog_tls::SyslogTlsListener;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
    app_auditing(common::local_audit()).await
}

/// [`app`] over the audit sinks `audit` configures.
async fn app_auditing(audit: AuditConfig) -> (testkit::TestDb, PgPool, Router) {
    let (db, pool) = common::migrated_pool().await;
    let (sender, _handle): (_, AuditHandle) = start(audit, None, Some(pool.clone()))
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
    let authz = AuthzHandle::build(&authz, &config.api.base_path).map(Arc::new);
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
             WHERE resource_id = $1 ORDER BY chain, chain_seq",
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

/// Wait until the record of a failed erasure has landed in `audit_event`.
async fn failed_erasure(pool: &PgPool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let (failed,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM audit.audit_event \
             WHERE operation = 'admin_access_log_erase' AND outcome <> 0",
        )
        .fetch_one(pool)
        .await
        .expect("read the failed erasures");
        if failed > 0 {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the record of the failed erasure never landed"
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
    let trail = trail(&pool, UID).await;
    assert_eq!(trail, [("D".to_owned(), "root".to_owned())]);
    let findings = AuditStore::new(pool.clone())
        .verify_chains_advancing(VerifyScope::Full)
        .await
        .expect("verify");
    assert!(findings.is_empty(), "{findings:?}");

    let (status, body) = erase(&app, "root", &json!({ "uid": UID, "grounds": ordered() })).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a tombstone is not erased again: {body}"
    );

    // The drain writes the queue in order, so once the refused second erasure
    // has landed every record of the first one has been stored or skipped.
    failed_erasure(&pool).await;
    let (entries, rows): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM audit.access_trail \
                  WHERE operation = 'admin_access_log_erase' AND chain = 'access_log' \
                    AND resource_id = $1), \
                (SELECT count(*) FROM audit.audit_event \
                  WHERE operation = 'admin_access_log_erase' AND outcome = 0)",
    )
    .bind(UID)
    .fetch_one(&pool)
    .await
    .expect("count the records of the erasure");
    assert_eq!(
        (entries, rows),
        (1, 0),
        "the erasure is stored once, as its access-log entry naming the erased entry"
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

/// The local store, a TLS syslog to `syslog` and a FHIR Feed to `arr`.
fn every_sink(syslog: &SyslogTlsListener, arr: &MockServer) -> AuditConfig {
    AuditConfig {
        syslog: Some(SyslogConfig {
            url: ferrohealth_secret::secret_url::SecretUrl::new(format!(
                "tls://127.0.0.1:{}",
                syslog.port
            )),
            tls: ferrohealth_config::tls::TlsClientSection {
                trust_roots_file: Some(syslog.ca_file.clone()),
                ..ferrohealth_config::tls::TlsClientSection::default()
            },
            ..SyslogConfig::default()
        }),
        fhir_feed: Some(FhirFeedConfig {
            endpoint: ferrohealth_smart::config::endpoint::EndpointSection {
                url: ferrohealth_secret::secret_url::SecretUrl::new(format!("{}/fhir", arr.uri())),
                ..ferrohealth_smart::config::endpoint::EndpointSection::default()
            },
            poll_interval: ferrohealth_config::units::Duration::from_millis(100),
        }),
        ..common::local_audit()
    }
}

/// Whether a syslog frame or a FHIR `AuditEvent` is the record of a done
/// erasure: it names the operation and succeeded.
fn done_erasure_frame(frame: &str) -> bool {
    frame.contains("admin_access_log_erase") && frame.contains(r#"EventOutcomeIndicator="0""#)
}

/// Whether a syslog frame is the record of the refused second erasure.
fn refused_erasure_frame(frame: &str) -> bool {
    frame.contains("admin_access_log_erase") && !frame.contains(r#"EventOutcomeIndicator="0""#)
}

/// Whether a FHIR `AuditEvent` is the record of a done erasure.
fn done_erasure_event(event: &Value) -> bool {
    event.to_string().contains("admin_access_log_erase") && event["outcome"] == json!("0")
}

/// The syslog frames up to and including the record of the refused second
/// erasure, which one drain ships after every record of the first.
async fn frames_until_refused(syslog: &mut SyslogTlsListener) -> Vec<String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let mut frames = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let Some(frame) = syslog.recv(left).await else {
            panic!("no syslog record of the refused erasure; frames seen: {frames:#?}");
        };
        let frame = String::from_utf8_lossy(&frame).into_owned();
        let last = refused_erasure_frame(&frame);
        frames.push(frame);
        if last {
            return frames;
        }
    }
}

/// The `AuditEvent` documents the FHIR Feed delivered, once its outbox is
/// empty after the record of the refused second erasure has been stored.
async fn delivered_when_drained(pool: &PgPool, arr: &MockServer) -> Vec<Value> {
    failed_erasure(pool).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        let (pending,): (i64,) = sqlx::query_as("SELECT count(*) FROM audit.fhir_feed_outbox")
            .fetch_one(pool)
            .await
            .expect("read the feed outbox");
        if pending == 0 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the FHIR Feed never drained its outbox"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    arr.received_requests()
        .await
        .expect("the ARR records its requests")
        .iter()
        .map(|request| serde_json::from_slice(&request.body).expect("a FHIR AuditEvent"))
        .collect()
}

/// A done erasure, whose act writes and emits its own record, is one record
/// on syslog, one in the store and one on the FHIR Feed (#5602).
#[tokio::test]
async fn an_erasure_is_one_record_on_every_sink() {
    let mut syslog = SyslogTlsListener::bind(0).await.expect("syslog listener");
    let arr = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/fhir/AuditEvent"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&arr)
        .await;
    let (_db, pool, app) = Box::pin(app_auditing(every_sink(&syslog, &arr))).await;

    let (status, body) = erase(&app, "root", &json!({ "uid": UID, "grounds": ordered() })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = erase(&app, "root", &json!({ "uid": UID, "grounds": ordered() })).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let frames = frames_until_refused(&mut syslog).await;
    let on_syslog = frames.iter().filter(|f| done_erasure_frame(f)).count();
    assert_eq!(
        on_syslog, 1,
        "one syslog record of the erasure: {frames:#?}"
    );

    let delivered = delivered_when_drained(&pool, &arr).await;
    let on_feed = delivered.iter().filter(|e| done_erasure_event(e)).count();
    assert_eq!(
        on_feed, 1,
        "one FHIR Feed record of the erasure: {delivered:#?}"
    );

    let (stored,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM audit.access_trail \
          WHERE operation = 'admin_access_log_erase' AND resource_id = $1",
    )
    .bind(UID)
    .fetch_one(&pool)
    .await
    .expect("count the stored records of the erasure");
    assert_eq!(stored, 1, "one stored record of the erasure");
}
