// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A request holds its pooled connection only while its statements run
//! (#5774): the request scope keeps the connection between its statements
//! and gives it back at `release_held`, which the HTTP layer calls when the
//! handler has answered; and the server opens the connections each pool
//! keeps before it answers ready.
//!
//! No openEHR spec governs connection handling: our own design.

use sqlx::PgPool;

use ferroehr::db::DbConfig;
use ferroehr::db::acquire::{acquire, awaiting, release_held, request_scope};
use ferroehr::db::domain::Domain;

/// Connections checked out of `pool` right now.
fn in_use(pool: &PgPool) -> u32 {
    let idle = u32::try_from(pool.num_idle()).expect("an idle count fits u32");
    pool.size().saturating_sub(idle)
}

/// Connections checked out of `pool` once every connection given back has
/// reached it: the pool takes a dropped connection back on a task of its
/// own, so wait up to two seconds for `expected`.
async fn settled_in_use(pool: &PgPool, expected: u32) -> u32 {
    for _ in 0..100 {
        if in_use(pool) == expected {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    in_use(pool)
}

/// A request keeps its connection between two statements, gives it back to
/// the pool at `release_held` while the request goes on, and checks one out
/// again for a statement after it.
#[tokio::test]
async fn a_request_gives_its_connection_back_once_its_handler_answered() {
    let db = testkit::db().await.expect("testkit database");
    let mut config = DbConfig::new(db.url());
    config.max_connections = Some(2);
    config.min_connections = Some(0);
    let pool = ferroehr::db::connect_domain(&config, Domain::Clinical)
        .await
        .expect("the pool connects");

    request_scope(async {
        let first: i32 = sqlx::query_scalar("SELECT 1")
            .fetch_one(&mut *acquire(&pool).await.expect("a connection"))
            .await
            .expect("the first statement");
        assert_eq!(first, 1);
        assert_eq!(
            in_use(&pool),
            1,
            "the scope keeps the connection between statements"
        );

        release_held();
        assert_eq!(
            settled_in_use(&pool, 0).await,
            0,
            "the answered request holds no connection"
        );

        let again: i32 = sqlx::query_scalar("SELECT 2")
            .fetch_one(&mut *acquire(&pool).await.expect("a connection again"))
            .await
            .expect("a statement after the release");
        assert_eq!(again, 2);
        assert_eq!(in_use(&pool), 1);
    })
    .await;
    assert_eq!(
        settled_in_use(&pool, 0).await,
        0,
        "the scope's end gives back what it kept"
    );
    pool.close().await;
}

/// `open_kept` has opened every connection each pool keeps when it returns,
/// before the server would answer ready.
#[tokio::test]
async fn the_server_opens_every_kept_connection_before_it_serves() {
    let db = testkit::db().await.expect("testkit database");
    let mut config = DbConfig::new(db.url());
    config.max_connections = Some(3);
    config.min_connections = Some(3);
    let serving = config.for_serving(256);
    assert_eq!(serving.pool_kept(), 3);
    let pools = ferroehr::db::connect_domains(&serving)
        .await
        .expect("the pools connect");
    ferroehr::db::pool::open_kept(&serving, &pools)
        .await
        .expect("the kept connections open");
    for domain in Domain::ALL {
        let pool = pools.get(domain);
        assert_eq!(pool.size(), 3, "{domain}");
        assert_eq!(
            settled_in_use(pool, 0).await,
            0,
            "{domain}: every connection is back in the pool"
        );
    }
    pools.close().await;
}

/// Two pools of one connection each, as the server opens them.
async fn one_connection_pools(db: &testkit::TestDb) -> ferroehr::db::domain::DomainPools {
    let mut settings = DbConfig::new(db.url());
    settings.max_connections = Some(1);
    settings.min_connections = Some(0);
    settings.connection_budget = 8;
    settings.acquire_timeout = ferrohealth_config::units::Duration::from_secs(5);
    ferroehr::db::connect_domains(&settings)
        .await
        .expect("the pools connect")
}

/// One statement on `pool` inside the running request.
async fn touch(pool: &PgPool) {
    sqlx::query("SELECT 1")
        .execute(&mut *acquire(pool).await.expect("a connection"))
        .await
        .expect("the statement runs");
}

/// Two requests take the clinical and the party pool in opposite orders,
/// each keeping its first connection when it asks for the second: with one
/// connection per pool both are answered at once, because a request that
/// would wait for a pool gives back what it keeps of the others (#5879).
/// Kept across the wait, the two would wait for each other until the
/// acquire timed out.
#[tokio::test]
async fn requests_taking_two_pools_in_opposite_orders_never_wait_for_each_other() {
    let db = testkit::db().await.expect("testkit database");
    let pools = one_connection_pools(&db).await;
    let both = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let request = |first: PgPool, second: PgPool, both: std::sync::Arc<tokio::sync::Barrier>| {
        request_scope(async move {
            touch(&first).await;
            both.wait().await;
            touch(&second).await;
        })
    };
    let started = std::time::Instant::now();
    let answered = tokio::time::timeout(
        std::time::Duration::from_secs(4),
        futures::future::join(
            request(
                pools.clinical.clone(),
                pools.party.clone(),
                std::sync::Arc::clone(&both),
            ),
            request(pools.party.clone(), pools.clinical.clone(), both),
        ),
    )
    .await;
    assert!(answered.is_ok(), "the requests waited for each other");
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}

/// A request runs a clinical read and a party read side by side, and the
/// clinical one waits for its pool. The connection the party read gives back
/// meanwhile goes to the party pool: kept by the waiting request, it would be
/// held for the whole wait, and requests each keeping one pool's connection
/// while they wait for the other's stall until the acquire times out (#6027).
#[tokio::test]
async fn a_request_waiting_for_a_pool_keeps_no_connection_a_read_beside_it_gave_back() {
    let db = testkit::db().await.expect("testkit database");
    let pools = one_connection_pools(&db).await;
    let clinical_taken = acquire(&pools.clinical)
        .await
        .expect("the clinical connection");
    let (party_read, read) = tokio::sync::oneshot::channel();
    let (clinical, party) = (pools.clinical.clone(), pools.party.clone());
    let request = tokio::spawn(request_scope(async move {
        futures::future::join(touch(&clinical), async {
            touch(&party).await;
            party_read
                .send(())
                .expect("the test waits for the party read");
        })
        .await;
    }));
    read.await.expect("the party read ran");
    let party_free =
        tokio::time::timeout(std::time::Duration::from_secs(2), acquire(&pools.party)).await;
    assert!(
        party_free.is_ok_and(|conn| conn.is_ok()),
        "the request kept its party connection while it waited for the clinical pool"
    );
    drop(clinical_taken);
    tokio::time::timeout(std::time::Duration::from_secs(4), request)
        .await
        .expect("the request is answered once the clinical pool is free")
        .expect("the request ran");
}

/// A request keeps its connection across a wait that is ready at once, and
/// gives it back before a wait for another task's work: a copy another
/// request is loading, or a lock another holds, whose holder may need a
/// connection of the same pool (#6027).
#[tokio::test]
async fn a_request_gives_its_connection_back_before_it_waits_for_another_task() {
    let db = testkit::db().await.expect("testkit database");
    let pools = one_connection_pools(&db).await;
    let pool = pools.clinical.clone();
    let (answer, answered) = tokio::sync::oneshot::channel::<()>();
    let (waiting, waits) = tokio::sync::oneshot::channel();
    let request = tokio::spawn(request_scope(async move {
        touch(&pool).await;
        awaiting(std::future::ready(())).await;
        assert_eq!(
            in_use(&pool),
            1,
            "a wait that is ready keeps the connection"
        );
        waiting.send(()).expect("the test waits for the request");
        awaiting(answered).await.expect("the other task answers");
    }));
    waits.await.expect("the request reached its wait");
    assert_eq!(
        settled_in_use(&pools.clinical, 0).await,
        0,
        "the request kept its connection while it waited for another task"
    );
    answer.send(()).expect("the request waits for the answer");
    request.await.expect("the request ran");
}

/// A request that asks a pool for a second connection while it holds one of
/// it is refused in a debug build: once every connection is held so, each
/// such request waits for one only another of them could give back (#5879).
#[tokio::test]
#[should_panic(expected = "a second connection while it held one")]
async fn a_second_connection_of_a_held_pool_is_refused_in_a_debug_build() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    request_scope(async {
        let _held = acquire(&pool).await.expect("a connection");
        let _second = acquire(&pool).await;
    })
    .await;
}

/// A task spawned to work for a request that holds a connection carries that
/// hold: its ask for a connection of the same pool is refused in a debug
/// build, since the request may wait for the task while every connection is
/// held so (#5947).
#[tokio::test]
async fn a_task_working_for_a_request_cannot_ask_for_the_pool_the_request_holds() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let spawned = request_scope(async {
        let _held = acquire(&pool).await.expect("a connection");
        let pool = pool.clone();
        tokio::spawn(ferroehr::db::acquire::inheriting(async move {
            let _second = acquire(&pool).await;
        }))
        .await
    })
    .await;
    let refused = spawned.expect_err("the spawned acquire is refused");
    assert!(refused.is_panic(), "{refused:?}");
}

