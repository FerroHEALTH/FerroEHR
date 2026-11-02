// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHDS Art. 10 opt-out from access through the access services, end to
//! end over the assembled router (#4516).
//!
//! Regulation (EU) 2025/327 Art. 10(1) lets a person opt out from access to
//! their data "through the electronic health data access services referred to
//! in Articles 4 and 12", reversibly, and Art. 10(2) lets a healthcare
//! provider or health professional reach the data where processing is
//! necessary to protect vital interests (`docs/law/eu/ehds/text.html`). The
//! tests assert that an opted-out EHR answers an access-service caller at
//! every route, query and export as an unknown EHR does while every other
//! caller reads it as before, that a reversal brings it back, that an
//! emergency purpose overrides it with the override on the trail, that
//! recording, reversing and reading the register are access records, and
//! that recording is refused while the deployment declares no access-service
//! caller. No openEHR spec governs an opt-out: our own design/extension.

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
use ferroehr::config::authz::AuthzConfig;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::config::AuditConfig;
use ferroehr::system_log::sender::{AuditHandle, start};
use ferroehr::system_log::store::AuditStore;
use ferroehr_rest::extensions::access::authz::AuthzHandle;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common;
use crate::common::BASE;
use crate::common::EXT;

/// The header the default configuration reads the declared purpose from.
const PURPOSE: &str = "x-purpose-of-use";
/// An EHR id no EHR holds.
const UNKNOWN_EHR: &str = "66666666-7777-4888-9999-000000000000";
/// The role this deployment declares for its access services.
const ACCESS_SERVICE: &str = "EHDS-ACCESS-SERVICE";

fn basic(name: &str) -> String {
    use base64::Engine;
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{name}:pw").as_bytes())
    )
}

/// An audited app with `ETREAT` agreed as an emergency purpose, Basic users
/// for an administrator, a clinician of the provider's own care and an
/// access service, and the IPS template uploaded; `declared` decides whether
/// the deployment declares [`ACCESS_SERVICE`] an access-service role.
async fn app(declared: bool) -> (testkit::TestDb, PgPool, Router) {
    let (db, pool) = common::migrated_pool().await;
    let audit = AuditConfig {
        purpose_codes: vec!["TREAT".to_owned(), "ETREAT".to_owned()],
        emergency_purpose_codes: vec!["ETREAT".to_owned()],
        ..common::local_audit()
    };
    let (sender, _handle): (_, AuditHandle) = start(audit, None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_audit(sender)
            .with_audit_store(AuditStore::new(pool.clone())),
    );
    let mut config = common::api_config(true);
    common::with_basic_users(
        &mut config,
        &[
            ("root", &["ADMIN"]),
            ("clinician", &["USER"]),
            ("service", &["USER", ACCESS_SERVICE]),
        ],
    );
    if declared {
        config.rbac.access_service_roles = vec![ACCESS_SERVICE.to_owned()];
    }
    let mut authz = AuthzConfig::default();
    authz.rbac.enabled = true;
    let authz = AuthzHandle::build(
        &authz,
        &config.rest.base_path,
        None,
        common::null_resolvers(),
    )
    .map(Arc::new);
    let app = ferroehr_rest::build_full(config, svc, authz, Observability::default())
        .await
        .expect("build app");
    let (status, body) = send(
        &app,
        "POST",
        &format!("{BASE}/definition/template/adl1.4"),
        "root",
        None,
        Some(("application/xml", common::ips_opt_xml())),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");
    (db, pool, app)
}

/// Sends `method uri` as `user`, with an optional declared purpose and body.
async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    user: &str,
    purpose: Option<&str>,
    body: Option<(&str, String)>,
) -> (StatusCode, String) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, basic(user))
        .header(header::ACCEPT, "application/json")
        .header("Prefer", "return=representation");
    if let Some(purpose) = purpose {
        req = req.header(PURPOSE, purpose);
    }
    let req = match body {
        Some((content_type, body)) => req
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::from(body)),
        None => req.body(Body::empty()),
    }
    .unwrap();
    common::send_body(app, req).await
}

