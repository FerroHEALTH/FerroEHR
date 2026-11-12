// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The operational-policy admin registers at the wire
//! (`/admin/registers/{register}`, #5442): the audit category map, the EHDS
//! Art. 3(3) access delay and the Cedar policy set.
//!
//! Each register is read, written whole over the version `If-Match` names and
//! listed by version, by `admin_role` only; a change decides the next request
//! on every replica, with no restart; and a Cedar policy set that does not
//! parse or validate is refused and never replaces the set in force. No
//! openEHR spec governs operational policy: our own design/extension.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use ferroehr::config::authz::AuthzConfig;
use ferroehr::service::FerroEhrService;
use ferroehr_rest::extensions::access::authz::{AuthzHandle, build_engine};
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common;
use crate::common::{BASE, EXT};

/// One replica of a deployment over `pool`: `root` holds `admin_role`, `dr`
/// does not, and ABAC runs the embedded Cedar engine over the Cedar register
/// when `abac`.
async fn replica(pool: &PgPool, abac: bool) -> (Arc<FerroEhrService>, Router) {
    let svc = Arc::new(FerroEhrService::new(
        &ferroehr::db::domain::DomainPools::from_shared(pool),
    ));
    svc.policy_registers()
        .set_cedar_check(ferroehr_rest::extensions::access::authz::cedar::check());
    let mut config = common::api_config(true);
    common::with_basic_users(&mut config, &[("dr", &["USER"]), ("root", &["ADMIN"])]);
    let mut authz = AuthzConfig::default();
    authz.abac.enabled = abac;
    // No patient claim, so the Cedar policy set alone decides a request.
    authz.abac.patient_claim = String::new();
    let engine = build_engine(&authz.abac, svc.policy_registers()).expect("the ABAC engine");
    let handle = AuthzHandle::build(
        &authz,
        &config.api.base_path,
        engine,
        common::null_resolvers(),
    )
    .map(Arc::new);
    listening(&svc).await;
    let app = ferroehr_rest::build_full(config, Arc::clone(&svc), handle, Observability::default())
        .await
        .expect("build app");
    (svc, app)
}

