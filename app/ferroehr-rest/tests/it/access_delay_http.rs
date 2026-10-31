// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHDS Art. 3(3) delay of the person's own access, end to end over the
//! assembled router (#4351).
//!
//! Regulation (EU) 2025/327 Art. 3(1) gives a person access to their data
//! "immediately after" it is registered, and Art. 3(3) lets a Member State
//! delay that access "for a limited period of time until a health
//! professional is able to properly communicate and explain" it
//! (`docs/law/eu/ehds/text.html`). The tests assert that a composition of a
//! configured category reads as absent to a patient-context request (a SMART
//! `patient/` grant) until a professional releases it or its limit passes,
//! that a professional always reads it, that the release and the expiry are
//! access records, and that a deployment without the delay withholds nothing.
//! No openEHR spec governs a delay of the person's access: our own
//! design/extension.

#![expect(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use ferroehr::service::FerroEhrService;
use ferroehr::service::admin::access_delay::config::{AccessDelayConfig, AccessDelayPolicy};
use ferroehr::system_log::categories::CategoryMapConfig;
use ferroehr::system_log::config::{AuditConfig, StoreConfig};
use ferroehr::system_log::sender::{AuditHandle, start};
use ferroehr::system_log::store::AuditStore;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common;
use crate::common::{BASE, EXT};

const ISSUER: &str = "https://issuer.example";
const AUDIENCE: &str = "ferroehr";
/// The professional's subject.
const PROFESSIONAL: &str = "dr-jansen";

/// A deployment with the delay on for patient summaries (72 hours) when
/// `delayed`, the professional's and the patient's issuer bound, auditing to
/// the local store, and the IPS template uploaded.
async fn app(delayed: bool) -> (testkit::TestDb, PgPool, Arc<FerroEhrService>, Router) {
    let (db, pool) = common::migrated_pool().await;
    let audit = AuditConfig {
        enabled: true,
        store: StoreConfig {
            enabled: true,
            retention_days: 0,
            retention_years: None,
            sgb_v_309_controller: false,
            verify_interval_seconds: 0,
            full_verify_interval_seconds: 0,
        },
        ..AuditConfig::default()
    };
    let (sender, _handle): (_, AuditHandle) = start(audit, None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let classifier = CategoryMapConfig::default()
        .compile()
        .expect("the shipped map compiles");
    let section = AccessDelayConfig {
        enabled: delayed,
        categories: vec!["Patient-Summaries".to_owned()],
        hours: 72,
    };
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_audit(sender)
            .with_audit_store(AuditStore::new(pool.clone()))
            .with_access_delay(AccessDelayPolicy::compile(&section, &classifier)),
    );
    svc.mirror_category_map(&classifier)
        .await
        .expect("mirror the category map");
    let mut config = common::api_config(true);
    config.auth = common::issuer_auth(ISSUER, AUDIENCE);
    config.smart.patient_issuers = vec![ISSUER.to_owned()];
    let app = ferroehr_rest::build_full(config, Arc::clone(&svc), None, Observability::default())
        .await
        .expect("build app");
    let (status, body) = send(
        &app,
        "POST",
        &format!("{BASE}/definition/template/adl1.4"),
        &professional(),
        Some(("application/xml", common::ips_opt_xml())),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");
    (db, pool, svc, app)
}

/// The professional's token: every `user/` grant and the product scope.
fn professional() -> String {
    common::bearer(
        &json!({ "sub": PROFESSIONAL, "iss": ISSUER, "aud": AUDIENCE,
        "exp": jiff::Timestamp::now().as_second() + 3600 }),
    )
}

/// The person's token: `patient/` read and query grants in the launch context
/// of `ehr`.
fn patient(ehr: &str) -> String {
    common::bearer(&json!({ "sub": "patient-7", "iss": ISSUER, "aud": AUDIENCE,
        "exp": jiff::Timestamp::now().as_second() + 3600,
        "scope": "patient/composition-*.r patient/aql-*.s", "ehrId": ehr }))
}

/// Sends `method uri` with `credential` and an optional body.
async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    credential: &str,
    body: Option<(&str, String)>,
) -> (StatusCode, String) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, credential)
        .header(header::ACCEPT, "application/json")
        .header("Prefer", "return=representation");
    let req = match body {
        Some((content_type, body)) => req
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::from(body)),
        None => req.body(Body::empty()),
    }
    .unwrap();
    common::send_body(app, req).await
}

