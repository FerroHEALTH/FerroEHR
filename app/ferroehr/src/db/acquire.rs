// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The one way the crate obtains a pooled connection, with the connect phase of
//! a failure kept in the error type.
//!
//! A [`sqlx::Error`] does not record whether it arose while a connection was
//! being obtained or on an open session, and a statement executed on a
//! `&PgPool` acquires implicitly, so its failure carries no phase either.
//! [`Pool::acquire`] fails only before any statement of the caller runs: the
//! pool timed out or was closed, or opening a new connection failed (sqlx 0.9,
//! <https://docs.rs/sqlx/0.9.0/sqlx/pool/struct.Pool.html#method.acquire>).
//! [`acquire`] wraps every such failure in an [`AcquireError`], which the
//! storage and service bridges answer with `503` plus `Retry-After`, while a
//! failure on the acquired session stays a plain [`sqlx::Error`] and is
//! classified by identity. Executing on the connection [`acquire`] returns,
//! instead of on the pool, is what makes the phase known; a transaction is
//! begun on that connection (`conn.begin()`), so a failing `BEGIN` is a
//! session failure.
//!
//! While the database is down, the pool keeps retrying a refused connect
//! until its acquire deadline (sqlx 0.9 `PoolInner::connect`), so a request
//! would wait the whole `acquire_timeout_secs` for an answer the server could
//! give at once. [`acquire`] therefore probes the database itself once an
//! acquire has waited one second: a probe that cannot connect marks the
//! database down, and while it is down every acquire answers at once, one
//! probe at a time testing whether it is back. A probe that connects leaves a
//! busy pool queueing as before.
//!
//! No openEHR spec governs this — our own design.
//!
//! [`Pool::acquire`]: sqlx::Pool::acquire

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use sqlx::Connection;
use sqlx::PgPool;
use sqlx::Postgres;
use sqlx::pool::PoolConnection;
use sqlx::postgres::PgConnection;

/// A failure to obtain a connection from a pool: no statement of the caller
/// reached the database, so the request can be repeated once it answers.
///
/// The driver error is the source and stays out of this error's own message.
#[derive(Debug)]
pub struct AcquireError(Cause);

/// Where an [`AcquireError`] came from.
#[derive(Debug)]
enum Cause {
    /// The pool itself failed.
    Pool(sqlx::Error),
    /// The database is known to be down: the driver error is the latest
    /// probe's, shared by every acquire it answered.
    Outage(Arc<sqlx::Error>),
}

impl std::fmt::Display for AcquireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.0 {
            Cause::Pool(_) => "acquiring a database connection failed",
            Cause::Outage(_) => "the database is down; no connection was attempted",
        })
    }
}

impl std::error::Error for AcquireError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.driver())
    }
}

impl AcquireError {
    /// Returns the driver error the pool, or the probe that found the
    /// database down, reported.
    #[must_use]
    pub fn driver(&self) -> &sqlx::Error {
        match &self.0 {
            Cause::Pool(error) => error,
            Cause::Outage(error) => error,
        }
    }

    /// Returns whether the pool gave up waiting for a free connection, as
    /// opposed to failing to open one.
    #[must_use]
    pub fn is_pool_timeout(&self) -> bool {
        matches!(self.0, Cause::Pool(sqlx::Error::PoolTimedOut))
    }

    /// Returns whether the acquire was answered because the database is
    /// known to be down, without waiting for the pool.
    #[must_use]
    pub fn is_outage(&self) -> bool {
        matches!(self.0, Cause::Outage(_))
    }
}

/// How long an acquire waits for the pool before it probes the database.
const PROBE_AFTER: Duration = Duration::from_secs(1);

/// The longest a probe's connect may take before the database counts as down.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// While the database is down, the shortest gap between two probes.
const PROBE_INTERVAL: Duration = Duration::from_millis(500);

/// What is known about one database, by [`database_key`].
enum Health {
    /// A probe is connecting and nothing is known yet.
    Probing,
    /// The last probe could not connect.
    Down {
        /// The probe's driver error.
        error: Arc<sqlx::Error>,
        /// When the last probe ended.
        probed_at: Instant,
        /// Whether a probe is connecting now.
        probing: bool,
    },
}

/// The databases a probe is testing or found down; a database absent here
/// is presumed up.
static HEALTH: LazyLock<Mutex<BTreeMap<String, Health>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

/// The database a pool connects to, as `host:port/database`.
fn database_key(pool: &PgPool) -> String {
    let options = pool.connect_options();
    format!(
        "{}:{}/{}",
        options.get_host(),
        options.get_port(),
        options.get_database().unwrap_or("")
    )
}