/// Start `svc`'s register listener, as the binary does at boot, and wait
/// until it is connected.
async fn listening(svc: &Arc<FerroEhrService>) {
    svc.policy_registers().spawn_listener();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !svc.policy_registers().is_listening() {
        assert!(
            std::time::Instant::now() < deadline,
            "the register listener did not connect"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Wait until `svc` has received a register notification beyond `seen`: a
/// write on another replica reaches this one when its commit notifies.
async fn delivered(svc: &Arc<FerroEhrService>, seen: u64) {
    assert!(
        svc.policy_registers()
            .notified_after(seen, Duration::from_secs(10))
            .await,
        "the register write's notification did not arrive"
    );
}

/// `method` on the register `name` (with `tail` after it) as `user`, with an
/// optional `If-Match` and JSON body.
async fn call(
    app: &Router,
    method: &str,
    user: &str,
    name: &str,
    tail: &str,
    if_match: Option<&str>,
    body: Option<&Value>,
) -> (StatusCode, header::HeaderMap, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(format!("{EXT}/admin/registers/{name}{tail}"))
        .header(header::AUTHORIZATION, common::basic_credential(user))
        .header(header::ACCEPT, "application/json");
    if let Some(version) = if_match {
        req = req.header(header::IF_MATCH, version);
    }
    let req = match body {
        Some(body) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string())),
        None => req.body(Body::empty()),
    }
    .unwrap();
    let (status, headers, text) = common::send(app, req).await;
    let value = serde_json::from_str(&text).unwrap_or(Value::String(text));
    (status, headers, value)
}

/// Read register `name` as the admin.
async fn read(app: &Router, name: &str) -> (StatusCode, header::HeaderMap, Value) {
    call(app, "GET", "root", name, "", None, None).await
}

/// Write register `name` as the admin over version `over`.
async fn write(app: &Router, name: &str, over: &str, document: &Value) -> (StatusCode, Value) {
    let (status, _, body) = call(app, "PUT", "root", name, "", Some(over), Some(document)).await;
    (status, body)
}

/// A document of each register that holds, as `(name, document)`.
fn documents() -> [(&'static str, Value); 3] {
    [
        (
            "audit-categories",
            json!({ "national": ["urn:oid:2.999.1|NAT"] }),
        ),
        (
            "access-delay",
            json!({ "enabled": true, "categories": ["Laboratory-Reports"], "max_delay": "3d" }),
        ),
        (
            "cedar-policies",
            json!({ "policies": "permit(principal, action, resource);" }),
        ),
    ]
}

/// Every register reads at version 0, its default, takes a write over that
/// version, answers the new version with its `ETag` and committer, lists both
/// versions in its history, and refuses a write over a version that is no
/// longer the newest (`412`), a write without `If-Match` (`400`) and an
/// unknown register (`404`).
#[tokio::test]
async fn every_register_is_read_written_and_versioned() {
    let (_db, pool) = common::migrated_pool().await;
    let (_svc, app) = replica(&pool, false).await;
    for (name, document) in documents() {
        let (status, headers, body) = read(&app, name).await;
        assert_eq!(status, StatusCode::OK, "{name}: {body}");
        assert_eq!(common::header(&headers, header::ETAG), Some("\"0\""));
        assert_eq!(body["version"], 0, "{name}: {body}");
        assert_eq!(body["committer"], Value::Null);

        let (status, body) = write(&app, name, "\"0\"", &document).await;
        assert_eq!(status, StatusCode::OK, "{name}: {body}");
        assert_eq!(body["version"], 1, "{name}: {body}");
        assert_eq!(body["committer"], "root", "{name}: {body}");
        let (_, headers, body) = read(&app, name).await;
        assert_eq!(common::header(&headers, header::ETAG), Some("\"1\""));
        for (key, value) in document.as_object().expect("an object") {
            assert_eq!(&body["document"][key], value, "{name}.{key}: {body}");
        }

        let (status, body) = write(&app, name, "\"0\"", &document).await;
        assert_eq!(status, StatusCode::PRECONDITION_FAILED, "{name}: {body}");
        let (status, _, body) = call(&app, "PUT", "root", name, "", None, Some(&document)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name}: {body}");

        let (status, _, body) = call(&app, "GET", "root", name, "/history", None, None).await;
        assert_eq!(status, StatusCode::OK, "{name}: {body}");
        let versions: Vec<i64> = body["versions"]
            .as_array()
            .expect("versions")
            .iter()
            .map(|version| version["version"].as_i64().expect("a version"))
            .collect();
        assert_eq!(versions, [1, 0], "{name}: {body}");
    }
    let (status, _, _) = read(&app, "categories").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A caller without `admin_role` neither reads, writes nor lists a register.
#[tokio::test]
async fn a_caller_without_the_admin_role_is_refused() {
    let (_db, pool) = common::migrated_pool().await;
    let (_svc, app) = replica(&pool, false).await;
    for (name, document) in documents() {
        for (method, tail, body) in [
            ("GET", "", None),
            ("PUT", "", Some(&document)),
            ("GET", "/history", None),
        ] {
            let (status, _, answer) =
                call(&app, method, "dr", name, tail, Some("\"0\""), body).await;
            assert_eq!(
                status,
                StatusCode::FORBIDDEN,
                "{method} {name}{tail}: {answer}"
            );
        }
        let (_, _, body) = read(&app, name).await;
        assert_eq!(body["version"], 0, "the refused write left {name} alone");
    }
}

/// A category declared on one replica is one a retention period may name on
/// the next request to another replica, with no restart.
#[tokio::test]
async fn a_category_change_applies_on_the_next_request() {
    let (_db, pool) = common::migrated_pool().await;
    let (_a, writer) = replica(&pool, false).await;
    let (b, reader) = replica(&pool, false).await;
    let period = json!({
        "kind": "COMPOSITION",
        "jurisdiction": "DE",
        "category": "urn:oid:2.999.1|NAT",
        "period": "10 years",
        "anchor": "last_commit",
        "source": "a national rule"
    });
    let retention = |app: Router| {
        let period = period.to_string();
        async move {
            common::send_body(
                &app,
                Request::put(format!("{EXT}/admin/retention/policy"))
                    .header(header::AUTHORIZATION, common::basic_credential("root"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(period))
                    .unwrap(),
            )
            .await
        }
    };
    let (status, body) = retention(reader.clone()).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "no such category yet: {body}"
    );
    let seen = b.policy_registers().notifications();
    let (status, body) = write(
        &writer,
        "audit-categories",
        "\"0\"",
        &json!({ "national": ["urn:oid:2.999.1|NAT"] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    delivered(&b, seen).await;
    let (status, body) = retention(reader).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "the next request knows it: {body}"
    );
}

/// The access delay turned on through one replica lets a professional place a
/// hold through another on the next request, and turned off refuses it again.
#[tokio::test]
async fn an_access_delay_change_applies_on_the_next_request() {
    let (_db, pool) = common::migrated_pool().await;
    let (_a, writer) = replica(&pool, false).await;
    let (b, reader) = replica(&pool, false).await;
    let hold = |app: Router| async move {
        common::send_body(
            &app,
            Request::post(format!("{EXT}/admin/access-delay"))
                .header(header::AUTHORIZATION, common::basic_credential("root"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "ehr_id": "7d44b88c-4199-4bad-97dc-d78268e01398",
                        "vo_id": "8849182c-82ad-4088-a07f-48ead4180515"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
    };
    let (status, body) = hold(reader.clone()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "the delay is off: {body}");
    let on = json!({ "enabled": true, "categories": [], "max_delay": "72h" });
    let seen = b.policy_registers().notifications();
    let (status, body) = write(&writer, "access-delay", "\"0\"", &on).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    delivered(&b, seen).await;
    let (status, body) = hold(reader.clone()).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the delay is on, so the hold reaches the EHR it names: {body}"
    );
    let seen = b.policy_registers().notifications();
    let (status, body) = write(
        &writer,
        "access-delay",
        "\"1\"",
        &json!({ "enabled": false }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    delivered(&b, seen).await;
    let (status, body) = hold(reader).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "the delay is off again: {body}"
    );
}

/// The Cedar policy set written through one replica decides the next request
/// to another: the default set permits nothing, a permitting set admits the
/// request, and a set that does not parse or validate is refused and leaves
/// the set in force.
#[tokio::test]
async fn a_cedar_change_applies_on_the_next_request_and_a_bad_set_is_refused() {
    let (_db, pool) = common::migrated_pool().await;
    let (_a, writer) = replica(&pool, true).await;
    let (b, reader) = replica(&pool, true).await;
    let create = |app: Router| async move {
        common::send_body(
            &app,
            Request::post(format!("{BASE}/ehr"))
                .header(header::AUTHORIZATION, common::basic_credential("dr"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
    };
    let (status, body) = create(reader.clone()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "no policy permits: {body}");

    let permit = json!({ "policies": "permit(principal, action, resource);" });
    let seen = b.policy_registers().notifications();
    let (status, body) = write(&writer, "cedar-policies", "\"0\"", &permit).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    delivered(&b, seen).await;
    let (status, body) = create(reader.clone()).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the next request is permitted: {body}"
    );

    for bad in [
        "this is not a cedar policy",
        "permit(principal, action == Action::\"no_such_action\", resource);",
    ] {
        let (status, body) = write(
            &writer,
            "cedar-policies",
            "\"1\"",
            &json!({ "policies": bad }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
    }
    let (_, headers, _) = read(&reader, "cedar-policies").await;
    assert_eq!(common::header(&headers, header::ETAG), Some("\"1\""));
    let (status, body) = create(reader).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the set in force stays: {body}"
    );
}

/// A replica whose listener has lost its connection reads the registers on
/// every request until it connects again, so a write it was never notified
/// of applies on its next request rather than an outdated policy.
#[tokio::test]
async fn a_lost_listener_falls_back_to_a_read_on_every_request() {
    let (_db, pool) = common::migrated_pool().await;
    let (_a, writer) = replica(&pool, false).await;
    let (b, reader) = replica(&pool, false).await;
    let listeners: Vec<i32> = sqlx::query_scalar(
        "SELECT pid FROM pg_stat_activity WHERE datname = current_database() \
         AND query LIKE 'LISTEN %'",
    )
    .fetch_all(&pool)
    .await
    .expect("the listening sessions");
    assert_eq!(listeners.len(), 2, "one listener per replica");
    for pid in listeners {
        sqlx::query("SELECT pg_terminate_backend($1)")
            .bind(pid)
            .execute(&pool)
            .await
            .expect("drop a listener's connection");
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while b.policy_registers().is_listening() {
        assert!(
            std::time::Instant::now() < deadline,
            "the replica did not notice its lost listener"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let (status, body) = write(
        &writer,
        "access-delay",
        "\"0\"",
        &json!({ "enabled": true, "categories": [], "max_delay": "72h" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = common::send_body(
        &reader,
        Request::post(format!("{EXT}/admin/access-delay"))
            .header(header::AUTHORIZATION, common::basic_credential("root"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({
                    "ehr_id": "7d44b88c-4199-4bad-97dc-d78268e01398",
                    "vo_id": "8849182c-82ad-4088-a07f-48ead4180515"
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the delay is on for the replica's next request: {body}"
    );
    assert_eq!(
        b.policy_registers().access_delay().version,
        1,
        "with its listener down the replica read the registers on that request"
    );
}
