// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! While the database is down, an acquire answers at once instead of
//! queueing to the acquire timeout; a busy pool still queues; and acquires
//! succeed again as soon as the database accepts connections (#4323).

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use std::str::FromStr;
use std::time::{Duration, Instant};

use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tokio::net::{TcpListener, TcpStream};

use ferroehr::db::acquire::{AcquireError, acquire};

/// The acquire timeout of every pool here, far above any bound asserted.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(30);

/// Whether the acquire was answered from a known outage, as its message says.
fn is_outage(error: &AcquireError) -> bool {
    error.to_string() == "the database is down; no connection was attempted"
}

/// A loopback port nothing listens on.
async fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
    listener.local_addr().expect("its address").port()
}

/// A lazy pool for the test database, reached through `port` on loopback.
fn pool_through(url: &str, port: u16) -> PgPool {
    let options = PgConnectOptions::from_str(url)
        .expect("the test DSN parses")
        .host("127.0.0.1")
        .port(port);
    PgPoolOptions::new()
        .max_connections(2)
        .min_connections(0)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .connect_lazy_with(options)
}

/// Forwards every connection on `listener` to the database at `target`.
async fn forward(listener: TcpListener, target: String) {
    while let Ok((mut inbound, _)) = listener.accept().await {
        let target = target.clone();
        tokio::spawn(async move {
            if let Ok(mut outbound) = TcpStream::connect(&target).await {
                let _copied = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
            }
        });
    }
}

#[tokio::test]
async fn a_down_database_answers_at_once_and_a_returning_one_serves_again() {
    let db = testkit::db().await.expect("testkit database");
    let real = PgConnectOptions::from_str(db.url()).expect("the test DSN parses");
    let target = format!("{}:{}", real.get_host(), real.get_port());
    let port = closed_port().await;
    let pool = pool_through(db.url(), port);

    let started = Instant::now();
    let first = acquire(&pool).await.expect_err("nothing listens");
    let first_took = started.elapsed();
    assert!(
        is_outage(&first),
        "the probe found the database down: {first:?}"
    );
    assert!(
        first_took < Duration::from_secs(4),
        "the first acquire answered after {first_took:?}, not the acquire timeout"
    );

    let started = Instant::now();
    let second = acquire(&pool).await.expect_err("still down");
    let second_took = started.elapsed();
    assert!(is_outage(&second), "{second:?}");
    assert!(
        second_took < Duration::from_millis(200),
        "a known outage answers at once, took {second_took:?}"
    );

    // The database comes back on the same address.
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("the port is free again");
    let proxy = tokio::spawn(forward(listener, target));
    let started = Instant::now();
    let conn = loop {
        match acquire(&pool).await {
            Ok(conn) => break conn,
            Err(error) => {
                assert!(is_outage(&error), "{error:?}");
                assert!(
                    started.elapsed() < Duration::from_secs(3),
                    "the database is back but acquires still fail"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    };
    eprintln!(
        "first refusal after {first_took:?}, known outage after {second_took:?}, \
         served again {:?} after the database returned",
        started.elapsed()
    );
    drop(conn);
    proxy.abort();
}

#[tokio::test]
async fn a_busy_pool_with_the_database_up_still_queues() {
    let db = testkit::db().await.expect("testkit database");
    let options = PgConnectOptions::from_str(db.url()).expect("the test DSN parses");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .min_connections(0)
        .acquire_timeout(Duration::from_secs(3))
        .connect_lazy_with(options);
    let held = acquire(&pool).await.expect("the one connection");

    let started = Instant::now();
    let waited = acquire(&pool).await.expect_err("the pool stays full");
    let took = started.elapsed();
    assert!(
        waited.is_pool_timeout(),
        "a full pool times out: {waited:?}"
    );
    assert!(!is_outage(&waited), "the probe found the database up");
    assert!(
        took >= Duration::from_millis(2_900),
        "a busy pool queues to its acquire timeout, answered after {took:?}"
    );
    drop(held);
    let again = acquire(&pool).await.expect("the freed connection serves");
    drop(again);
}