/// A request that turns from the party pool to the clinical one gives its
/// party connection back to the pool as its clinical work begins, and one
/// that turns back checks a party connection out again and gives back the
/// clinical one (#5774).
#[tokio::test]
async fn a_request_turning_to_the_other_domain_gives_back_the_first_domains_connection() {
    let db = testkit::db().await.expect("testkit database");
    let pools = one_connection_pools(&db).await;
    let (clinical, party) = (pools.get(Domain::Clinical), pools.get(Domain::Party));

    request_scope(async {
        touch(party).await;
        assert_eq!(in_use(party), 1, "the scope keeps the party connection");
        touch(party).await;
        assert_eq!(in_use(party), 1, "a second party statement reuses it");

        touch(clinical).await;
        assert_eq!(
            settled_in_use(party, 0).await,
            0,
            "the party connection is back in its pool once the clinical work begins"
        );
        assert_eq!(
            in_use(clinical),
            1,
            "the scope keeps the clinical connection"
        );

        touch(party).await;
        assert_eq!(
            in_use(party),
            1,
            "turning back checks a party connection out again"
        );
        assert_eq!(
            settled_in_use(clinical, 0).await,
            0,
            "and gives the clinical one back"
        );
    })
    .await;
    pools.close().await;
}

/// The audit connection a write takes before its domain connection stays
/// kept when the request turns to the domain pool: the audit pool is taken
/// first and never asked again while a domain connection is held (#5964).
#[tokio::test]
async fn a_request_keeps_its_audit_connection_when_it_turns_to_a_domain() {
    let db = testkit::db().await.expect("testkit database");
    let pools = one_connection_pools(&db).await;
    let (audit, clinical) = (pools.get(Domain::Audit), pools.get(Domain::Clinical));

    request_scope(async {
        touch(audit).await;
        touch(clinical).await;
        assert_eq!(in_use(audit), 1, "the audit connection stays kept");
        assert_eq!(in_use(clinical), 1);
        touch(audit).await;
        assert_eq!(in_use(audit), 1, "the kept audit connection serves again");
    })
    .await;
    pools.close().await;
}

