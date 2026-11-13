// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `EHR_ACCESS` settings across the replicas of one deployment (#5509).
//!
//! Every change to an EHR's access settings is a version committed in a
//! contribution (RM ehr `org.openehr.rm.ehr.ehr_access.adoc`; RM common
//! change control), and each replica holds a copy of the settings it decides
//! by. The tests assert that a commit on one replica decides the next request
//! on another once its notification arrives; that a replica whose listener is
//! down reads the settings on every request, so a change it was never
//! notified of decides its next request; and that a settings store that
//! cannot answer is `503`. No openEHR spec governs the copy: our own design.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use ferroehr::config::rest::ApiConfig;
use ferroehr::error::ServiceError;
use ferroehr::ids::EhrId;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::service::{DEFAULT_SYSTEM_ID, FerroEhrService};
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common;

const EHR_ID: &str = "5d1e8f0a-6b2c-4d3e-8f9a-0b1c2d3e4f5a";

/// The Basic users: two granted no role and the administrator.
const USERS: &[(&str, &[&str])] = &[("bob", &[]), ("carol", &[]), ("root", &["ADMIN"])];

/// One replica of a deployment over `pool`, its `EHR_ACCESS` listener
/// connected when `listening`.
async fn replica(pool: &PgPool, listening: bool) -> (Arc<FerroEhrService>, Router) {
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool))
            .with_binding_mapping(
                ferroehr::party_binding::BindingMapping::default().with_admin_role("ADMIN"),
            ),
    );
    common::grant_basic_users(&svc, pool, USERS).await;
    if listening {
        svc.ehr_access_listened().spawn_listener();
        wait_until(
            || svc.ehr_access_listened().is_listening(),
            "the listener connects",
        )
        .await;
    }
    let mut config = AppConfig {
        server: common::server(),
        api: ApiConfig {
            openapi_ui: ferroehr::config::management::AccessLevel::Off,
            ..Default::default()
        },
        ..Default::default()
    };
    common::basic_users(&mut config, USERS);
    config.auth.shared.enabled = true;
    let app = common::build_full(config, Arc::clone(&svc), None, Observability::default()).await;
    (svc, app)
}

/// Waits until `holds`, at most ten seconds, naming `what` on a timeout.
async fn wait_until(holds: impl Fn() -> bool, what: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !holds() {
        assert!(std::time::Instant::now() < deadline, "{what}: timed out");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The EHR, created through `svc`, with settings restricted to `bob` as its
/// gate-keeper.
async fn restricted_to_bob(svc: &FerroEhrService, app: &Router, pool: &PgPool) -> Value {
    let ehr_id: EhrId = EHR_ID.parse().expect("an EHR id");
    Box::pin(svc.create_ehr_with_id_meta(ehr_id, WriteRequest::new(None)))
        .await
        .expect("create the EHR");
    let ehr = svc.ehr_read(ehr_id).await.expect("the EHR").body;
    let bob = common::basic_person(pool, "bob").await;
    commit(app, &ehr, &common::access_settings("restricted", &[bob]), 1).await;
    ehr
}

/// Commits `settings` over the `EHR_ACCESS` version `preceding` of `ehr`, as
/// the administrator through `app`.
async fn commit(app: &Router, ehr: &Value, settings: &Value, preceding: u32) {
    let access_vo = ehr["ehr_access"]["id"]["value"]
        .as_str()
        .expect("EHR.ehr_access.id.value");
    let coded = |code: &str, value: &str| {
        json!({ "_type": "DV_CODED_TEXT", "value": value,
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                    "code_string": code } })
    };
    let committer = json!({ "_type": "PARTY_IDENTIFIED",
        "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": "ferroehr", "id": "root" }] });
    let contribution = json!({
        "_type": "CONTRIBUTION",
        "versions": [{
            "_type": "ORIGINAL_VERSION",
            "commit_audit": { "change_type": coded("251", "modification"), "committer": committer },
            "lifecycle_state": coded("532", "complete"),
            "preceding_version_uid": {
                "value": format!("{access_vo}::{DEFAULT_SYSTEM_ID}::{preceding}")
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
        }],
        "audit": { "change_type": coded("251", "modification"), "committer": committer }
    });
    let (code, _headers, body) = common::send(
        app,
        request(&format!("/ehr/{EHR_ID}/contribution"), "root")
            .method("POST")
            .body(Body::from(contribution.to_string()))
            .expect("a request"),
    )
    .await;
    assert_eq!(code, StatusCode::CREATED, "the settings commit: {body}");
}

