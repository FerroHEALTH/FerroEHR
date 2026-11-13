// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The repair of an EHR held closed by `EHR_ACCESS` settings that do not
//! parse, end to end over the assembled router (#4853).
//!
//! The tests assert that an administrator's repair commits a new
//! `EHR_ACCESS` version with its own commit audit and contribution, that the
//! EHR is then served under the new settings, that the repair is an access
//! record naming the administrator, and that it is refused without the admin
//! role, for an EHR that is not held, and for settings that do not parse. No
//! openEHR specification governs FerroEHR's access-control scheme: our own
//! design.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use ferroehr::config::authz::AuthzConfig;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::system_log::sender::{AuditHandle, start};
use ferroehr::system_log::store::AuditStore;
use ferroehr_rest::extensions::access::authz::AuthzHandle;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common;

/// The EHR whose settings are stored unreadable.
const HELD: &str = "3fa85f64-5717-4562-b3fc-2c963f66afa6";
/// An EHR with the default, settings-less `EHR_ACCESS`.
const OPEN: &str = "7c1b9d3e-2a45-4f6b-9c8d-0e1f2a3b4c5d";
fn basic(name: &str) -> String {
    use base64::Engine;
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{name}:pw").as_bytes())
    )
}

/// Settings that restrict the EHR to the Basic user `user` as its gate-keeper.
async fn restricted_to(pool: &PgPool, user: &str) -> Value {
    common::access_settings("restricted", &[common::basic_person(pool, user).await])
}

/// An audited app with RBAC on, an administrator and two clinicians, over
/// [`HELD`], whose stored settings no longer parse, and [`OPEN`].
async fn app() -> (testkit::TestDb, PgPool, Router) {
    let (db, pool) = common::migrated_pool().await;
    let (sender, _handle): (_, AuditHandle) =
        start(common::local_audit(), None, Some(pool.clone()))
            .await
            .expect("the audit sender");
    let svc = common::binding_service(&pool)
        .with_audit(sender)
        .with_audit_store(AuditStore::new(pool.clone()));
    for id in [HELD, OPEN] {
        Box::pin(
            svc.create_ehr_with_id_meta(id.parse().expect("an EHR id"), WriteRequest::new(None)),
        )
        .await
        .expect("create the EHR");
    }
    let mut config = common::api_config(true);
    common::with_basic_users(
        &mut config,
        &svc,
        &pool,
        &[
            ("root", &["ADMIN"]),
            ("bob", &["USER"]),
            ("carol", &["USER"]),
        ],
    )
    .await;
    let held: ferroehr::ids::EhrId = HELD.parse().expect("an EHR id");
    let ehr = svc.ehr_read(held).await.expect("the EHR").body;
    svc.create_ehr_contribution(
        held,
        access_contribution(&ehr, &restricted_to(&pool, "bob").await),
    )
    .await
    .expect("the settings commit");
    sqlx::query(
        "UPDATE version SET body = jsonb_set(body::jsonb, '{settings,default_access}', \
         '\"bogus\"')::text WHERE kind = 'EHR_ACCESS' AND ehr_id = $1",
    )
    .bind(held)
    .execute(&pool)
    .await
    .expect("store settings that do not parse");
    let mut authz = AuthzConfig::default();
    authz.rbac.enabled = true;
    let authz = AuthzHandle::build(
        &authz,
        &config.api.base_path,
        None,
        common::null_resolvers(),
    )
    .map(Arc::new);
    let app = common::build_full(config, Arc::new(svc), authz, Observability::default()).await;
    (db, pool, app)
}

/// The CONTRIBUTION that commits `settings` as the second `EHR_ACCESS`
/// version of the EHR `ehr` (its `EHR` object) names.
fn access_contribution(ehr: &Value, settings: &Value) -> Value {
    let access_vo = ehr["ehr_access"]["id"]["value"]
        .as_str()
        .expect("EHR.ehr_access.id.value");
    let audit = json!({
        "change_type": {
            "_type": "DV_CODED_TEXT", "value": "modification",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "251"
            }
        },
        "committer": { "_type": "PARTY_IDENTIFIED", "name": "seed" }
    });
    json!({
        "_type": "CONTRIBUTION",
        "versions": [ {
            "_type": "ORIGINAL_VERSION",
            "commit_audit": audit,
            "lifecycle_state": {
                "_type": "DV_CODED_TEXT", "value": "complete",
                "defining_code": {
                    "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                    "code_string": "532"
                }
            },
            "preceding_version_uid": {
                "value": format!("{access_vo}::{}::1", ferroehr::service::DEFAULT_SYSTEM_ID)
            },
            "data": {
                "_type": "EHR_ACCESS",
                "archetype_node_id": "openEHR-EHR-EHR_ACCESS.generic.v1",
                "archetype_details": {
                    "_type": "ARCHETYPED",
                    "archetype_id": { "_type": "ARCHETYPE_ID",
                                      "value": "openEHR-EHR-EHR_ACCESS.generic.v1" },
                    "rm_version": "1.2.0"
                },
                "name": { "_type": "DV_TEXT", "value": "EHR Access" },
                "settings": settings
            }
        } ],
        "audit": audit
    })
}

