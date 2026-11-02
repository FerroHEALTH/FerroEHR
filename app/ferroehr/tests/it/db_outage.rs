// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! While the database is down, an acquire answers at once instead of
//! queueing to the acquire timeout; a busy pool still queues; acquires
//! succeed again as soon as the database accepts connections (#4323); and a
//! database that accepts connections slowly is never taken for down (#4782).

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use ferroehr::db::acquire::{AcquireError, acquire};

/// The acquire timeout of every pool here, far above any bound asserted.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(30);

/// Whether the acquire was answered from a known outage, as its message says.
fn is_outage(error: &AcquireError) -> bool {
    error.to_string() == "the database is down; no connection was attempted"
}

/// A loopback port that stands for a database which is down until the
/// returned switch is set, and up from then on, with the task holding it.
///
/// While the switch is clear, every connection is accepted and reset at once
/// (`SO_LINGER` 0), which every platform reports to the client as a reset; a
/// bound socket that does not listen is refused on Linux but left unanswered
/// on macOS. Once the switch is set, every connection is forwarded to the
/// database at `target`. The listener holds the port throughout, so no
/// parallel test's server can take it between the outage and the return.
async fn outage_port(target: String) -> (Arc<AtomicBool>, JoinHandle<()>, u16) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback listener");
    let port = listener.local_addr().expect("its address").port();
    let back = Arc::new(AtomicBool::new(false));
    let up = Arc::clone(&back);
    let task = tokio::spawn(async move {
        while let Ok((inbound, _)) = listener.accept().await {
            if up.load(Ordering::SeqCst) {
                tokio::spawn(relay(inbound, target.clone()));
            } else {
                inbound
                    .set_zero_linger()
                    .expect("SO_LINGER 0 on the accepted connection");
                drop(inbound);
            }
        }
    });
    (back, task, port)
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

/// Forwards `inbound` to the database at `target`.
async fn relay(mut inbound: TcpStream, target: String) {
    if let Ok(mut outbound) = TcpStream::connect(&target).await {
        let _copied = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
    }
}

/// Forwards every connection on `listener` to the database at `target`,
/// holding each one `delay` before the handshake reaches the database.
async fn forward_slowly(listener: TcpListener, target: String, delay: Duration) {
    while let Ok((inbound, _)) = listener.accept().await {
        let target = target.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            relay(inbound, target).await;
        });
    }
}

#[tokio::test]
async fn a_down_database_answers_at_once_and_a_returning_one_serves_again() {
    let db = testkit::db().await.expect("testkit database");
    let real = PgConnectOptions::from_str(db.url()).expect("the test DSN parses");
    let target = format!("{}:{}", real.get_host(), real.get_port());
    let (back, proxy, port) = outage_port(target).await;
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

    // The database comes back on the same address, the port held since.
    back.store(true, Ordering::SeqCst);
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

#[tokio::test]
async fn a_slow_connect_past_the_probe_bound_is_served_after_the_delay() {
    // Past the probe's start (1 s) plus its bound (2 s), under the acquire timeout.
    const DELAY: Duration = Duration::from_millis(3_500);
    let db = testkit::db().await.expect("testkit database");
    let real = PgConnectOptions::from_str(db.url()).expect("the test DSN parses");
    let target = format!("{}:{}", real.get_host(), real.get_port());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback listener");
    let port = listener.local_addr().expect("its address").port();
    let proxy = tokio::spawn(forward_slowly(listener, target, DELAY));
    let pool = pool_through(db.url(), port);

    let started = Instant::now();
    let first = acquire(&pool)
        .await
        .expect("a slow database is served, not refused");
    let first_took = started.elapsed();
    assert!(
        first_took >= DELAY,
        "the acquire waited for the slow connect, answered after {first_took:?}"
    );

    // The pool must open a second connection; the database was not marked down.
    let started = Instant::now();
    let second = acquire(&pool)
        .await
        .expect("the slow database is still served");
    let second_took = started.elapsed();
    assert!(
        second_took >= DELAY,
        "the second acquire also waited for its connect, answered after {second_took:?}"
    );
    eprintln!("served after {first_took:?}, and again after {second_took:?}");
    drop((first, second));
    proxy.abort();
}

#[tokio::test]
async fn a_reset_connect_is_recorded_as_an_outage_and_the_next_acquire_answers_at_once() {
    let db = testkit::db().await.expect("testkit database");
    let real = PgConnectOptions::from_str(db.url()).expect("the test DSN parses");
    let target = format!("{}:{}", real.get_host(), real.get_port());
    let (_back, proxy, port) = outage_port(target).await;
    let pool = pool_through(db.url(), port);

    let started = Instant::now();
    let first = acquire(&pool).await.expect_err("every connect is reset");
    let first_took = started.elapsed();
    assert!(is_outage(&first), "the reset recorded an outage: {first:?}");
    assert!(
        first_took < Duration::from_secs(1),
        "the pool's own reset connect answered before any probe, took {first_took:?}"
    );

    let started = Instant::now();
    let second = acquire(&pool).await.expect_err("still reset");
    let second_took = started.elapsed();
    assert!(is_outage(&second), "{second:?}");
    assert!(
        second_took < Duration::from_millis(200),
        "a known outage answers at once, took {second_took:?}"
    );
    proxy.abort();
}
