// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The connections an instance opens: each domain's pool and the listener
//! sessions, inside [`DbConfig::connection_budget`] (#5845), and the
//! connections the server opens before it answers ready (#5774).
//!
//! A request holds a connection of a pool from its first statement until it
//! turns to work that runs none, to the other storage domain's pool or to a
//! wait on another server ([`crate::db::acquire::release_held`]); the pool
//! needs about the arrival rate times that hold, and room for a burst.
//! No openEHR spec governs connection handling: our own design.

use crate::db::acquire::{Checkout, acquire};
use crate::db::domain::{Domain, DomainPools};
use sqlx::postgres::PgPoolOptions;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use tracing::Instrument as _;

use crate::db::{DbConfig, DbError};

/// The pool connections per core an unset [`DbConfig::max_connections`]
/// gives.
///
/// A request holds a connection about as long as it runs on a core, so this is
/// about what every core busy at once keeps in use; the burst above it waits in
/// the pool.
pub const POOL_PER_CORE: u32 = 4;

/// The smallest pool an unset [`DbConfig::max_connections`] gives, when the
/// budget holds it.
pub const POOL_FLOOR: u32 = 10;

/// The smallest pool the budget may leave a domain: a request and a
/// background task each take one connection of a pool at a time.
pub const POOL_MIN: u32 = 2;

/// The connections of the audit runtime's own pool, over the audit store.
///
/// Its chainer, its forwarders, its upkeep and the chain verification each
/// take one at a time, and a task beyond them waits for one
/// ([`crate::system_log::sender`]).
pub const AUDIT_RUNTIME_POOL: u32 = 4;

/// The domains whose writes notify a listened channel, so each may need a
/// listener session of its own; no channel is notified in the audit domain.
pub const LISTENING: [Domain; 2] = [Domain::Clinical, Domain::Party];

/// The named statements one connection keeps prepared.
///
/// The crate's own statement texts, which sqlx's default of 100 holds, plus
/// one AQL statement shape for each plan the default `[record.query]
/// plan_cache_capacity` keeps (`aql::sql::shape`). Past it sqlx closes the
/// least recently used. No openEHR spec governs this: our own design (#5884).
pub const STATEMENT_CACHE: usize = 100 + 256;

/// The cores this process may run on.
fn cores() -> u32 {
    // NOTE: our own design (#5774); a host that cannot tell its cores is sized as one core.
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    u32::try_from(cores).unwrap_or(u32::MAX)
}

impl DbConfig {
    /// The `LISTEN` sessions this instance holds: one per distinct
    /// [`Self::listen_dsn`] of the [`LISTENING`] domains, each carrying every
    /// channel of its database
    /// ([`crate::policy_register::listen::Listeners`]).
    #[must_use]
    pub fn listen_sessions(&self) -> u32 {
        let mut dsns: Vec<&str> = LISTENING.iter().map(|d| self.listen_dsn(*d)).collect();
        dsns.sort_unstable();
        dsns.dedup();
        u32::try_from(dsns.len()).unwrap_or(u32::MAX)
    }

    /// The pool connections each domain may hold within
    /// [`Self::connection_budget`]: the budget less the listener sessions and
    /// the audit runtime's [`AUDIT_RUNTIME_POOL`], shared evenly by the three
    /// pools.
    #[must_use]
    pub fn pool_share(&self) -> u32 {
        let pools = u32::try_from(Domain::ALL.len()).unwrap_or(u32::MAX);
        self.connection_budget
            .saturating_sub(self.listen_sessions())
            .saturating_sub(AUDIT_RUNTIME_POOL)
            .checked_div(pools)
            .unwrap_or(0)
    }

    /// The size of each domain's pool: [`Self::max_connections`], or else
    /// [`POOL_PER_CORE`] per core the process may run on, at least
    /// [`POOL_FLOOR`], within [`Self::pool_share`].
    #[must_use]
    pub fn pool_size(&self) -> u32 {
        self.max_connections.unwrap_or_else(|| {
            cores()
                .saturating_mul(POOL_PER_CORE)
                .max(POOL_FLOOR)
                .min(self.pool_share())
        })
    }