/// Creates an EHR as the clinician and commits the IPS composition into it;
/// returns the EHR id and the composition's versioned-object id.
async fn ehr_with_composition(app: &Router) -> (String, String) {
    let (status, body) = send(app, "POST", &format!("{BASE}/ehr"), "clinician", None, None).await;
    assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
    let value: Value = serde_json::from_str(&body).expect("EHR json");
    let ehr = value["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned();
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
    let vo = headers
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag")
        .trim_start_matches("W/")
        .trim_matches('"')
        .split("::")
        .next()
        .expect("object id")
        .to_owned();
    (ehr, vo)
}

/// The admin records (`reverse = false`) or reverses the opt-out of `ehr`;
/// returns the answer.
async fn opt_out(app: &Router, ehr: &str, reverse: bool) -> (StatusCode, String) {
    let path = if reverse {
        "/admin/access-opt-out/reverse"
    } else {
        "/admin/access-opt-out"
    };
    send(
        app,
        "POST",
        &common::full(path),
        "root",
        None,
        Some(("application/json", json!({ "ehr_id": ehr }).to_string())),
    )
    .await
}

/// The composition uids an AQL population query returns to `user`.
async fn composition_uids(app: &Router, user: &str, purpose: Option<&str>) -> Vec<String> {
    let (status, body) = send(
        app,
        "POST",
        &format!("{BASE}/query/aql"),
        user,
        purpose,
        Some((
            "application/json",
            json!({ "q": "SELECT c/uid/value FROM COMPOSITION c" }).to_string(),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "query: {body}");
    let result: Value = serde_json::from_str(&body).expect("RESULT_SET JSON");
    result["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row[0].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// The problem type of an error body.
fn error_kind(body: &str) -> Value {
    serde_json::from_str::<Value>(body).expect("an error body")["type"].clone()
}

/// The `(action, principal, emergency_access)` of every stored record whose
/// resource is `resource`, polled until `expected` have landed.
async fn trail(pool: &PgPool, resource: &str, expected: usize) -> Vec<(String, String, bool)> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let rows: Vec<(String, Option<String>, bool)> = sqlx::query_as(
            "SELECT action, principal, emergency_access FROM audit.audit_event \
             WHERE resource_id = $1 ORDER BY chain_seq",
        )
        .bind(resource)
        .fetch_all(pool)
        .await
        .expect("read the trail");
        if rows.len() >= expected {
            return rows
                .into_iter()
                .map(|(action, principal, emergency)| {
                    (action, principal.unwrap_or_default(), emergency)
                })
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

/// While the opt-out stands, the access service reads the EHR as absent at
/// every route, query and export, a write included, and the provider's own
/// clinician reads it as before; the reversal brings it back.
#[tokio::test]
async fn an_opted_out_ehr_reads_as_absent_to_the_access_services_until_reversed() {
    let (_db, _pool, app) = app(true).await;
    let (ehr, vo) = ehr_with_composition(&app).await;
    let (_, kept) = ehr_with_composition(&app).await;
    let composition = format!("{BASE}/ehr/{ehr}/composition/{vo}");
    assert_eq!(
        send(&app, "GET", &composition, "service", None, None)
            .await
            .0,
        StatusCode::OK
    );

    let (status, body) = opt_out(&app, &ehr, false).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "record: {body}");

    let (status, body) = send(
        &app,
        "GET",
        &format!("{BASE}/ehr/{ehr}"),
        "service",
        None,
        None,
    )
    .await;
    let (unknown_status, unknown_body) = send(
        &app,
        "GET",
        &format!("{BASE}/ehr/{UNKNOWN_EHR}"),
        "service",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(status, unknown_status);
    assert_eq!(error_kind(&body), error_kind(&unknown_body));
    for path in [
        composition.clone(),
        format!("{BASE}/ehr/{ehr}/ehr_status"),
        format!("{BASE}/ehr/{ehr}/versioned_composition/{vo}"),
        format!("{EXT}/message/export/{ehr}"),
        format!(
            "{BASE}/query/aql?q={}&ehr_id={ehr}",
            urlencoding::encode("SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c")
        ),
    ] {
        let (status, body) = send(&app, "GET", &path, "service", None, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
    }
    let (status, body) = send(
        &app,
        "POST",
        &format!("{BASE}/ehr/{ehr}/composition"),
        "service",
        None,
        Some((
            "application/json",
            common::ips_canonical_composition().to_string(),
        )),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a write is refused as absent: {body}"
    );
    let uids = composition_uids(&app, "service", None).await;
    assert!(uids.iter().any(|uid| uid.starts_with(&kept)), "{uids:?}");
    assert!(
        !uids.iter().any(|uid| uid.starts_with(&vo)),
        "the population query leaves the opted-out EHR out: {uids:?}"
    );

    assert_eq!(
        send(&app, "GET", &composition, "clinician", None, None)
            .await
            .0,
        StatusCode::OK,
        "the provider's own care reads the EHR as before"
    );
    let uids = composition_uids(&app, "clinician", None).await;
    assert!(uids.iter().any(|uid| uid.starts_with(&vo)), "{uids:?}");

    let (status, body) = opt_out(&app, &ehr, true).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "reverse: {body}");
    assert_eq!(
        send(&app, "GET", &composition, "service", None, None)
            .await
            .0,
        StatusCode::OK,
        "the reversal brings the EHR back to the access services"
    );
    let uids = composition_uids(&app, "service", None).await;
    assert!(uids.iter().any(|uid| uid.starts_with(&vo)), "{uids:?}");
}

/// Recording, reversing and reading the register are access records naming
/// the administrator, and the register keeps the reversed opt-out.
#[tokio::test]
async fn recording_reversing_and_reading_the_opt_out_are_access_records() {
    let (_db, pool, app) = app(true).await;
    let (ehr, _) = ehr_with_composition(&app).await;
    assert_eq!(opt_out(&app, &ehr, false).await.0, StatusCode::NO_CONTENT);
    assert_eq!(opt_out(&app, &ehr, true).await.0, StatusCode::NO_CONTENT);
    let (status, body) = send(
        &app,
        "GET",
        &format!("{EXT}/admin/access-opt-out?ehr_id={ehr}"),
        "root",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let register: Value = serde_json::from_str(&body).expect("register json");
    let rows = register["access_opt_outs"].as_array().expect("rows");
    assert_eq!(rows.len(), 1, "{register}");
    assert_eq!(rows[0]["ehr_id"], json!(ehr));
    assert!(rows[0]["reversed_at"].is_string(), "{register}");

    let actions: Vec<String> = trail(&pool, "access-opt-out:ehr-wide", 3)
        .await
        .into_iter()
        .map(|(action, principal, emergency)| {
            assert_eq!(principal, "root");
            assert!(!emergency);
            action
        })
        .collect();
    assert_eq!(actions, ["C", "D", "R"]);
}

/// An access service declaring an emergency purpose reads the opted-out EHR
/// under Art. 10(2), with the emergency mark on its record and the override
/// recorded against the EHR; an ordinary purpose does not lift the opt-out.
#[tokio::test]
async fn the_vital_interest_override_reads_the_ehr_and_is_recorded() {
    let (_db, pool, app) = app(true).await;
    let (ehr, vo) = ehr_with_composition(&app).await;
    assert_eq!(opt_out(&app, &ehr, false).await.0, StatusCode::NO_CONTENT);
    let composition = format!("{BASE}/ehr/{ehr}/composition/{vo}");

    let (status, body) = send(&app, "GET", &composition, "service", Some("TREAT"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "an ordinary purpose: {body}");
    let (status, body) = send(&app, "GET", &composition, "service", Some("ETREAT"), None).await;
    assert_eq!(status, StatusCode::OK, "the override reads it: {body}");
    let uids = composition_uids(&app, "service", Some("ETREAT")).await;
    assert!(uids.iter().any(|uid| uid.starts_with(&vo)), "{uids:?}");

    assert_eq!(
        trail(&pool, "access-opt-out-override:ehr-wide", 1).await,
        vec![("E".to_owned(), "service".to_owned(), true)]
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let marked: Option<bool> = sqlx::query_scalar(
            "SELECT emergency_access FROM audit.audit_event \
             WHERE operation = 'composition_get' AND outcome = 0 AND principal = 'service' \
             ORDER BY chain_seq DESC LIMIT 1",
        )
        .fetch_optional(&pool)
        .await
        .expect("read the trail");
        if let Some(marked) = marked {
            assert!(marked, "the override access carries the emergency mark");
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the override read was not recorded"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// With no access-service role declared, the opt-out is off: recording one is
/// refused and nothing is written, while reversing and reading stay open.
#[tokio::test]
async fn recording_is_refused_while_no_access_service_is_declared() {
    let (_db, _pool, app) = app(false).await;
    let (ehr, vo) = ehr_with_composition(&app).await;
    let (status, body) = opt_out(&app, &ehr, false).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("access_service_roles"), "{body}");
    assert_eq!(opt_out(&app, &ehr, true).await.0, StatusCode::NO_CONTENT);
    let (status, body) = send(
        &app,
        "GET",
        &format!("{EXT}/admin/access-opt-out?ehr_id={ehr}"),
        "root",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let register: Value = serde_json::from_str(&body).expect("register json");
    assert_eq!(register["access_opt_outs"], json!([]));
    assert_eq!(
        send(
            &app,
            "GET",
            &format!("{BASE}/ehr/{ehr}/composition/{vo}"),
            "service",
            None,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
}
