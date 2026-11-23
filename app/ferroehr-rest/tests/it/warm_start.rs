// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A replica that starts under a burst answers the burst without a refusal
//! once it is ready: the server's pools open every connection before it
//! answers ready, so no request of the first burst waits for a connection
//! to open while the admission bound fills behind it. No openEHR spec
//! governs connection handling: our own design (#5774).

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use ferroehr::config::rest::ApiConfig;
use ferroehr::db::DbConfig;
use ferroehr::db::domain::{Domain, DomainPools};
use ferroehr::ids::EhrId;
use ferroehr::party_binding::grants::operation::Operation;
use ferroehr::service::FerroEhrService;
use ferroehr::service::version_update::WriteRequest;
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode};
use sqlx::Connection as _;

use crate::common;

const USERS: &[(&str, &[&str])] = &[("bob", &[]), ("root", &["ADMIN"])];

/// The connections of each of the replica's pools, and the requests it
/// serves at once.
const POOL: u32 = 4;

/// The EHRs the burst reads, each kept by its reader and cold in the
/// replica's copies.
const EHRS: usize = 8;

/// The requests of the burst, all sent the moment the replica is ready.
const BURST: usize = 96;

/// A replica started the way the server starts: its pools under the
/// serving settings with every connection opened, then its router.
async fn started_replica(db: &testkit::TestDb) -> (Router, DomainPools) {
    let mut settings = DbConfig::new(db.url());
    settings.max_connections = Some(POOL);
    settings.connection_budget = 3 * POOL + 8;
    settings.acquire_timeout = ferrohealth_config::units::Duration::from_secs(10);
    let serving = settings.for_serving(POOL);
    let pools = ferroehr::db::connect_domains(&serving)
        .await
        .expect("the pools connect");
    ferroehr::db::pool::open_kept(&serving, &pools)
        .await
        .expect("every connection opens");
    let svc = Arc::new(FerroEhrService::new(&pools).with_binding_mapping(
        ferroehr::party_binding::BindingMapping::default().with_admin_role("ADMIN"),
    ));
    common::allowing(&svc, &[Operation::HoldRoles, Operation::HoldScopes]);
    let mut config = AppConfig {
        server: ferrohealth_server::section::ServerSection {
            max_concurrent_requests: std::num::NonZeroUsize::new(
                usize::try_from(POOL).expect("the pool fits usize"),
            )
            .expect("a pool of connections"),
            max_waiting_requests: BURST,
            admission_wait: ferrohealth_config::units::Duration::from_secs(30),
            ..common::server()
        },
        api: ApiConfig {
            openapi_ui: ferroehr::config::management::AccessLevel::Off,
            ..Default::default()
        },
        ..Default::default()
    };
    common::basic_users(&mut config, USERS);
    config.auth.shared.enabled = true;
    let app = common::build_full(config, svc, None, Observability::default()).await;
    (app, pools)
}

/// The sessions opened on the test database since `since`, other than the
/// watching one: the connections the replica opened after it was ready.
async fn opened_since(watch: &mut sqlx::PgConnection, since: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity \
         WHERE datname = current_database() AND backend_type = 'client backend' \
           AND pid <> pg_backend_pid() \
           AND backend_start > $1::timestamptz",
    )
    .bind(since)
    .fetch_one(watch)
    .await
    .expect("the sessions read")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_replica_started_under_a_burst_refuses_none_of_it_once_ready() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let warm = Arc::new(
        FerroEhrService::new(&DomainPools::from_shared(&pool)).with_binding_mapping(
            ferroehr::party_binding::BindingMapping::default().with_admin_role("ADMIN"),
        ),
    );
    common::grant_basic_users(&warm, &pool, USERS).await;
    let bob = common::basic_person(&pool, "bob").await;
    let mut ehrs = Vec::with_capacity(EHRS);
    for _ in 0..EHRS {
        let ehr_id = EhrId(uuid::Uuid::now_v7());
        Box::pin(warm.create_ehr_with_id_meta(ehr_id, WriteRequest::new(None)))
            .await
            .expect("create the EHR");
        common::commit_access_settings(&warm, ehr_id, common::access_settings("open", &[bob]))
            .await;
        ehrs.push(ehr_id);
    }
    let mut watch = sqlx::PgConnection::connect(db.url())
        .await
        .expect("the watching session connects");

    let (app, pools) = started_replica(&db).await;
    for domain in Domain::ALL {
        assert_eq!(
            pools.get(domain).size(),
            POOL,
            "{domain}: every connection is open when the replica is ready"
        );
    }
    let ready: String = sqlx::query_scalar("SELECT clock_timestamp()::text")
        .fetch_one(&mut watch)
        .await
        .expect("the database clock reads");

    let burst: Vec<_> = (0..BURST)
        .map(|n| {
            let app = app.clone();
            let ehr_id = ehrs.get(n % EHRS).copied().expect("an EHR of the set");
            tokio::spawn(async move {
                let request = Request::builder()
                    .uri(common::full(&format!("/ehr/{ehr_id}")))
                    .header("authorization", common::basic_credential("bob"))
                    .body(Body::empty())
                    .expect("a request");
                common::send_status(&app, request).await
            })
        })
        .collect();
    let mut refused = 0_usize;
    let mut other = Vec::new();
    for answer in burst {
        let status = tokio::time::timeout(Duration::from_secs(60), answer)
            .await
            .expect("the burst is answered")
            .expect("the request task ends");
        if status == StatusCode::SERVICE_UNAVAILABLE {
            refused += 1;
        } else if status != StatusCode::OK {
            other.push(status);
        }
    }
    assert_eq!(refused, 0, "no request of the burst is refused 503");
    assert!(other.is_empty(), "every read is answered 200: {other:?}");
    assert_eq!(
        opened_since(&mut watch, &ready).await,
        0,
        "the replica opens no connection after it is ready"
    );
    pools.close().await;
}
