// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The emergency-access mark on the access record, end to end over the
//! assembled router, the audit sender and the local Audit Record Repository.
//!
//! Regulation (EU) 2025/327 Art. 11(5) lets a health professional reach data a
//! natural person restricted under Art. 8 where the person's vital interests
//! require it, and requires such cases to be "logged in a clear and
//! understandable format" and "easily accessible for the data subject"; Art.
//! 9(1) includes those accesses in the information the person obtains
//! (`docs/law/eu/ehds/text.html`). An access whose declared purpose is one of
//! `[audit] emergency_purpose_codes` is marked, and the person's own
//! subject-scoped access-log retrieval shows the mark. The mark lifts nothing:
//! a GDPR Art. 18 restriction admits only the processing Art. 18(2) lists, and
//! the data subject's vital interests are not on that list
//! (`docs/law/eu/gdpr/text.html Art. 18(2)`). No openEHR spec governs the
//! read-side access log — our own design/extension.

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

use argon2::Argon2;
use argon2::password_hash::PasswordHasher;
use axum::Router;
use axum::body::Body;
use ferroehr::config::auth::{AuthConfig, BasicConfig, BasicUser};
use ferroehr::config::authz::AuthzConfig;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::config::{AuditConfig, StoreConfig};
use ferroehr::system_log::fhir::{EMERGENCY_ACCESS_TEXT, SYS_V3_ACT_REASON};
use ferroehr::system_log::sender::{AuditHandle, SubjectResolver, start};
use ferroehr::system_log::store::AuditStore;
use ferroehr_rest::extensions::access::authz::AuthzHandle;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode, header};
use serde_json::Value;
use sqlx::PgPool;

use crate::common;
use crate::common::BASE;

/// The subject every EHR in this suite resolves to.
const SUBJECT: &str = "patient-art-11-5";
/// The header the default configuration reads the declared purpose from.
const PURPOSE: &str = "x-purpose-of-use";

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

fn basic(name: &str) -> String {
    use base64::Engine;
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{name}:pw").as_bytes())
    )
}