/// The care projector reads the party store and gives that connection back
/// before it takes a clinical one for its transaction: while the party pool
/// is exhausted, the projector waits for it holding no clinical connection,
/// and it completes once the party pool has a connection again (#6187).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_care_projector_waits_for_the_party_pool_holding_no_clinical_connection() {
    let db = testkit::db().await.expect("testkit database");
    let mut settings = DbConfig::new(db.url());
    settings.max_connections = Some(1);
    settings.min_connections = Some(0);
    settings.connection_budget = 8;
    settings.acquire_timeout = ferrohealth_config::units::Duration::from_secs(20);
    let pools = ferroehr::db::connect_domains(&settings)
        .await
        .expect("the pools connect");
    let (clinical, party) = (
        pools.get(Domain::Clinical).clone(),
        pools.get(Domain::Party).clone(),
    );
    let svc = std::sync::Arc::new(ferroehr::service::FerroEhrService::new(&pools));

    let held = acquire(&party).await.expect("the one party connection");
    let projecting = {
        let svc = std::sync::Arc::clone(&svc);
        tokio::spawn(async move { svc.project_care().await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(
        !projecting.is_finished(),
        "the projector waits for the party pool"
    );
    assert_eq!(
        settled_in_use(&clinical, 0).await,
        0,
        "the waiting projector holds no clinical connection"
    );

    drop(held);
    projecting
        .await
        .expect("the projector task")
        .expect("the projector completes once the party pool has a connection");
    pools.close().await;
}

/// A write holds its audit connection and its domain's for its transaction,
/// so a request that asks the other domain's pool for a connection inside
/// it is refused in a debug build: writes each holding one domain and
/// waiting for the other's pool stall until the acquire timeout (#6186).
#[tokio::test]
#[should_panic(expected = "inside a write of the other")]
async fn a_write_asking_the_other_domains_pool_is_refused_in_a_debug_build() {
    let db = testkit::db().await.expect("testkit database");
    let pools = one_connection_pools(&db).await;
    let (audit, clinical, party) = (
        pools.get(Domain::Audit),
        pools.get(Domain::Clinical),
        pools.get(Domain::Party),
    );
    request_scope(async {
        let _audit = acquire(audit).await.expect("the audit connection");
        let _clinical = acquire(clinical).await.expect("the clinical connection");
        let _party = acquire(party).await;
    })
    .await;
}

/// A request that reads the other domain before it takes a write's audit
/// and domain connections, or that holds no audit connection, is not
/// refused (#6186).
#[tokio::test]
async fn a_read_of_the_other_domain_before_a_write_is_not_refused() {
    let db = testkit::db().await.expect("testkit database");
    let pools = one_connection_pools(&db).await;
    let (audit, clinical, party) = (
        pools.get(Domain::Audit),
        pools.get(Domain::Clinical),
        pools.get(Domain::Party),
    );
    request_scope(async {
        touch(party).await;
        let _audit = acquire(audit).await.expect("the audit connection");
        let _clinical = acquire(clinical).await.expect("the clinical connection");
    })
    .await;
    request_scope(async {
        let _clinical = acquire(clinical).await.expect("the clinical connection");
        touch(party).await;
    })
    .await;
    pools.close().await;
}