/// Creates an EHR and commits the IPS composition into it as the
/// professional; returns the EHR id and the composition's object id.
async fn record(app: &Router) -> (String, String) {
    let (status, body) = send(app, "POST", &format!("{BASE}/ehr"), &professional(), None).await;
    assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
    let ehr = serde_json::from_str::<Value>(&body).expect("EHR json")["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned();
    let vo = commit(app, &ehr).await;
    (ehr, vo)
}

/// Commits the IPS composition into `ehr` as the professional; returns its
/// object id.
async fn commit(app: &Router, ehr: &str) -> String {
    let (status, headers, body) = common::send(
        app,
        Request::post(format!("{BASE}/ehr/{ehr}/composition"))
            .header(header::AUTHORIZATION, professional())
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(common::ips_canonical_composition().to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "commit: {body}");
    headers
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag")
        .trim_start_matches("W/")
        .trim_matches('"')
        .split("::")
        .next()
        .expect("object id")
        .to_owned()
}

/// The status of a read of the composition `vo` with `credential`.
async fn read(app: &Router, ehr: &str, vo: &str, credential: &str) -> StatusCode {
    send(
        app,
        "GET",
        &format!("{BASE}/ehr/{ehr}/composition/{vo}"),
        credential,
        None,
    )
    .await
    .0
}

/// The composition uids a query scoped to `ehr` returns to `credential`.
async fn composition_uids(app: &Router, ehr: &str, credential: &str) -> Vec<String> {
    let q = urlencoding::encode("SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c");
    let (status, body) = send(
        app,
        "GET",
        &format!("{BASE}/query/aql?q={q}&ehr_id={ehr}"),
        credential,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "query: {body}");
    serde_json::from_str::<Value>(&body).expect("RESULT_SET JSON")["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row[0].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// The professional posts `body` to the admin route `path`.
async fn admin(app: &Router, path: &str, body: &Value) -> (StatusCode, String) {
    send(
        app,
        "POST",
        &format!("{EXT}/admin/{path}"),
        &professional(),
        Some(("application/json", body.to_string())),
    )
    .await
}

/// The principal of the access record whose object is `resource`, once the
/// trail holds it.
async fn recorded(pool: &PgPool, resource: &str) -> Option<String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT principal FROM audit.audit_event WHERE resource_id = $1 \
             ORDER BY chain_seq DESC LIMIT 1",
        )
        .bind(resource)
        .fetch_optional(pool)
        .await
        .expect("read the trail");
        if let Some((principal,)) = row {
            return principal;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no access record of {resource}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A composition of a delayed category reads as absent to the person, by id,
/// in a query and in the version history, until a professional releases it;
/// the professional reads it throughout, and the release is an access record
/// naming the professional.
#[tokio::test]
async fn a_new_result_is_withheld_from_the_person_until_a_professional_releases_it() {
    let (_db, pool, _svc, app) = app(true).await;
    let (ehr, vo) = record(&app).await;
    let person = patient(&ehr);

    assert_eq!(read(&app, &ehr, &vo, &person).await, StatusCode::NOT_FOUND);
    let (status, body) = send(
        &app,
        "GET",
        &format!("{BASE}/ehr/{ehr}/versioned_composition/{vo}/revision_history"),
        &person,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let uids = composition_uids(&app, &ehr, &person).await;
    assert!(!uids.iter().any(|uid| uid.starts_with(&vo)), "{uids:?}");

    assert_eq!(read(&app, &ehr, &vo, &professional()).await, StatusCode::OK);
    let uids = composition_uids(&app, &ehr, &professional()).await;
    assert!(uids.iter().any(|uid| uid.starts_with(&vo)), "{uids:?}");

    let (status, body) = admin(
        &app,
        "access-delay/release",
        &json!({ "ehr_id": ehr, "vo_id": vo }),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "release: {body}");
    assert_eq!(read(&app, &ehr, &vo, &person).await, StatusCode::OK);
    let uids = composition_uids(&app, &ehr, &person).await;
    assert!(uids.iter().any(|uid| uid.starts_with(&vo)), "{uids:?}");
    assert_eq!(
        recorded(&pool, &format!("access-delay:{vo}"))
            .await
            .as_deref(),
        Some(PROFESSIONAL),
        "the release names the professional who made it"
    );
}

/// A hold ends by itself at its limit (Art. 3(3): "for a limited period of
/// time"): the person then reads the composition, and the expiry is recorded
/// once.
#[tokio::test]
async fn a_hold_ends_at_its_limit_and_the_expiry_is_recorded() {
    let (_db, pool, svc, app) = app(true).await;
    let (ehr, vo) = record(&app).await;
    let person = patient(&ehr);
    assert_eq!(read(&app, &ehr, &vo, &person).await, StatusCode::NOT_FOUND);

    // The limit passing, without waiting 72 hours for it.
    sqlx::query(
        "UPDATE clinical.access_delay SET withheld_at = now() - interval '73 hours', \
         withheld_until = now() - interval '1 hour' WHERE vo_id = $1::uuid",
    )
    .bind(&vo)
    .execute(&pool)
    .await
    .expect("age the hold");
    sqlx::query(
        "UPDATE clinical.vo_head SET person_withheld_until = now() - interval '1 hour' \
         WHERE vo_id = $1::uuid",
    )
    .bind(&vo)
    .execute(&pool)
    .await
    .expect("age the mark");

    assert_eq!(read(&app, &ehr, &vo, &person).await, StatusCode::OK);
    assert_eq!(svc.record_access_delay_expiries().await.expect("record"), 1);
    assert_eq!(svc.record_access_delay_expiries().await.expect("record"), 0);
    recorded(&pool, &format!("access-delay-expiry:{vo}")).await;
}

/// A professional holds a composition by hand within the configured limit,
/// and an end past the limit is refused.
#[tokio::test]
async fn a_professional_holds_a_composition_within_the_limit() {
    let (_db, _pool, _svc, app) = app(true).await;
    let (ehr, vo) = record(&app).await;
    let person = patient(&ehr);
    let (status, body) = admin(
        &app,
        "access-delay/release",
        &json!({ "ehr_id": ehr, "vo_id": vo }),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(read(&app, &ehr, &vo, &person).await, StatusCode::OK);

    let too_late = jiff::Timestamp::now() + jiff::SignedDuration::from_hours(100);
    let (status, body) = admin(
        &app,
        "access-delay",
        &json!({ "ehr_id": ehr, "vo_id": vo, "until": too_late.to_string() }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "past the limit: {body}");
    assert_eq!(read(&app, &ehr, &vo, &person).await, StatusCode::OK);

    let (status, body) = admin(
        &app,
        "access-delay",
        &json!({ "ehr_id": ehr, "vo_id": vo, "note": "to discuss at the visit" }),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "hold: {body}");
    assert_eq!(read(&app, &ehr, &vo, &person).await, StatusCode::NOT_FOUND);
    assert_eq!(read(&app, &ehr, &vo, &professional()).await, StatusCode::OK);

    let (status, body) = send(
        &app,
        "GET",
        &format!("{EXT}/admin/access-delay?ehr_id={ehr}"),
        &professional(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let register: Value = serde_json::from_str(&body).expect("register json");
    let bases: Vec<&str> = register["access_delays"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row["basis"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(bases, ["professional", "category"], "newest hold first");
}

/// Without `[access_delay]` the person reads new data immediately (Art. 3(1))
/// and no hold may be placed.
#[tokio::test]
async fn a_deployment_without_the_delay_withholds_nothing() {
    let (_db, _pool, _svc, app) = app(false).await;
    let (ehr, vo) = record(&app).await;
    assert_eq!(read(&app, &ehr, &vo, &patient(&ehr)).await, StatusCode::OK);
    let (status, body) = admin(&app, "access-delay", &json!({ "ehr_id": ehr, "vo_id": vo })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(read(&app, &ehr, &vo, &patient(&ehr)).await, StatusCode::OK);
}
