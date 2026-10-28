// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A statement whose request is abandoned frees its pool connection at once
//! and stops in PostgreSQL within the client-connection check (#4285).
//!
//! Dropping a request future drops the driver's future, which sends the
//! server nothing; the lease closes the connection instead of returning it,
//! and the session's `client_connection_check_interval` makes the backend
//! notice the closed socket and stop the statement.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use std::time::{Duration, Instant};

use sqlx::PgPool;

use ferroehr::db::DbConfig;
use ferroehr::db::acquire::lease;

/// The pool under test: two connections, none kept open while idle.
async fn small_pool(url: &str) -> PgPool {
    let mut config = DbConfig::new(url);
    config.max_connections = 2;
    config.min_connections = 0;
    ferroehr::db::connect(&config)
        .await
        .expect("the pool connects")
}

/// Connections checked out of `pool` right now.
fn in_use(pool: &PgPool) -> u32 {
    let idle = u32::try_from(pool.num_idle()).expect("an idle count fits u32");
    pool.size().saturating_sub(idle)
}

/// Whether backend `pid` is still running a statement.
async fn active(observer: &PgPool, pid: i32) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid = $1 AND state = 'active')",
    )
    .bind(pid)
    .fetch_one(observer)
    .await
    .expect("pg_stat_activity reads")
}

/// Polls `condition` every 20 ms until it holds or `within` passes.
async fn eventually<F, Fut>(within: Duration, mut condition: F) -> Option<Duration>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let started = Instant::now();
    while started.elapsed() < within {
        if condition().await {
            return Some(started.elapsed());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    None
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_abandoned_statement_frees_its_connection_and_stops() {
    let db = testkit::db().await.expect("testkit database");
    let observer = db.pool();
    let pool = small_pool(db.url()).await;

    let mut leased = lease(&pool).await.expect("a connection");
    let pid: i32 = leased
        .run(|conn| sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(conn))
        .await
        .expect("the backend pid");
    let statement = tokio::spawn(async move {
        leased
            .run(|conn| sqlx::query("SELECT pg_sleep(30)").execute(conn))
            .await
    });
    eventually(Duration::from_secs(5), || active(&observer, pid))
        .await
        .expect("the statement starts");
    assert_eq!(
        in_use(&pool),
        1,
        "the running statement holds its connection"
    );

    // The request goes away: its future, and the lease inside it, drop.
    statement.abort();
    assert!(
        statement.await.is_err_and(|e| e.is_cancelled()),
        "the task was cancelled"
    );

    let freed = eventually(Duration::from_secs(1), || async { in_use(&pool) == 0 })
        .await
        .expect("the connection leaves the pool within a second");
    let stopped = eventually(Duration::from_secs(3), || async {
        !active(&observer, pid).await
    })
    .await
    .expect("the backend stops within the check interval");
    eprintln!("pool slot freed after {freed:?}, backend stopped after {stopped:?}");

    // The pool serves again at once, on a fresh connection.
    let one: i32 = sqlx::query_scalar("SELECT 1")
        .fetch_one(&pool)
        .await
        .expect("the pool serves");
    assert_eq!(one, 1);
}

#[tokio::test]
async fn a_completed_statement_returns_its_connection_to_the_pool() {
    let db = testkit::db().await.expect("testkit database");
    let pool = small_pool(db.url()).await;
    let mut leased = lease(&pool).await.expect("a connection");
    let pid: i32 = leased
        .run(|conn| sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(conn))
        .await
        .expect("the backend pid");
    drop(leased);
    eventually(Duration::from_secs(1), || async { pool.num_idle() == 1 })
        .await
        .expect("the connection returns to the pool");
    let again: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&pool)
        .await
        .expect("the pool serves");
    assert_eq!(again, pid, "the same connection serves the next statement");
}

#[tokio::test]
async fn every_session_checks_its_client_connection() {
    let db = testkit::db().await.expect("testkit database");
    let interval: String = sqlx::query_scalar("SHOW client_connection_check_interval")
        .fetch_one(&db.pool())
        .await
        .expect("the setting reads");
    assert_eq!(interval, "1s");
}