    /// The most connections this instance opens while it serves: three pools
    /// of [`Self::pool_size`], the audit runtime's [`AUDIT_RUNTIME_POOL`] and
    /// [`Self::listen_sessions`].
    #[must_use]
    pub fn connections(&self) -> u32 {
        let pools = u32::try_from(Domain::ALL.len()).unwrap_or(u32::MAX);
        self.pool_size()
            .saturating_mul(pools)
            .saturating_add(AUDIT_RUNTIME_POOL)
            .saturating_add(self.listen_sessions())
    }

    /// What is wrong with the connection budget: a budget too small to leave
    /// every pool [`POOL_MIN`] connections, or a [`Self::max_connections`]
    /// whose pools and listeners exceed it.
    #[must_use]
    pub fn budget_errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        let listeners = self.listen_sessions();
        if self.pool_share() < POOL_MIN {
            errors.push(format!(
                "database.connection_budget = {} leaves the three pools fewer than {POOL_MIN} \
                 connections each beside the audit runtime's {AUDIT_RUNTIME_POOL} and the \
                 {listeners} listener connection(s); raise it to at least {}",
                self.connection_budget,
                POOL_MIN
                    .saturating_mul(3)
                    .saturating_add(AUDIT_RUNTIME_POOL)
                    .saturating_add(listeners)
            ));
        }
        if let Some(size) = self.max_connections
            && self.connections() > self.connection_budget
        {
            errors.push(format!(
                "database.max_connections = {size} needs 3 × {size} pool connections, the \
                 audit runtime's {AUDIT_RUNTIME_POOL} and {listeners} listener connection(s), \
                 {} in all, over \
                 database.connection_budget = {}; raise the budget within PostgreSQL's \
                 max_connections over your instances, or lower the pool",
                self.connections(),
                self.connection_budget
            ));
        }
        errors
    }

    /// The connections each pool keeps open while idle:
    /// [`Self::min_connections`], none when unset, never more than the pool
    /// size.
    #[must_use]
    pub fn pool_kept(&self) -> u32 {
        self.min_connections.unwrap_or(0).min(self.pool_size())
    }

    /// These settings as the server runs them, for one that admits at most
    /// `max_concurrent_requests` at once: an unset pool size is bounded by
    /// that, since a request holds at most one connection of a pool, and an
    /// unset [`Self::min_connections`] keeps every connection of the pool, so
    /// a server that answers ready opens no connection under its first burst.
    #[must_use]
    pub fn for_serving(&self, max_concurrent_requests: u32) -> Self {
        let mut serving = self.clone();
        let size = self
            .max_connections
            .unwrap_or_else(|| self.pool_size().min(max_concurrent_requests.max(1)));
        serving.max_connections = Some(size);
        serving.min_connections = Some(self.min_connections.unwrap_or(size));
        serving
    }
}

/// How long a connection sits idle before the pool pings it on its next
/// checkout (#5109).
///
/// A busy pool hands its connections out again well within
/// it, so a loaded server sends no ping; a quiet one pays one round trip per
/// checkout of a connection idle that long. No openEHR spec governs
/// connection handling: our own design.
pub const IDLE_PROBE: Duration = Duration::from_secs(1);

/// The span a new connection's session setup runs in, so its statements read
/// as the pool's and not as the statements of the request that happened to
/// open the connection (#5899).
pub const SESSION_SETUP_SPAN: &str = "db_session_setup";

/// How long a connection serves before the pool may retire it on a release:
/// the `max_lifetime` sqlx 0.9 sets by default
/// (<https://docs.rs/sqlx/0.9.0/sqlx/pool/struct.PoolOptions.html#method.max_lifetime>).
pub const LIFETIME: Duration = Duration::from_mins(30);

/// The window over which the connections of a pool past [`LIFETIME`] retire,
/// one at a time (#6341).
pub const RETIRE_SPREAD: Duration = Duration::from_mins(10);

/// The age past which a connection retires on its next release whatever the
/// pacing, so no connection lives unbounded.
pub const LIFETIME_CAP: Duration = LIFETIME.saturating_add(RETIRE_SPREAD.saturating_mul(2));