/// An audited app with the local store on, `ETREAT` agreed as an emergency
/// purpose, Basic users for a clinician, an administrator and a patient portal
/// holding the subject-scoped audit role, and one EHR; returns the EHR id.
async fn app() -> (testkit::TestDb, PgPool, Router, String) {
    let (db, pool) = common::migrated_pool().await;
    let audit = AuditConfig {
        enabled: true,
        purpose_codes: vec!["TREAT".to_owned(), "ETREAT".to_owned()],
        emergency_purpose_codes: vec!["ETREAT".to_owned()],
        store: StoreConfig {
            enabled: true,
            retention_days: 0,
            retention_years: None,
            sgb_v_309_controller: false,
            verify_interval_seconds: 0,
        },
        ..AuditConfig::default()
    };
    let resolver: SubjectResolver =
        Arc::new(|_ehr_id| Box::pin(async { Some(SUBJECT.to_owned()) }));
    let (sender, _handle): (_, AuditHandle) = start(audit, Some(resolver), Some(pool.clone()))
        .await
        .expect("the audit sender");
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_audit(sender)
            .with_audit_store(AuditStore::new(pool.clone())),
    );
    let mut config = common::api_config(true);
    config.auth = AuthConfig {
        enabled: true,
        basic: Some(BasicConfig {
            users: vec![
                user("root", &["ADMIN"]),
                user("clinician", &["USER"]),
                user("portal", &["PORTAL"]),
            ],
        }),
        ..AuthConfig::default()
    };
    let mut authz = AuthzConfig::default();
    authz.rbac.enabled = true;
    authz.rbac.subject_audit_role = Some("PORTAL".to_owned());
    let authz = AuthzHandle::build(
        &authz,
        &config.server.base_path,
        None,
        common::null_resolvers(),
    )
    .map(Arc::new);
    let app =
        ferroehr_rest::build_full(config, svc, authz, Observability::default()).expect("build app");

    let (status, body) = common::send_body(
        &app,
        Request::post(format!("{BASE}/definition/template/adl1.4"))
            .header(header::AUTHORIZATION, basic("root"))
            .header(header::CONTENT_TYPE, "application/xml")
            .body(Body::from(common::ips_opt_xml()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");
    let (status, body) = common::send_body(
        &app,
        Request::post(format!("{BASE}/ehr"))
            .header(header::AUTHORIZATION, basic("clinician"))
            .header("Prefer", "return=representation")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
    let value: Value = serde_json::from_str(&body).expect("EHR json");
    let ehr = value["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned();
    (db, pool, app, ehr)
}

/// Commit the IPS composition as the clinician; returns its versioned-object id.
async fn commit(app: &Router, ehr: &str) -> String {
    let (status, headers, body) = common::send(
        app,
        Request::post(format!("{BASE}/ehr/{ehr}/composition"))
            .header(header::AUTHORIZATION, basic("clinician"))
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

/// Read the composition as the clinician, declaring `purpose` when given.
async fn read(app: &Router, ehr: &str, vo_id: &str, purpose: Option<&str>) -> (StatusCode, String) {
    let mut req = Request::get(format!("{BASE}/ehr/{ehr}/composition/{vo_id}"))
        .header(header::AUTHORIZATION, basic("clinician"));
    if let Some(purpose) = purpose {
        req = req.header(PURPOSE, purpose);
    }
    common::send_body(app, req.body(Body::empty()).unwrap()).await
}

/// The `(purpose, outcome, emergency_access)` of every stored `composition_get`
/// record, polled until `expected` records have landed.
async fn composition_reads(pool: &PgPool, expected: usize) -> Vec<(Option<String>, i16, bool)> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let rows: Vec<(Option<String>, i16, bool)> = sqlx::query_as(
            "SELECT purpose, outcome, emergency_access FROM audit.audit_event \
             WHERE operation = 'composition_get' ORDER BY chain_seq",
        )
        .fetch_all(pool)
        .await
        .expect("read the trail");
        if rows.len() >= expected {
            return rows;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "only {} of {expected} composition_get records landed",
            rows.len()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// An access declared with an emergency purpose is marked in the store, and
/// the patient portal's subject-scoped retrieval of the person's own access
/// log (ITI-81 with `patient`) serves that access with the mark: the fixed
/// `ETREAT` coding on `purposeOfEvent` and the plain-words text. An access
/// with an ordinary purpose carries no mark.
#[tokio::test]
async fn an_emergency_access_is_marked_and_shown_in_the_subjects_own_access_log() {
    let (_db, pool, app, ehr) = app().await;
    let vo_id = commit(&app, &ehr).await;

    let (status, body) = read(&app, &ehr, &vo_id, Some("ETREAT")).await;
    assert_eq!(status, StatusCode::OK, "emergency read: {body}");
    let (status, body) = read(&app, &ehr, &vo_id, Some("TREAT")).await;
    assert_eq!(status, StatusCode::OK, "ordinary read: {body}");

    assert_eq!(
        composition_reads(&pool, 2).await,
        vec![
            (Some("ETREAT".to_owned()), 0, true),
            (Some("TREAT".to_owned()), 0, false),
        ]
    );

    let (status, body) = common::send_body(
        &app,
        Request::get(format!("{BASE}/fhir/r4/AuditEvent?patient={SUBJECT}"))
            .header(header::AUTHORIZATION, basic("portal"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "subject-scoped retrieval: {body}");
    let bundle: Value = serde_json::from_str(&body).expect("bundle json");
    let reads: Vec<&Value> = bundle["entry"]
        .as_array()
        .expect("entries")
        .iter()
        .map(|entry| &entry["resource"])
        .filter(|resource| {
            resource["subtype"]
                .as_array()
                .is_some_and(|s| s.iter().any(|c| c["code"] == "composition_get"))
        })
        .collect();
    assert_eq!(reads.len(), 2, "{bundle}");
    let declared = |resource: &Value| {
        resource["agent"]
            .as_array()
            .expect("agents")
            .iter()
            .find_map(|agent| agent["purposeOfUse"][0]["coding"][0]["code"].as_str())
            .map(str::to_owned)
    };
    let emergency = reads
        .iter()
        .find(|r| declared(r).as_deref() == Some("ETREAT"))
        .expect("the emergency read is in the subject's log");
    let concept = &emergency["purposeOfEvent"][0];
    assert_eq!(concept["coding"][0]["system"], SYS_V3_ACT_REASON);
    assert_eq!(concept["coding"][0]["code"], "ETREAT");
    assert_eq!(concept["text"], EMERGENCY_ACCESS_TEXT);
    let ordinary = reads
        .iter()
        .find(|r| declared(r).as_deref() == Some("TREAT"))
        .expect("the ordinary read is in the subject's log");
    assert!(
        ordinary.get("purposeOfEvent").is_none(),
        "an ordinary access carries no mark: {ordinary}"
    );
}

/// An emergency purpose does not make a GDPR Art. 18 restricted composition
/// readable: the read is refused exactly as it is without one, because Art.
/// 18(2) admits no processing for the data subject's own vital interests. The
/// refused attempt is still logged, and still carries the emergency mark.
#[tokio::test]
async fn an_emergency_purpose_does_not_lift_a_gdpr_restriction() {
    let (_db, pool, app, ehr) = app().await;
    let vo_id = commit(&app, &ehr).await;

    let (status, body) = common::send_body(
        &app,
        Request::post(format!("{BASE}/admin/restriction"))
            .header(header::AUTHORIZATION, basic("root"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(format!(
                r#"{{"ehr_id":"{ehr}","vo_id":"{vo_id}","ground":"gdpr-18-1-a"}}"#
            )))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "restrict: {body}");

    let (plain_status, plain_body) = read(&app, &ehr, &vo_id, None).await;
    assert_eq!(plain_status, StatusCode::FORBIDDEN, "{plain_body}");
    let (status, body) = read(&app, &ehr, &vo_id, Some("ETREAT")).await;
    assert_eq!(
        (status, body.as_str()),
        (plain_status, plain_body.as_str()),
        "an emergency purpose must be refused exactly as a read without one"
    );

    assert_eq!(
        composition_reads(&pool, 2).await,
        vec![(None, 4, false), (Some("ETREAT".to_owned()), 4, true)]
    );
}