/// A request builder for `path` as the Basic user `user`.
fn request(path: &str, user: &str) -> http::request::Builder {
    Request::builder()
        .uri(common::full(path))
        .header("content-type", "application/json")
        .header("authorization", common::basic_credential(user))
}

/// The status of `bob`'s read of the EHR through `app`.
async fn bob_reads(app: &Router) -> StatusCode {
    common::send_status(
        app,
        request(&format!("/ehr/{EHR_ID}"), "bob")
            .body(Body::empty())
            .expect("a request"),
    )
    .await
}

/// A gate-keeper revoked on one replica is refused on the other at its next
/// request after the commit's notification, though the other held the
/// settings that admitted him.
#[tokio::test]
async fn a_revoked_gate_keeper_is_refused_on_the_next_request_of_another_replica() {
    let (_db, pool) = common::migrated_pool().await;
    let (a, writer) = replica(&pool, true).await;
    let (b, reader) = replica(&pool, true).await;
    let ehr = restricted_to_bob(&a, &writer, &pool).await;
    assert_eq!(
        bob_reads(&reader).await,
        StatusCode::OK,
        "the gate-keeper reads the EHR on the other replica"
    );
    let seen = b.ehr_access_listened().notifications();
    let carol = common::basic_person(&pool, "carol").await;
    commit(
        &writer,
        &ehr,
        &common::access_settings("restricted", &[carol]),
        2,
    )
    .await;
    assert!(
        b.ehr_access_listened()
            .notified_after(seen, Duration::from_secs(10))
            .await,
        "the commit's notification reaches the other replica"
    );
    assert_eq!(
        bob_reads(&reader).await,
        StatusCode::FORBIDDEN,
        "the revoked gate-keeper is refused on the other replica's next request"
    );
    assert_eq!(
        bob_reads(&writer).await,
        StatusCode::FORBIDDEN,
        "and on the committing replica"
    );
}

/// A replica whose listener has lost its connection reads the settings on
/// every request, so a revocation it was never notified of decides its next
/// request.
#[tokio::test]
async fn a_lost_listener_reads_the_settings_on_every_request() {
    let (_db, pool) = common::migrated_pool().await;
    let (a, writer) = replica(&pool, true).await;
    let (b, reader) = replica(&pool, true).await;
    let ehr = restricted_to_bob(&a, &writer, &pool).await;
    let listeners: Vec<i32> = sqlx::query_scalar(
        "SELECT pid FROM pg_stat_activity WHERE datname = current_database() \
         AND query LIKE 'LISTEN %ferroehr_ehr_access%'",
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
    wait_until(
        || !b.ehr_access_listened().is_listening(),
        "the replica notices its lost listener",
    )
    .await;
    assert_eq!(
        bob_reads(&reader).await,
        StatusCode::OK,
        "the gate-keeper reads the EHR while the listener is down"
    );
    let carol = common::basic_person(&pool, "carol").await;
    commit(
        &writer,
        &ehr,
        &common::access_settings("restricted", &[carol]),
        2,
    )
    .await;
    assert_eq!(
        bob_reads(&reader).await,
        StatusCode::FORBIDDEN,
        "the revocation decides the replica's next request"
    );
    assert!(
        !b.ehr_access_listened().is_listening(),
        "the replica decided with its listener down"
    );
}

/// A settings store that cannot answer refuses the request `503`, and the
/// service reports it as the server's overload.
#[tokio::test]
async fn a_settings_store_that_cannot_answer_is_503() {
    let (_db, pool) = common::migrated_pool().await;
    let pools = ferroehr::db::domain::DomainPools::from_shared(&pool);
    pools.close().await;
    let svc = Arc::new(FerroEhrService::new(&pools));
    let ehr_id: EhrId = EHR_ID.parse().expect("an EHR id");
    assert!(
        matches!(
            svc.current_ehr_access_settings(ehr_id).await,
            Err(ServiceError::Overloaded(_))
        ),
        "the settings read reports the overload"
    );
    let mut config = AppConfig {
        server: common::server(),
        ..Default::default()
    };
    config.auth.shared.enabled = false;
    let app = common::build_full(config, svc, None, Observability::default()).await;
    let code = common::send_status(
        &app,
        Request::builder()
            .uri(common::full(&format!("/ehr/{EHR_ID}")))
            .body(Body::empty())
            .expect("a request"),
    )
    .await;
    assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
}