/// What an acquire does before it asks the pool.
enum Gate {
    /// The database is presumed up: ask the pool.
    Pass,
    /// The database is down and this acquire probes it.
    Probe,
    /// The database is down and another probe is due later or running.
    Refuse(Arc<sqlx::Error>),
}

/// Runs `body` on the health map; a poisoned lock is taken as it stands,
/// since every writer leaves the map consistent between statements.
fn with_health<T>(body: impl FnOnce(&mut BTreeMap<String, Health>) -> T) -> T {
    let mut guard = HEALTH
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    body(&mut guard)
}

/// Decides what an acquire for `key` does first.
fn gate(key: &str) -> Gate {
    with_health(|health| match health.get_mut(key) {
        None | Some(Health::Probing) => Gate::Pass,
        Some(Health::Down {
            error,
            probed_at,
            probing,
        }) => {
            if *probing || probed_at.elapsed() < PROBE_INTERVAL {
                Gate::Refuse(Arc::clone(error))
            } else {
                *probing = true;
                Gate::Probe
            }
        }
    })
}

/// Claims the probe for a database presumed up; `false` when another acquire
/// already probes it or found it down.
fn claim_probe(key: &str) -> bool {
    with_health(|health| {
        if health.contains_key(key) {
            false
        } else {
            health.insert(key.to_owned(), Health::Probing);
            true
        }
    })
}

/// Records a probe's outcome: a connect clears the entry, a failure marks
/// the database down.
fn record(key: &str, outcome: Result<(), Arc<sqlx::Error>>) {
    with_health(|health| match outcome {
        Ok(()) => {
            health.remove(key);
        }
        Err(error) => {
            health.insert(
                key.to_owned(),
                Health::Down {
                    error,
                    probed_at: Instant::now(),
                    probing: false,
                },
            );
        }
    });
}

/// Opens and closes one connection outside the pool: `Err` when the database
/// cannot be reached, `Ok` when it answers, including a refusal of the
/// session itself (too many connections, a bad credential), which no outage
/// explains.
async fn probe(pool: &PgPool) -> Result<(), Arc<sqlx::Error>> {
    let options = pool.connect_options();
    match tokio::time::timeout(
        PROBE_TIMEOUT,
        Box::pin(PgConnection::connect_with(&*options)),
    )
    .await
    {
        Ok(Ok(conn)) => {
            // NOTE: the probe's connection is not the pool's; a failed goodbye changes nothing.
            if let Err(error) = conn.close().await {
                tracing::debug!(%error, "database probe: closing the probe connection failed");
            }
            Ok(())
        }
        Ok(Err(error)) if is_unreachable(&error) => Err(Arc::new(error)),
        Ok(Err(_)) => Ok(()),
        Err(_elapsed) => Err(Arc::new(sqlx::Error::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "the database probe did not connect in time",
        )))),
    }
}

/// Whether a connect failure means the database cannot be reached: no
/// listener, no route, a reset, or a server that is shutting down or not yet
/// accepting connections (`57P01`, `57P02`, `57P03`).
fn is_unreachable(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Io(_) => true,
        sqlx::Error::Database(db) => db
            .code()
            .is_some_and(|code| matches!(&*code, "57P01" | "57P02" | "57P03")),
        _ => false,
    }
}

/// The probe an acquire claimed for one database: dropped before its outcome
/// is recorded (the acquire was abandoned mid-probe), it hands the probe back
/// so the next acquire can claim it.
struct ProbeClaim<'k> {
    key: &'k str,
    recorded: bool,
}

impl ProbeClaim<'_> {
    /// Records the probe's outcome.
    fn record(mut self, outcome: Result<(), Arc<sqlx::Error>>) {
        record(self.key, outcome);
        self.recorded = true;
    }
}

impl Drop for ProbeClaim<'_> {
    fn drop(&mut self) {
        if self.recorded {
            return;
        }
        with_health(|health| match health.get_mut(self.key) {
            Some(Health::Probing) => {
                health.remove(self.key);
            }
            Some(Health::Down { probing, .. }) => *probing = false,
            None => {}
        });
    }
}

/// Probes the database for the claimed key and records the outcome.
async fn probe_and_record(pool: &PgPool, claim: ProbeClaim<'_>) -> Result<(), Arc<sqlx::Error>> {
    let key = claim.key;
    let outcome = probe(pool).await;
    match &outcome {
        Ok(()) => tracing::debug!(database = key, "database probe: reachable"),
        Err(error) => tracing::warn!(
            database = key,
            %error,
            "database probe: unreachable; acquires answer at once until a probe connects"
        ),
    }
    claim.record(outcome.clone());
    outcome
}

