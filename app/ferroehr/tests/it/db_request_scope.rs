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
use ferroehr::db::acquire::{acquire, release_held, request_scope};
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