/// Sends `method path` as `user`, with an optional JSON body.
async fn send(
    app: &Router,
    method: &str,
    path: &str,
    user: &str,
    body: Option<Value>,
) -> (StatusCode, String) {
    let req = Request::builder()
        .method(method)
        .uri(common::full(path))
        .header(header::AUTHORIZATION, basic(user))
        .header(header::ACCEPT, "application/json");
    let req = match body {
        Some(body) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string())),
        None => req.body(Body::empty()),
    }
    .expect("request");
    common::send_body(app, req).await
}

/// `user` asks for the repair of `ehr` with `settings`.
async fn repair(app: &Router, user: &str, ehr: &str, settings: &Value) -> (StatusCode, String) {
    send(
        app,
        "POST",
        "/admin/ehr-access/repair",
        user,
        Some(json!({ "ehr_id": ehr, "settings": settings })),
    )
    .await
}

/// The `(action, principal)` of every stored access record on `resource`,
/// polled until `expected` have landed.
async fn trail(pool: &PgPool, resource: &str, expected: usize) -> Vec<(String, String)> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT action, principal FROM audit.audit_event \
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

/// The administrator's repair commits the third `EHR_ACCESS` version as a
/// modification under its own contribution, the EHR is served again under the
/// new settings, and the repair is an access record naming the administrator.
#[tokio::test]
async fn an_administrator_repairs_a_held_ehr() {
    let (_db, pool, app) = Box::pin(app()).await;
    let ehr = format!("/ehr/{HELD}");
    assert_eq!(
        send(&app, "GET", &ehr, "bob", None).await.0,
        StatusCode::INTERNAL_SERVER_ERROR,
        "the EHR is held before the repair"
    );

    let (status, body) = repair(&app, "root", HELD, &restricted_to(&pool, "carol").await).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let answer: Value = serde_json::from_str(&body).expect("a JSON answer");
    let version_uid = answer["version_uid"].as_str().expect("version_uid");
    assert!(version_uid.ends_with("::3"), "{answer}");
    assert_eq!(answer["ehr_id"], json!(HELD));

    let (status, body) = send(&app, "GET", &ehr, "carol", None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the new settings name carol their gate-keeper: {body}"
    );
    assert_eq!(
        send(&app, "GET", &ehr, "bob", None).await.0,
        StatusCode::FORBIDDEN,
        "the new settings no longer name bob"
    );

    let (change_type, contributions): (String, i64) = sqlx::query_as(
        "SELECT a.change_type, \
                (SELECT count(*) FROM contribution c WHERE c.ehr_id = v.ehr_id) \
         FROM version v JOIN commit_audit a ON a.id = v.commit_audit_id \
         WHERE v.kind = 'EHR_ACCESS' AND v.ehr_id = $1 AND v.sys_version = 3",
    )
    .bind(uuid::Uuid::parse_str(HELD).expect("an EHR id"))
    .fetch_one(&pool)
    .await
    .expect("the repaired version");
    assert_eq!(change_type, "251", "the repair is a modification");
    assert!(
        contributions >= 3,
        "the repair commits its own contribution"
    );

    let vo = version_uid.split("::").next().expect("the object id");
    assert_eq!(
        trail(&pool, &format!("ehr-access-repair:{vo}"), 1).await,
        [("U".to_owned(), "root".to_owned())]
    );
}

/// The repair is refused without the admin role, for an EHR that is not held,
/// for an unknown EHR, and for settings that do not parse; none of them
/// commits.
#[tokio::test]
async fn the_repair_is_refused_unless_an_administrator_repairs_a_held_ehr() {
    let (_db, pool, app) = Box::pin(app()).await;
    let fixed = restricted_to(&pool, "carol").await;

    let (status, body) = repair(&app, "bob", HELD, &fixed).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    let (status, body) = repair(&app, "root", OPEN, &fixed).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let (status, body) = repair(&app, "root", "66666666-7777-4888-9999-000000000000", &fixed).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let mut unreadable = fixed.clone();
    unreadable["default_access"] = json!("bogus");
    let (status, body) = repair(&app, "root", HELD, &unreadable).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, body) = repair(&app, "root", HELD, &json!({ "_type": "OTHER_SCHEME" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, body) = send(
        &app,
        "POST",
        "/admin/ehr-access/repair",
        "root",
        Some(json!({ "ehr_id": HELD })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let versions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM version WHERE kind = 'EHR_ACCESS' AND ehr_id = ANY($1)",
    )
    .bind(vec![
        uuid::Uuid::parse_str(HELD).expect("an EHR id"),
        uuid::Uuid::parse_str(OPEN).expect("an EHR id"),
    ])
    .fetch_one(&pool)
    .await
    .expect("count the versions");
    assert_eq!(versions, 3, "no refused repair commits");
    assert_eq!(
        send(&app, "GET", &format!("/ehr/{HELD}"), "bob", None)
            .await
            .0,
        StatusCode::INTERNAL_SERVER_ERROR,
        "the EHR stays held"
    );
}
