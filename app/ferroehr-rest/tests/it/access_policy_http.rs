// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The controller's `ACCESS_POLICY` at the wire (`/admin/access_policy`,
//! #5512): read with its version as the `ETag`, written whole over the
//! version `If-Match` names or created with `If-None-Match: *`, listed by
//! version, by `admin_role` only; a change decides the next request on every
//! replica, with no restart. No openEHR spec governs the access policy: our
//! own design/extension.

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

const ARCHETYPE: &str = "openEHR-EHR-ACCESS_POLICY.controller.v1";

/// One replica of a deployment over `pool`: `root` holds `admin_role`, `dr`
/// does not. Its access policy listener runs when `listening`.
async fn replica(pool: &PgPool, listening: bool) -> (Arc<FerroEhrService>, Router) {
    let svc = Arc::new(FerroEhrService::new(
        &ferroehr::db::domain::DomainPools::from_shared(pool),
    ));
    let mut config = common::api_config(true);
    common::with_basic_users(
        &mut config,
        &svc,
        pool,
        &[("dr", &["USER"]), ("root", &["ADMIN"])],
    )
    .await;
    if listening {
        let listened = svc.access_policy().listened();
        listened.spawn_listener();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !listened.is_listening() {
            assert!(
                std::time::Instant::now() < deadline,
                "the access policy listener did not connect"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    let authz = AuthzConfig::default();
    let engine = build_engine(&authz.abac, svc.policy_registers()).expect("the ABAC engine");
    let handle = AuthzHandle::build(
        &authz,
        &config.api.base_path,
        engine,
        common::null_resolvers(),
    )
    .map(Arc::new);
    let app = common::build_full(config, Arc::clone(&svc), handle, Observability::default()).await;
    (svc, app)
}

/// A policy over the default archetype, with `care` as its care relationship
/// requirement.
fn policy(care: bool) -> Value {
    json!({
        "_type": "ACCESS_POLICY",
        "name": { "_type": "DV_TEXT", "value": "Hospital access policy" },
        "archetype_node_id": ARCHETYPE,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": ARCHETYPE },
            "rm_version": "1.2.0" },
        "controller": { "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
            "id": { "_type": "HIER_OBJECT_ID", "value": "2c3d4e5f-6a7b-4c8d-9e0f-a1b2c3d4e5f7" } },
        "care_relationship_required": care,
        "purpose_required": false
    })
}

/// `method` on `/admin/access_policy{tail}` as `user`, with the conditional
/// header `condition` and an optional JSON body.
async fn call(
    app: &Router,
    method: &str,
    user: &str,
    tail: &str,
    condition: Option<(header::HeaderName, &str)>,
    body: Option<&Value>,
) -> (StatusCode, header::HeaderMap, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(format!("{EXT}/admin/access_policy{tail}"))
        .header(header::AUTHORIZATION, common::basic_credential(user))
        .header(header::ACCEPT, "application/json");
    if let Some((name, value)) = condition {
        req = req.header(name, value);
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

/// Create the first version as the admin; its `ETag`.
async fn create(app: &Router, body: &Value) -> String {
    let (status, headers, answer) = call(
        app,
        "PUT",
        "root",
        "",
        Some((header::IF_NONE_MATCH, "*")),
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{answer}");
    common::header(&headers, header::ETAG)
        .expect("an ETag")
        .to_owned()
}

/// The policy is read, created, replaced over its `ETag` and listed by
/// version, and a stale, missing or doubled precondition is refused.
#[tokio::test]
async fn the_policy_is_read_written_and_versioned() {
    let (_db, pool) = common::migrated_pool().await;
    let (_svc, app) = replica(&pool, false).await;
    let (status, _, body) = call(&app, "GET", "root", "", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (status, _, body) = call(&app, "GET", "root", "/history", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));

    let first = create(&app, &policy(false)).await;
    assert!(
        first.starts_with("W/\"") && first.ends_with("::1\""),
        "{first}"
    );
    let (status, headers, body) = call(&app, "GET", "root", "", None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(common::header(&headers, header::ETAG), Some(first.as_str()));
    assert!(common::header(&headers, header::LAST_MODIFIED).is_some());
    assert_eq!(body["care_relationship_required"], false);

    let (status, headers, body) = call(
        &app,
        "PUT",
        "root",
        "",
        Some((header::IF_MATCH, &first)),
        Some(&policy(true)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["care_relationship_required"], true);
    let second = common::header(&headers, header::ETAG)
        .expect("an ETag")
        .to_owned();
    assert!(second.ends_with("::2\""), "{second}");

    for condition in [
        Some((header::IF_MATCH, first.as_str())),
        Some((header::IF_NONE_MATCH, "*")),
    ] {
        let (status, _, body) =
            call(&app, "PUT", "root", "", condition, Some(&policy(false))).await;
        assert_eq!(status, StatusCode::PRECONDITION_FAILED, "{body}");
    }
    let (status, _, body) = call(&app, "PUT", "root", "", None, Some(&policy(false))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, _, body) = call(&app, "GET", "root", "/history", None, None).await;
    assert_eq!(status, StatusCode::OK);
    let uids: Vec<&str> = body
        .as_array()
        .expect("the versions")
        .iter()
        .map(|v| v["uid"]["value"].as_str().expect("a uid"))
        .collect();
    assert_eq!(uids.len(), 2, "{body}");
    assert!(
        second.contains(uids[0]) && first.contains(uids[1]),
        "{uids:?}"
    );
    assert_eq!(
        body[0]["commit_audit"]["committer"]["_type"],
        "PARTY_IDENTIFIED"
    );

    let (status, headers, body) = call(&app, "GET", "root", "/versioned", None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["_type"], "VERSIONED_ACCESS_POLICY");
    let uid = body["uid"]["value"].as_str().expect("a uid");
    assert_eq!(
        common::header(&headers, header::ETAG),
        Some(format!("W/\"{uid}\"").as_str())
    );
}

/// A body that is no access policy, or breaks the draft BMM, is refused whole
/// and the version in force stays.
#[tokio::test]
async fn a_policy_that_does_not_hold_is_refused_whole() {
    let (_db, pool) = common::migrated_pool().await;
    let (_svc, app) = replica(&pool, false).await;
    let first = create(&app, &policy(false)).await;
    let mut negative_grace = policy(true);
    negative_grace["relationship_grace"] = json!({ "_type": "DV_DURATION", "value": "-P1D" });
    let mut unarchetyped = policy(true);
    unarchetyped
        .as_object_mut()
        .expect("an object")
        .remove("archetype_details");
    for body in [
        json!({ "_type": "EHR_STATUS" }),
        negative_grace,
        unarchetyped,
    ] {
        let (status, _, answer) = call(
            &app,
            "PUT",
            "root",
            "",
            Some((header::IF_MATCH, &first)),
            Some(&body),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {answer}");
    }
    let (_, headers, body) = call(&app, "GET", "root", "", None, None).await;
    assert_eq!(common::header(&headers, header::ETAG), Some(first.as_str()));
    assert_eq!(body["care_relationship_required"], false);
}

/// A caller without `admin_role` neither reads, writes nor lists the policy.
#[tokio::test]
async fn a_caller_without_the_admin_role_is_refused() {
    let (_db, pool) = common::migrated_pool().await;
    let (_svc, app) = replica(&pool, false).await;
    for (method, tail, body) in [
        ("GET", "", None),
        ("PUT", "", Some(policy(false))),
        ("GET", "/history", None),
        ("GET", "/versioned", None),
    ] {
        let (status, _, answer) = call(
            &app,
            method,
            "dr",
            tail,
            Some((header::IF_NONE_MATCH, "*")),
            body.as_ref(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {tail}: {answer}");
    }
    let (status, _, _) = call(&app, "GET", "root", "", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "the refused write left none");
}

/// A request on another replica after the write's notification arrives runs
/// under the new version.
#[tokio::test]
async fn a_change_decides_the_next_request_on_another_replica() {
    let (_db, pool) = common::migrated_pool().await;
    let (_a, writer) = replica(&pool, true).await;
    let (b, reader) = replica(&pool, true).await;
    let next_request = || async {
        common::send_status(
            &reader,
            Request::get(format!("{BASE}/ehr/7d44b88c-4199-4bad-97dc-d78268e01398"))
                .header(header::AUTHORIZATION, common::basic_credential("root"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
    };
    next_request().await;
    assert_eq!(b.access_policy().in_force().version, 0);
    let seen = b.access_policy().listened().notifications();
    create(&writer, &policy(true)).await;
    assert!(
        b.access_policy()
            .listened()
            .notified_after(seen, Duration::from_secs(10))
            .await,
        "the write's notification did not arrive"
    );
    next_request().await;
    let in_force = b.access_policy().in_force();
    assert_eq!(in_force.version, 1, "the next request read the new version");
    assert!(
        in_force
            .policy
            .as_ref()
            .is_some_and(|p| p.care_relationship_required)
    );
}

/// A replica whose listener lost its connection reads the policy on its next
/// request.
#[tokio::test]
async fn a_replica_that_lost_its_listener_reads_on_the_next_request() {
    let (_db, pool) = common::migrated_pool().await;
    let (_a, writer) = replica(&pool, false).await;
    let (b, reader) = replica(&pool, true).await;
    let listeners: Vec<i32> = sqlx::query_scalar(
        "SELECT pid FROM pg_stat_activity WHERE datname = current_database() \
         AND query LIKE 'LISTEN %'",
    )
    .fetch_all(&pool)
    .await
    .expect("the listening sessions");
    assert_eq!(listeners.len(), 1, "the reader's listener");
    for pid in listeners {
        sqlx::query("SELECT pg_terminate_backend($1)")
            .bind(pid)
            .execute(&pool)
            .await
            .expect("drop the listener's connection");
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while b.access_policy().listened().is_listening() {
        assert!(
            std::time::Instant::now() < deadline,
            "the replica did not notice its lost listener"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    create(&writer, &policy(true)).await;
    common::send_status(
        &reader,
        Request::get(format!("{BASE}/ehr/7d44b88c-4199-4bad-97dc-d78268e01398"))
            .header(header::AUTHORIZATION, common::basic_credential("root"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        b.access_policy().in_force().version,
        1,
        "with its listener down the replica read the policy on that request"
    );
}