/// Obtains a connection from `pool`, marking any failure as acquire-time.
///
/// The wait, successful or not, is recorded on
/// [`DB_POOL_ACQUIRE_DURATION`](crate::telemetry::metrics::DB_POOL_ACQUIRE_DURATION).
///
/// The connection returns to the pool when the returned guard drops; a caller
/// that executes one statement passes `&mut *acquire(pool).await?` so the guard
/// drops at the end of that statement, as an implicit acquire would.
///
/// # Errors
///
/// [`AcquireError`] when the pool timed out, was closed, or could not open a
/// new connection.
pub async fn acquire(pool: &PgPool) -> Result<PoolConnection<Postgres>, AcquireError> {
    let started = Instant::now();
    // Boxed: the probe and the pool's connect future are large, and every
    // caller's state machine would otherwise carry them inline.
    let result = Box::pin(acquire_unmetered(pool)).await;
    crate::telemetry::metrics::metrics()
        .db_pool_acquire_duration
        .record(started.elapsed().as_secs_f64(), &[]);
    result
}

/// [`acquire`] without the metric: the outage gate, then the pool, with one
/// probe once the pool has kept the caller waiting [`PROBE_AFTER`].
async fn acquire_unmetered(pool: &PgPool) -> Result<PoolConnection<Postgres>, AcquireError> {
    let key = database_key(pool);
    match gate(&key) {
        Gate::Pass => {}
        Gate::Refuse(error) => return Err(AcquireError(Cause::Outage(error))),
        Gate::Probe => {
            let claim = ProbeClaim {
                key: &key,
                recorded: false,
            };
            probe_and_record(pool, claim)
                .await
                .map_err(|error| AcquireError(Cause::Outage(error)))?;
        }
    }
    // Boxed as sqlx's own `&Pool` executor boxes it: the connect future is
    // large, and every caller's state machine would otherwise carry it inline.
    let mut waiting = Box::pin(pool.acquire());
    if let Ok(result) = tokio::time::timeout(PROBE_AFTER, &mut waiting).await {
        return result.map_err(|error| AcquireError(Cause::Pool(error)));
    }
    if claim_probe(&key) {
        let claim = ProbeClaim {
            key: &key,
            recorded: false,
        };
        // The pool answering first drops the probe, and its claim with it.
        let probed = tokio::select! {
            result = &mut waiting => {
                return result.map_err(|error| AcquireError(Cause::Pool(error)));
            }
            probed = probe_and_record(pool, claim) => probed,
        };
        probed.map_err(|error| AcquireError(Cause::Outage(error)))?;
    }
    waiting
        .await
        .map_err(|error| AcquireError(Cause::Pool(error)))
}

/// A pooled connection that is closed, instead of returned to the pool, when
/// it is dropped while a statement started through [`Lease::run`] is still
/// running.
///
/// Dropping a request future mid-statement drops the driver's future, which
/// sends PostgreSQL nothing: returned to the pool, the connection would wait
/// for the abandoned statement to finish before it could serve again (sqlx
/// 0.9 tests a released connection with a round trip). Closed, its pool slot
/// frees at once, and the backend sees the closed socket at its next
/// `client_connection_check_interval` check
/// (<https://www.postgresql.org/docs/18/runtime-config-connection.html#GUC-CLIENT-CONNECTION-CHECK-INTERVAL>),
/// which every session of this crate's pools sets, and stops the statement.
///
/// No openEHR spec governs this — our own design.
#[derive(Debug)]
pub struct Lease {
    conn: PoolConnection<Postgres>,
    running: bool,
}

impl Lease {
    /// Runs `statement` on the leased connection.
    ///
    /// The lease is armed for as long as the returned future runs: drop the
    /// future before it completes, and dropping the lease closes the
    /// connection. A statement that completes, successfully or not, leaves
    /// the connection fit to return to the pool.
    pub async fn run<'c, F>(
        &'c mut self,
        statement: impl FnOnce(&'c mut PgConnection) -> F,
    ) -> F::Output
    where
        F: Future + 'c,
    {
        let Self { conn, running } = self;
        *running = true;
        let output = statement(conn).await;
        *running = false;
        output
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if self.running {
            tracing::debug!("closing a pooled connection whose statement was abandoned");
            self.conn.close_on_drop();
        }
    }
}

/// Obtains a [`Lease`] on a connection from `pool`, for a statement that may
/// run long enough for its caller to give up on it.
///
/// # Errors
///
/// [`AcquireError`] when the pool timed out, was closed, or could not open a
/// new connection.
pub async fn lease(pool: &PgPool) -> Result<Lease, AcquireError> {
    Ok(Lease {
        conn: acquire(pool).await?,
        running: false,
    })
}
