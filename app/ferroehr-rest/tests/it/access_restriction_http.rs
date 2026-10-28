// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHDS Art. 8 restriction of access by health professionals, end to end
//! over the assembled router (#3682).
//!
//! Regulation (EU) 2025/327 Art. 8 lets a person restrict the access of health
//! professionals and healthcare providers to all or parts of their data and
//! keeps the restriction "not visible to healthcare providers"; Art. 11(5) lets
//! the restricted data be reached in the vital interest of the person, logged
//! for them (`docs/law/eu/ehds/text.html`). The tests assert that a restricted
//! composition and a restricted EHR answer every professional route as an
//! unknown one does, that a query and an export leave them out, that only the
//! configured emergency role declaring an emergency purpose reads them, with
//! the emergency mark on its record, and that a lift brings them back. No
//! openEHR spec governs restriction of access — our own design/extension.

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
use ferroehr::system_log::config::{AuditConfig, StoreConfig};
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
/// A versioned-object id no composition holds.
const UNKNOWN_VO: &str = "11111111-2222-4333-8444-555555555555";
/// An EHR id no EHR holds.
const UNKNOWN_EHR: &str = "66666666-7777-4888-9999-000000000000";

fn basic(name: &str) -> String {
    use base64::Engine;
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{name}:pw").as_bytes())
    )
}

/// An audited app with `ETREAT` agreed as an emergency purpose, the
/// `EMERGENCY` role configured as the emergency access role, Basic users for
/// an administrator, a clinician and a medic holding the emergency role, and
/// the IPS template uploaded.
async fn app() -> (testkit::TestDb, PgPool, Router) {
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
            full_verify_interval_seconds: 0,
        },
        ..AuditConfig::default()
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
            ("medic", &["USER", "EMERGENCY"]),
        ],
    );
    config.rbac.emergency_access_role = Some("EMERGENCY".to_owned());
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