/// The retirements of one pool, paced so its connections past [`LIFETIME`]
/// retire one per [`RETIRE_SPREAD`] divided by the pool size.
///
/// The server opens every connection of a pool before it answers ready
/// (#5774), so their ages coincide: retired by sqlx's own `max_lifetime`,
/// all of them would close in the same second and the requests that follow
/// would each wait for a new connection to open (#6341). A replacement is
/// opened at the moment its predecessor retired, so the next generation
/// stays spread. No openEHR spec governs connection handling: our own design.
#[derive(Debug)]
pub(crate) struct Retirement {
    /// The least time between two retirements of the pool.
    spacing: Duration,
    /// When the pool may next retire a connection; `None` before the first.
    next: Mutex<Option<Instant>>,
}

impl Retirement {
    /// The pacing of a pool of `size` connections.
    pub(crate) fn new(size: u32) -> Self {
        Self {
            spacing: RETIRE_SPREAD
                .checked_div(size.max(1))
                .unwrap_or(RETIRE_SPREAD),
            next: Mutex::new(None),
        }
    }

    /// Whether a connection of `age`, released at `now`, retires: once past
    /// [`LIFETIME`] when no other connection of the pool retired within the
    /// spacing, and always once past [`LIFETIME_CAP`].
    pub(crate) fn retires(&self, age: Duration, now: Instant) -> bool {
        if age < LIFETIME {
            return false;
        }
        let mut next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
        if age < LIFETIME_CAP && next.is_some_and(|next| now < next) {
            return false;
        }
        *next = now.checked_add(self.spacing);
        true
    }
}

/// The pool options for one domain: sizing + acquire timeout from settings,
/// the domain's session setup on every physical connection, a liveness ping
/// only for a connection idle for [`IDLE_PROBE`], and each connection retired
/// on a release past [`LIFETIME`] at the pace of [`Retirement`]. The idle reap
/// stays on the sqlx default.
pub(crate) fn options(settings: &DbConfig, domain: Domain) -> PgPoolOptions {
    // Rendered once here rather than per connection. The value is an integer
    // from our own configuration, never client input, and it is bound as a
    // literal because PostgreSQL's `SET` takes no parameter placeholder.
    let statement_timeout = (!settings.statement_timeout.is_zero()).then(|| {
        format!(
            "SET statement_timeout = {}",
            settings.statement_timeout.as_millis()
        )
    });
    let search_path = domain.search_path();
    let retirement = Arc::new(Retirement::new(settings.pool_size()));
    PgPoolOptions::new()
        .max_connections(settings.pool_size())
        .min_connections(settings.pool_kept())
        .acquire_timeout(settings.acquire_timeout.get())
        // No liveness ping per checkout: `test_before_acquire` (default true)
        // pings each connection before handing it out, one round trip per
        // acquisition (https://docs.rs/sqlx/0.9.0/sqlx/pool/struct.PoolOptions.html#method.test_before_acquire).
        // Only a connection that sat idle for `IDLE_PROBE` is pinged, so a
        // session the database cut while nobody used it (a restart, a
        // failover, a terminated backend) is replaced before a request gets it.
        .test_before_acquire(false)
        .before_acquire(|conn, meta| {
            Box::pin(async move {
                if meta.idle_for < IDLE_PROBE {
                    return Ok(true);
                }
                // NOTE: sqlx 0.9 `PoolOptions::before_acquire`, a failed ping is the answer:
                // `false` closes the connection and the pool opens another in its place.
                Ok(sqlx::Connection::ping(conn).await.is_ok())
            })
        })
        // NOTE: sqlx 0.9 `PoolOptions::after_release`, `false` closes the connection; the
        // driver's own `max_lifetime` is off so the pacing alone retires (#6341).
        .max_lifetime(None)
        .after_release(move |_conn, meta| {
            let keep = !retirement.retires(meta.age, Instant::now());
            Box::pin(std::future::ready(Ok(keep)))
        })
        .after_connect(move |conn, _meta| {
            // Cloned per call: `after_connect` takes an `Fn`, so the captured
            // value cannot be moved out of it.
            let statement_timeout = statement_timeout.clone();
            Box::pin(
                async move {
                    crate::db::session::open_session(
                        conn,
                        search_path,
                        statement_timeout.as_deref(),
                    )
                    .await
                }
                .instrument(tracing::debug_span!(SESSION_SETUP_SPAN)),
            )
        })
}

