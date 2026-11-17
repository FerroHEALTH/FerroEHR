// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The outage gate of [`super::acquire`]: once an acquire has waited
//! [`PROBE_AFTER`], one probe tests whether the database is down, and while
//! it is down every acquire answers at once, one probe at a time testing
//! whether it is back. No openEHR spec governs this: our own design.

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use sqlx::Connection;
use sqlx::PgPool;
use sqlx::Postgres;
use sqlx::pool::PoolConnection;
use sqlx::postgres::PgConnection;

use super::{AcquireError, Cause};

/// How long an acquire waits for the pool before it probes the database.
const PROBE_AFTER: Duration = Duration::from_secs(1);

/// The longest an acquire waits for a probe's connect; a probe still
/// connecting then finds the database slow, which is no evidence of an outage.
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

/// Records a probe's outcome: a reachable database clears the entry, an
/// unreachable one is marked down.
fn record(key: &str, outcome: Result<Reach, Arc<sqlx::Error>>) {
    with_health(|health| match outcome {
        Ok(_) => {
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

/// How a probe found a database it could reach.
#[derive(Debug, Clone, Copy)]
enum Reach {
    /// The database answered: a connection opened, or the server refused the
    /// session itself (too many connections, a bad credential).
    Answered,
    /// The connect was still in progress when [`PROBE_TIMEOUT`] passed.
    Slow,
}

/// Opens and closes one connection outside the pool: `Err` only on evidence
/// that the database cannot be reached ([`is_unreachable`]), `Ok` when it
/// answers or is still connecting after [`PROBE_TIMEOUT`].
async fn probe(pool: &PgPool) -> Result<Reach, Arc<sqlx::Error>> {
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
            Ok(Reach::Answered)
        }
        Ok(Err(error)) if is_unreachable(&error) => Err(Arc::new(error)),
        Ok(Err(_)) => Ok(Reach::Answered),
        Err(_elapsed) => Ok(Reach::Slow),
    }
}

/// Whether a connect failure means the database cannot be reached: no
/// listener, no route, a reset, or a server that is shutting down or not yet
/// accepting connections (`57P01`, `57P02`, `57P03`). A connect that timed
/// out shows a slow path, not an absent server.
fn is_unreachable(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Io(io) => io.kind() != std::io::ErrorKind::TimedOut,
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
    fn record(mut self, outcome: Result<Reach, Arc<sqlx::Error>>) {
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
async fn probe_and_record(pool: &PgPool, claim: ProbeClaim<'_>) -> Result<Reach, Arc<sqlx::Error>> {
    let key = claim.key;
    let outcome = probe(pool).await;
    match &outcome {
        Ok(Reach::Answered) => tracing::debug!(database = key, "database probe: reachable"),
        Ok(Reach::Slow) => tracing::warn!(
            database = key,
            "database probe: still connecting at its bound; the database is slow, acquires keep queueing"
        ),
        Err(error) => tracing::warn!(
            database = key,
            %error,
            "database probe: unreachable; acquires answer at once until a probe connects"
        ),
    }
    claim.record(outcome.clone());
    outcome
}

/// The error for a pool that failed: a connect the database refused or reset
/// ([`is_unreachable`]) records the outage, as a probe would, so the next
/// acquire answers at once instead of connecting again.
fn pool_failed(key: &str, error: sqlx::Error) -> AcquireError {
    if !is_unreachable(&error) {
        return AcquireError(Cause::Pool(error));
    }
    let error = Arc::new(error);
    tracing::warn!(
        database = key,
        %error,
        "database connect: unreachable; acquires answer at once until a probe connects"
    );
    record(key, Err(Arc::clone(&error)));
    AcquireError(Cause::Outage(error))
}

/// [`super::acquire`] without the metric: the outage gate, then the pool, with one
/// probe once the pool has kept the caller waiting [`PROBE_AFTER`].
pub(super) async fn acquire_unmetered(
    pool: &PgPool,
) -> Result<PoolConnection<Postgres>, AcquireError> {
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
    #[expect(
        clippy::disallowed_methods,
        reason = "the one place the crate takes a pooled connection: the ban routes every other caller here"
    )]
    let mut waiting = Box::pin(pool.acquire());
    if let Ok(result) = tokio::time::timeout(PROBE_AFTER, &mut waiting).await {
        return result.map_err(|error| pool_failed(&key, error));
    }
    if claim_probe(&key) {
        let claim = ProbeClaim {
            key: &key,
            recorded: false,
        };
        // The pool answering first drops the probe, and its claim with it.
        let probed = tokio::select! {
            result = &mut waiting => {
                return result.map_err(|error| pool_failed(&key, error));
            }
            probed = probe_and_record(pool, claim) => probed,
        };
        probed.map_err(|error| AcquireError(Cause::Outage(error)))?;
    }
    waiting.await.map_err(|error| pool_failed(&key, error))
}

#[cfg(test)]
mod tests {
    use std::io::{Error, ErrorKind};

    use super::is_unreachable;

    #[test]
    fn a_refused_or_reset_connect_is_an_outage_and_a_timed_out_one_is_not() {
        let io = |kind| sqlx::Error::Io(Error::from(kind));
        assert!(is_unreachable(&io(ErrorKind::ConnectionRefused)));
        assert!(is_unreachable(&io(ErrorKind::ConnectionReset)));
        assert!(!is_unreachable(&io(ErrorKind::TimedOut)));
    }
}