/// Creates an EHR as the clinician and returns its id.
async fn ehr(app: &Router) -> String {
    let (status, body) = send(app, "POST", &format!("{BASE}/ehr"), "clinician", None, None).await;
    assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
    let value: Value = serde_json::from_str(&body).expect("EHR json");
    value["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned()
}

/// Commits the IPS composition into `ehr` as the clinician; returns its
/// versioned-object id.
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

/// The admin records (`lift = false`) or lifts an access restriction.
async fn mark(app: &Router, ehr: &str, vo: Option<&str>, lift: bool) {
    let mut body = json!({ "ehr_id": ehr });
    if let Some(vo) = vo {
        body["vo_id"] = json!(vo);
    }
    let path = if lift {
        "admin/access-restriction/lift"
    } else {
        "admin/access-restriction"
    };
    let (status, response) = send(
        app,
        "POST",
        &format!("{BASE}/{path}"),
        "root",
        None,
        Some(("application/json", body.to_string())),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{path}: {response}");
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
            json!({ "q": "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c" }).to_string(),
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

/// The `error` member of an error body.
fn error_kind(body: &str) -> Value {
    serde_json::from_str::<Value>(body).expect("an error body")["error"].clone()
}

/// A restricted composition answers every professional read as an unknown one
/// does, leaves the query result and the export, and comes back on a lift.
#[tokio::test]
async fn a_restricted_composition_reads_as_absent_until_lifted() {
    let (_db, _pool, app) = app().await;
    let ehr = ehr(&app).await;
    let kept = commit(&app, &ehr).await;
    let hidden = commit(&app, &ehr).await;
    let read = |vo: String| {
        let app = app.clone();
        let ehr = ehr.clone();
        async move {
            send(
                &app,
                "GET",
                &format!("{BASE}/ehr/{ehr}/composition/{vo}"),
                "clinician",
                None,
                None,
            )
            .await
        }
    };
    assert_eq!(read(hidden.clone()).await.0, StatusCode::OK);

    mark(&app, &ehr, Some(&hidden), false).await;

    let (status, body) = read(hidden.clone()).await;
    let (unknown_status, unknown_body) = read(UNKNOWN_VO.to_owned()).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(status, unknown_status);
    assert_eq!(error_kind(&body), error_kind(&unknown_body));
    assert!(!body.to_lowercase().contains("restrict"), "{body}");
    for path in [
        format!("{BASE}/ehr/{ehr}/versioned_composition/{hidden}"),
        format!("{BASE}/ehr/{ehr}/versioned_composition/{hidden}/revision_history"),
    ] {
        let (status, body) = send(&app, "GET", &path, "clinician", None, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
    }
    let (status, body) = send(
        &app,
        "DELETE",
        &format!("{BASE}/ehr/{ehr}/composition/{hidden}::local.ehrbase.org::1"),
        "clinician",
        None,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a write is refused as absent: {body}"
    );

    let uids = composition_uids(&app, "clinician", None).await;
    assert!(uids.iter().any(|uid| uid.starts_with(&kept)), "{uids:?}");
    assert!(
        !uids.iter().any(|uid| uid.starts_with(&hidden)),
        "the query leaves the restricted composition out: {uids:?}"
    );
    let (status, export) = send(
        &app,
        "GET",
        &format!("{EXT}/message/export/{ehr}"),
        "clinician",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "export: {export}");
    assert!(!export.contains(&hidden), "the export leaves it out");
    assert!(export.contains(&kept), "the export keeps the rest");

    mark(&app, &ehr, Some(&hidden), true).await;
    assert_eq!(
        read(hidden.clone()).await.0,
        StatusCode::OK,
        "a lift brings it back"
    );
    let uids = composition_uids(&app, "clinician", None).await;
    assert!(uids.iter().any(|uid| uid.starts_with(&hidden)), "{uids:?}");
}

/// A whole-EHR restriction makes the EHR answer as an unknown EHR does, by id,
/// in a scoped query, and for a write, and keeps its compositions out of a
/// population query; the admin still reads the register.
#[tokio::test]
async fn a_restricted_ehr_reads_as_absent() {
    let (_db, _pool, app) = app().await;
    let ehr = ehr(&app).await;
    let vo = commit(&app, &ehr).await;
    mark(&app, &ehr, None, false).await;

    let (status, body) = send(
        &app,
        "GET",
        &format!("{BASE}/ehr/{ehr}"),
        "clinician",
        None,
        None,
    )
    .await;
    let (unknown_status, unknown_body) = send(
        &app,
        "GET",
        &format!("{BASE}/ehr/{UNKNOWN_EHR}"),
        "clinician",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(status, unknown_status);
    assert_eq!(error_kind(&body), error_kind(&unknown_body));

    let (status, body) = send(
        &app,
        "GET",
        &format!(
            "{BASE}/query/aql?q={}&ehr_id={ehr}",
            urlencoding::encode("SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c")
        ),
        "clinician",
        None,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a scoped query names no such EHR: {body}"
    );

    let (status, body) = send(
        &app,
        "POST",
        &format!("{BASE}/ehr/{ehr}/composition"),
        "clinician",
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
        "a write to it is refused as absent: {body}"
    );

    let uids = composition_uids(&app, "clinician", None).await;
    assert!(!uids.iter().any(|uid| uid.starts_with(&vo)), "{uids:?}");

    let (status, body) = send(
        &app,
        "GET",
        &format!("{EXT}/admin/access-restriction?ehr_id={ehr}"),
        "root",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let register: Value = serde_json::from_str(&body).expect("register json");
    assert_eq!(
        register["access_restrictions"].as_array().map(Vec::len),
        Some(1)
    );
}

/// Only the emergency role declaring an emergency purpose reads restricted
/// data, and that access carries the emergency mark; either alone does not.
#[tokio::test]
async fn only_an_emergency_access_reads_restricted_data_and_is_marked() {
    let (_db, pool, app) = app().await;
    let ehr = ehr(&app).await;
    let vo = commit(&app, &ehr).await;
    mark(&app, &ehr, Some(&vo), false).await;
    let path = format!("{BASE}/ehr/{ehr}/composition/{vo}");

    for (user, purpose) in [
        ("clinician", Some("ETREAT")),
        ("medic", None),
        ("medic", Some("TREAT")),
    ] {
        let (status, body) = send(&app, "GET", &path, user, purpose, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{user} {purpose:?}: {body}");
    }
    let (status, body) = send(&app, "GET", &path, "medic", Some("ETREAT"), None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the emergency access reads it: {body}"
    );
    let uids = composition_uids(&app, "medic", Some("ETREAT")).await;
    assert!(uids.iter().any(|uid| uid.starts_with(&vo)), "{uids:?}");

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let marked: Option<bool> = sqlx::query_scalar(
            "SELECT emergency_access FROM audit.audit_event \
             WHERE operation = 'composition_get' AND outcome = 0 AND principal = 'medic' \
             ORDER BY chain_seq DESC LIMIT 1",
        )
        .fetch_optional(&pool)
        .await
        .expect("read the trail");
        if let Some(marked) = marked {
            assert!(marked, "the emergency access carries the mark");
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the emergency read was not recorded"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