/// Opens the connections every pool of `pools` keeps ([`DbConfig::pool_kept`]
/// of `settings`) and returns them to their pools, so they are open before
/// the server answers ready; the pools keep them while idle.
///
/// An instance restarted under 3,000 requests a second refused 122 to 252
/// of them while its pools grew from the connections kept to their size:
/// each connection opened under load kept its request waiting up to 100 ms
/// (#5774). The three pools open side by side, and the server holds every
/// connection until all are open, so none is handed out twice. No openEHR
/// spec governs this: our own design.
///
/// # Errors
/// [`DbError::Acquire`] for the first connection that cannot be opened.
pub async fn open_kept(settings: &DbConfig, pools: &DomainPools) -> Result<(), DbError> {
    let kept = settings.pool_kept();
    let opened = tokio::try_join!(
        open_all(&pools.clinical, kept),
        open_all(&pools.party, kept),
        open_all(&pools.audit, kept),
    )?;
    drop(opened);
    Ok(())
}

/// `kept` connections of `pool`, each held, so none is handed out twice.
async fn open_all(pool: &sqlx::PgPool, kept: u32) -> Result<Vec<Checkout>, DbError> {
    let mut opened = Vec::new();
    for _ in 0..kept {
        opened.push(acquire(pool).await?);
    }
    Ok(opened)
}

/// One `PostgreSQL` this instance's planned connections exceed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverSlots {
    /// The domains whose pools reach it, comma-separated.
    pub domains: String,
    /// The connections this instance may open to it: the pools of those
    /// domains and the listener sessions of their databases.
    pub planned: u32,
    /// The connections it admits to a role without superuser rights.
    pub admitted: u32,
}

/// The servers whose connection limit this instance alone would exceed.
///
/// Per server the pools of `pools` reach, the pools on it, sized by
/// `settings`, and the listener sessions of its listening domains, against
/// its `max_connections` less `superuser_reserved_connections` and
/// `reserved_connections`
/// (<https://www.postgresql.org/docs/18/runtime-config-connection.html>).
///
/// # Errors
/// [`DbError::Acquire`] or [`DbError::Sqlx`] when a pool cannot read its
/// server's identity and limit.
pub async fn server_slots(
    settings: &DbConfig,
    pools: &DomainPools,
) -> Result<Vec<OverSlots>, DbError> {
    let mut servers: Vec<(String, i32, Vec<Domain>)> = Vec::new();
    for domain in Domain::ALL {
        let (id, admitted): (String, i32) = sqlx::query_as(
            "SELECT system_identifier::text, current_setting('max_connections')::int \
             - current_setting('superuser_reserved_connections')::int \
             - current_setting('reserved_connections')::int FROM pg_control_system()",
        )
        .fetch_one(&mut *acquire(pools.get(domain)).await?)
        .await?;
        match servers.iter_mut().find(|(known, _, _)| *known == id) {
            Some((_, _, domains)) => domains.push(domain),
            None => servers.push((id, admitted, vec![domain])),
        }
    }
    let mut over = Vec::new();
    for (_, admitted, domains) in servers {
        let mut dsns: Vec<&str> = domains
            .iter()
            .filter(|domain| LISTENING.contains(domain))
            .map(|domain| settings.listen_dsn(*domain))
            .collect();
        dsns.sort_unstable();
        dsns.dedup();
        let pools = u32::try_from(domains.len()).unwrap_or(u32::MAX);
        let audit = if domains.contains(&Domain::Audit) {
            AUDIT_RUNTIME_POOL
        } else {
            0
        };
        let planned = settings
            .pool_size()
            .saturating_mul(pools)
            .saturating_add(audit)
            .saturating_add(u32::try_from(dsns.len()).unwrap_or(u32::MAX));
        let admitted = u32::try_from(admitted).unwrap_or(0);
        if planned > admitted {
            over.push(OverSlots {
                domains: domains
                    .iter()
                    .map(|domain| domain.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                planned,
                admitted,
            });
        }
    }
    Ok(over)
}
