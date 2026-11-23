// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A request on a replica that has just started, with every copy cold, needs
//! at most one connection of each pool at a time: with pools of one
//! connection each, a burst of reads after the start is answered, never left
//! waiting for a connection another of its own statements holds, and each
//! read is answered `200` (#6345). No openEHR spec governs connection
//! handling: our own design (#5879).

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use ferroehr::config::rest::ApiConfig;
use ferroehr::db::DbConfig;
use ferroehr::db::domain::DomainPools;
use ferroehr::ids::EhrId;
use ferroehr::party_binding::grants::operation::Operation;
use ferroehr::service::FerroEhrService;
use ferroehr::service::version_update::WriteRequest;
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode};

use crate::common;

const EHR_ID: &str = "6e2f9a1b-7c3d-4e5f-9a0b-1c2d3e4f5a6b";

const USERS: &[(&str, &[&str])] = &[("bob", &[]), ("root", &["ADMIN"])];

/// A replica over pools of one connection each, its copies cold.
async fn cold_replica(db: &testkit::TestDb) -> Router {
    let mut settings = DbConfig::new(db.url());
    settings.max_connections = Some(1);
    settings.min_connections = Some(0);
    settings.connection_budget = 8;
    settings.acquire_timeout = ferrohealth_config::units::Duration::from_secs(5);
    let pools: DomainPools = ferroehr::db::connect_domains(&settings)
        .await
        .expect("the pools connect");
    let svc = Arc::new(FerroEhrService::new(&pools).with_binding_mapping(
        ferroehr::party_binding::BindingMapping::default().with_admin_role("ADMIN"),
    ));
    common::allowing(&svc, &[Operation::HoldRoles, Operation::HoldScopes]);
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
    common::build_full(config, svc, None, Observability::default()).await
}

#[tokio::test]
async fn a_cold_replica_answers_with_one_connection_per_pool() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let warm = Arc::new(
        FerroEhrService::new(&DomainPools::from_shared(&pool)).with_binding_mapping(
            ferroehr::party_binding::BindingMapping::default().with_admin_role("ADMIN"),
        ),
    );
    common::grant_basic_users(&warm, &pool, USERS).await;
    let bob = common::basic_person(&pool, "bob").await;
    let ehr_id: EhrId = EHR_ID.parse().expect("an EHR id");
    Box::pin(warm.create_ehr_with_id_meta(ehr_id, WriteRequest::new(None)))
        .await
        .expect("create the EHR");
    common::commit_access_settings(&warm, ehr_id, common::access_settings("open", &[bob])).await;

    let app = cold_replica(&db).await;
    for user in ["root", "bob", "root"] {
        let request = Request::builder()
            .uri(common::full(&format!("/ehr/{EHR_ID}")))
            .header("authorization", common::basic_credential(user))
            .body(Body::empty())
            .expect("a request");
        let status =
            tokio::time::timeout(Duration::from_secs(20), common::send_status(&app, request))
                .await
                .unwrap_or_else(|_| panic!("{user}'s read waited for a connection"));
        assert_eq!(status, StatusCode::OK, "{user}");
    }
}
