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
//! would wait the whole `acquire_timeout` for an answer the server could
//! give at once. [`acquire`] therefore probes the database itself once an
//! acquire has waited one second: only a probe whose connect is refused or
//! reset, or that the server turns away as shutting down, marks the database
//! down, and while it is down every acquire answers at once, one probe at a
//! time testing whether it is back. A probe that connects, or is still
//! connecting when its bound passes (a slow database, not a down one), leaves
//! the acquire queueing on the pool as a busy pool does. A connect of the
//! pool's own that is refused or reset marks the database down the same way.
//!
//! Inside [`request_scope`], a request keeps the connection it checked out
//! between its statements and returns it to the pool once: at [`release_held`],
//! which the HTTP layer calls when the handler has answered, or when the scope
//! ends. A request that turns from one storage domain to the other gives back
//! what it keeps of the first ([`acquire`]). While the request waits for a pool
//! or for another task's work ([`handoff`]), the scope keeps nothing (#6027).
//! How long the request held each connection is recorded on
//! [`DB_POOL_HOLD_DURATION`](crate::telemetry::metrics::DB_POOL_HOLD_DURATION)
//! (#5774). Outside a scope every guard returns its connection to the pool.
//!
//! No openEHR spec governs this — our own design.
//!
//! [`Pool::acquire`]: sqlx::Pool::acquire

use std::cell::{Cell, RefCell};
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::time::Instant;

use opentelemetry::KeyValue;
use sqlx::PgPool;
use sqlx::Postgres;
use sqlx::pool::PoolConnection;
use sqlx::postgres::PgConnection;

use crate::db::domain::Domain;
use crate::db::pool_name::{PoolName, label, name_of};
use outage::acquire_unmetered;

mod outage;

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
}

/// The attribute of the pool metrics that names the pool
/// ([`crate::db::pool_name::label`], #5972).
const POOL: &str = "pool";

/// A connection a [`request_scope`] keeps between checkouts.
struct Held {
    /// The pool's [`pool_key`].
    pool: usize,
    /// The connection.
    conn: PoolConnection<Postgres>,
    /// When the request took it from the pool.
    since: Instant,
    /// The pool's name ([`crate::db::pool_name::name_of`]).
    name: Option<PoolName>,
}

tokio::task_local! {
    /// The connections a [`request_scope`] keeps between checkouts, at most
    /// one per pool.
    static HELD: RefCell<Vec<Held>>;
    /// The pool of each connection a [`request_scope`] has checked out now,
    /// with the pool's name.
    static OUT: RefCell<Vec<(usize, Option<PoolName>)>>;
    /// The waits of a [`request_scope`] in progress now ([`handoff`]); while
    /// there is one, the scope keeps no connection.
    static WAITS: Cell<u32>;
}

/// Runs `body` as one request: the connection each pool hands it is kept
/// between its checkouts and returns to the pool at [`release_held`], when
/// `body` ends, or when the request waits for another pool ([`acquire`]).
pub async fn request_scope<F: Future>(body: F) -> F::Output {
    let kept = HELD.scope(RefCell::new(Vec::new()), async {
        let output = body.await;
        release_held();
        output
    });
    tracked(WAITS.scope(Cell::new(0), kept)).await
}

/// Runs `body`, a background task, with its checkouts counted.
///
/// As in a [`request_scope`], a second acquire of a pool it has a connection
/// of checked out is refused in a debug build ([`acquire`]); nothing is kept
/// between checkouts (#5929).
pub async fn tracked<F: Future>(body: F) -> F::Output {
    OUT.scope(RefCell::new(Vec::new()), body).await
}

/// `body`, carrying the pools this task holds a connection of into a task
/// spawned to work for it.
///
/// Checked out or kept, each counts: an [`acquire`] in `body` of one of them
/// is a nested acquire, refused in a debug build, since the task that holds
/// the connection may be waiting for `body` (#5947). Outside a
/// [`request_scope`] it carries nothing.
pub fn inheriting<F: Future>(body: F) -> impl Future<Output = F::Output> {
    let mut held: Vec<(usize, Option<PoolName>)> = OUT
        .try_with(|out| out.try_borrow().map(|out| out.clone()).unwrap_or_default())
        .unwrap_or_default();
    // NOTE: our own design (#5947), outside a request scope no connection is kept.
    let _outside_a_scope = HELD.try_with(|kept| {
        if let Ok(kept) = kept.try_borrow() {
            held.extend(kept.iter().map(|kept| (kept.pool, kept.name)));
        }
    });
    OUT.scope(RefCell::new(held), body)
}

/// Whether the current [`request_scope`] has a connection of the pool `key`
/// checked out now.
fn checked_out(key: usize) -> bool {
    OUT.try_with(|out| {
        out.try_borrow()
            .is_ok_and(|out| out.iter().any(|(pool, _)| *pool == key))
    })
    .unwrap_or(false)
}

/// Whether the current [`request_scope`] has a connection of the clinical or
/// the party pool checked out now.
fn holds_a_domain() -> bool {
    OUT.try_with(|out| {
        out.try_borrow().is_ok_and(|out| {
            out.iter()
                .any(|(_, name)| name.is_some_and(|name| name.domain() != Domain::Audit))
        })
    })
    .unwrap_or(false)
}

/// Whether the current [`request_scope`] has an audit connection checked
/// out beside one of a storage domain other than `domain`: it is inside a
/// write of that other domain, which takes the audit connection first and
/// its domain's next and keeps both for its transaction (#5964).
fn writes_another_domain(domain: Domain) -> bool {
    OUT.try_with(|out| {
        out.try_borrow().is_ok_and(|out| {
            let holds = |of: &dyn Fn(Domain) -> bool| {
                out.iter()
                    .any(|(_, name)| name.is_some_and(|name| of(name.domain())))
            };
            holds(&|held| held == Domain::Audit)
                && holds(&|held| held != Domain::Audit && held != domain)
        })
    })
    .unwrap_or(false)
}

/// Notes in the current [`request_scope`] that a connection of the pool `key`
/// named `name` is checked out (`out`) or back (`!out`).
fn note_checkout(key: usize, name: Option<PoolName>, out: bool) {
    // NOTE: our own design, outside a request scope there is nothing to note.
    let _outside_a_scope = OUT.try_with(|checked| {
        if let Ok(mut checked) = checked.try_borrow_mut() {
            if out {
                checked.push((key, name));
            } else if let Some(at) = checked.iter().position(|(pool, _)| *pool == key) {
                checked.swap_remove(at);
            }
        }
    });
}

/// The `tracing` target of the event [`release_held`] records when it gives
/// back at least one connection, which tests read to order the release
/// against the work that follows it.
pub const RELEASED: &str = "ferroehr::db::released";

/// Returns every connection the current [`request_scope`] keeps to its pool,
/// recording how long the request held each; a later [`acquire`] in the
/// scope checks out again. Outside a scope it does nothing.
pub fn release_held() {
    let released = HELD
        .try_with(|held| {
            held.try_borrow_mut()
                .map(|mut held| std::mem::take(&mut *held))
        })
        .ok()
        .and_then(Result::ok);
    let mut given_back = 0_usize;
    for held in released.into_iter().flatten() {
        held_for(held.since, held.name);
        drop(held.conn);
        given_back = given_back.saturating_add(1);
    }
    if given_back > 0 {
        tracing::trace!(target: RELEASED, connections = given_back, "connections given back");
    }
}

/// Awaits `wait`, a wait for work another task does or for a pool, after
/// giving back every connection the current [`request_scope`] keeps
/// ([`release_held`]).
///
/// A request that kept a connection across such a wait would hold it while
/// the work it waits for may need a connection of the same pool: requests
/// each keeping one and waiting so hold the pool until the acquire timeout
/// (#5879, #6008). Every wait of a request for another task's work goes
/// through here.
///
/// A request can run two reads side by side, each on its own pool. A
/// connection one of them gives back while the other waits here is not kept
/// either ([`Checkout`]): kept, it would be held across the rest of this
/// wait, and requests each keeping one pool's connection so while they wait
/// for another's stall as above (#6027). A request's wait on another
/// server, the terminology server or the blob store, goes through here too
/// (#5774). No specification governs this: our own design.
pub async fn handoff<F: Future>(wait: F) -> F::Output {
    let _waiting = Waiting::begin();
    release_held();
    let output = wait.await;
    kept_while_waiting();
    output
}

/// Awaits `wait`, a [`handoff`] unless it is ready at once.
///
/// For a wait that waits for another task's work only some of the times it
/// runs: a copy another request is loading, or a lock another holds. Ready
/// at once, it gives nothing back. No specification governs this: our own
/// design (#6027).
pub async fn awaiting<F: Future>(wait: F) -> F::Output {
    let mut wait = std::pin::pin!(wait);
    let first = std::future::poll_fn(|cx| std::task::Poll::Ready(wait.as_mut().poll(cx))).await;
    match first {
        std::task::Poll::Ready(output) => output,
        std::task::Poll::Pending => handoff(wait).await,
    }
}

/// One wait of the current [`request_scope`] in progress, counted in
/// [`WAITS`] until it drops, which a request dropped mid-wait does too.
struct Waiting {
    counted: bool,
}

impl Waiting {
    /// Counts a wait of the current scope; outside a scope it counts nothing.
    fn begin() -> Self {
        let counted = WAITS
            .try_with(|waits| waits.set(waits.get().saturating_add(1)))
            .is_ok();
        Self { counted }
    }
}

impl Drop for Waiting {
    fn drop(&mut self) {
        if self.counted {
            // NOTE: our own design (#6027), the scope outlives every wait counted in it.
            let _outside_a_scope = WAITS.try_with(|waits| waits.set(waits.get().saturating_sub(1)));
        }
    }
}

/// Whether the current [`request_scope`] waits for a pool or for another
/// task's work now.
fn waiting() -> bool {
    WAITS.try_with(|waits| waits.get() > 0).unwrap_or(false)
}

/// A wait of the current [`request_scope`] ended with a connection kept:
/// the request held that connection idle while it waited. Debug builds
/// refuse it; a release build counts it on
/// [`DB_POOL_NESTED_ACQUIRES`](crate::telemetry::metrics::DB_POOL_NESTED_ACQUIRES)
/// with `order="kept_while_waiting"` and warns. No openEHR spec governs
/// this: our own design (#6027).
fn kept_while_waiting() {
    let kept: Vec<&'static str> = HELD
        .try_with(|held| {
            held.try_borrow()
                .map(|held| held.iter().map(|kept| label(kept.name)).collect())
                .unwrap_or_default()
        })
        .unwrap_or_default();
    debug_assert!(
        kept.is_empty(),
        "a request kept a connection while it waited for a pool or another task (#6027)"
    );
    for pool in kept {
        crate::telemetry::metrics::metrics()
            .db_pool_nested_acquires
            .add(
                1,
                &[
                    KeyValue::new(POOL, pool),
                    KeyValue::new("order", "kept_while_waiting"),
                ],
            );
        tracing::warn!(
            pool,
            "a request kept a connection while it waited for a pool or another task"
        );
    }
}

/// Records that a request held a connection of the pool `name` from `since`
/// until now.
fn held_for(since: Instant, name: Option<PoolName>) {
    crate::telemetry::metrics::metrics()
        .db_pool_hold_duration
        .record(
            since.elapsed().as_secs_f64(),
            &[KeyValue::new(POOL, label(name))],
        );
}

/// The identity of the shared pool `pool` is a handle to: the address of the
/// options every handle of it reads.
fn pool_key(pool: &PgPool) -> usize {
    std::ptr::from_ref(pool.options()).addr()
}

/// Gives back to its pool every connection the current [`request_scope`]
/// keeps of a storage domain other than the one of `pool`, whose key is
/// `key`, recording how long the request held each.
///
/// A request reads the party store for its authentication and access
/// decision and then does its clinical work, or the reverse, and seldom
/// turns back: kept, the first domain's connection would sit idle until the
/// handler answers, several times as long as its statements ran (#5774). A
/// request that does turn back checks out again. An audit connection is
/// never given back here: a write takes it before its domain connection and
/// keeps it for its transaction (#5964), and the audit pool is never asked
/// for one while a domain connection is held. No openEHR spec governs
/// this: our own design.
fn release_other_domain(pool: &PgPool, key: usize) {
    let other = |kept: &Held, domain: Domain| {
        kept.pool != key
            && kept
                .name
                .is_some_and(|name| name.domain() != Domain::Audit && name.domain() != domain)
    };
    // NOTE: our own design (#5774), outside a scope nothing is kept.
    let _outside_a_scope = HELD.try_with(|held| {
        let Ok(mut held) = held.try_borrow_mut() else {
            return;
        };
        if !held.iter().any(|kept| other(kept, Domain::Audit)) {
            return;
        }
        let Some(domain) = name_of(pool)
            .map(PoolName::domain)
            .filter(|domain| *domain != Domain::Audit)
        else {
            return;
        };
        held.retain(|kept| {
            let give_back = other(kept, domain);
            if give_back {
                held_for(kept.since, kept.name);
            }
            !give_back
        });
    });
}

/// The connection the current [`request_scope`] keeps for the pool `key`,
/// taken out of the scope; `None` outside a scope or when it keeps none.
fn take_held(key: usize) -> Option<Held> {
    HELD.try_with(|held| {
        let mut held = held.try_borrow_mut().ok()?;
        let at = held.iter().position(|kept| kept.pool == key)?;
        Some(held.swap_remove(at))
    })
    .ok()
    .flatten()
}

/// A connection [`acquire`] handed out. It derefs to the connection.
///
/// When it drops, the connection goes back to the current [`request_scope`]
/// when that keeps none of its pool yet and waits for nothing ([`handoff`]),
/// and to the pool otherwise.
#[derive(Debug)]
pub struct Checkout {
    conn: Option<PoolConnection<Postgres>>,
    pool: usize,
    since: Instant,
    name: Option<PoolName>,
    close: bool,
}

impl Checkout {
    /// A checkout of `conn`, a connection of the pool `pool` named `name`,
    /// taken from it at `since`.
    fn new(
        conn: PoolConnection<Postgres>,
        pool: usize,
        since: Instant,
        name: Option<PoolName>,
    ) -> Self {
        note_checkout(pool, name, true);
        Self {
            conn: Some(conn),
            pool,
            since,
            name,
            close: false,
        }
    }

    /// Closes the connection when the checkout drops, instead of keeping it or
    /// returning it to the pool.
    pub fn close_on_drop(&mut self) {
        self.close = true;
    }

    /// Takes the connection out of its pool for good: the pool opens another
    /// in its place, and the caller closes this one.
    ///
    /// # Panics
    ///
    /// Never: only this method and `Drop::drop` take the connection out, and
    /// both consume the checkout.
    #[must_use]
    #[expect(
        clippy::expect_used,
        reason = "the connection is taken out only here and by `Drop::drop`, and both consume the checkout"
    )]
    pub fn detach(mut self) -> PgConnection {
        self.conn
            .take()
            .expect("the connection should be held until the checkout is consumed")
            .detach()
    }
}

impl Deref for Checkout {
    type Target = PgConnection;

    #[expect(
        clippy::expect_used,
        reason = "the connection is taken out only by `Drop::drop`, after which nothing derefs the checkout"
    )]
    fn deref(&self) -> &PgConnection {
        self.conn
            .as_deref()
            .expect("the connection should be held until the checkout drops")
    }
}

impl DerefMut for Checkout {
    #[expect(
        clippy::expect_used,
        reason = "the connection is taken out only by `Drop::drop`, after which nothing derefs the checkout"
    )]
    fn deref_mut(&mut self) -> &mut PgConnection {
        self.conn
            .as_deref_mut()
            .expect("the connection should be held until the checkout drops")
    }
}

impl Drop for Checkout {
    fn drop(&mut self) {
        note_checkout(self.pool, self.name, false);
        let Some(mut conn) = self.conn.take() else {
            return;
        };
        if self.close {
            conn.close_on_drop();
            return;
        }
        let (pool, since, name) = (self.pool, self.since, self.name);
        // Outside a scope the closure is dropped unrun, and dropping `conn`
        // returns it to the pool; a scope already keeping a connection of
        // this pool, or waiting for a pool or another task, hands it back,
        // and the request's hold of it ends here.
        let waiting = waiting();
        let returned = HELD.try_with(move |held| match held.try_borrow_mut() {
            Ok(mut held) if !waiting && !held.iter().any(|kept| kept.pool == pool) => {
                held.push(Held {
                    pool,
                    conn,
                    since,
                    name,
                });
                None
            }
            _ => {
                held_for(since, name);
                Some(conn)
            }
        });
        drop(returned);
    }
}

/// Obtains a connection from `pool`, marking any failure as acquire-time.
///
/// Inside a [`request_scope`] that keeps a connection of `pool`, that
/// connection is handed out without asking the pool. Otherwise the wait,
/// successful or not, is recorded on
/// [`DB_POOL_ACQUIRE_DURATION`](crate::telemetry::metrics::DB_POOL_ACQUIRE_DURATION).
///
/// The connection is released when the returned guard drops; a caller that
/// executes one statement passes `&mut *acquire(pool).await?` so the guard
/// drops at the end of that statement, as an implicit acquire would.
///
/// # Errors
///
/// [`AcquireError`] when the pool timed out, was closed, or could not open a
/// new connection.
pub async fn acquire(pool: &PgPool) -> Result<Checkout, AcquireError> {
    let key = pool_key(pool);
    release_other_domain(pool, key);
    if let Some(held) = take_held(key) {
        return Ok(Checkout::new(held.conn, key, held.since, held.name));
    }
    let started = Instant::now();
    let name = name_of(pool);
    if checked_out(key) {
        nested_acquire(label(name));
    }
    if name.is_some_and(|name| name.domain() == Domain::Audit) && holds_a_domain() {
        out_of_order(label(name));
    }
    if let Some(domain) = name
        .map(PoolName::domain)
        .filter(|domain| *domain != Domain::Audit)
        && writes_another_domain(domain)
    {
        inside_a_write(label(name));
    }
    // Boxed: the probe and the pool's connect future are large, and every
    // caller's state machine would otherwise carry them inline.
    let mut waiting = Box::pin(acquire_unmetered(pool));
    let first = std::future::poll_fn(|cx| std::task::Poll::Ready(waiting.as_mut().poll(cx))).await;
    let result = match first {
        std::task::Poll::Ready(result) => result,
        std::task::Poll::Pending => {
            // NOTE: our own design (#5879), a request about to wait for one pool gives back what
            // it keeps of the others: kept across the wait, they make the pools locks taken in no
            // fixed order, and requests each keeping one and waiting for another stall to timeout.
            handoff(waiting).await
        }
    };
    crate::telemetry::metrics::metrics()
        .db_pool_acquire_duration
        .record(
            started.elapsed().as_secs_f64(),
            &[KeyValue::new(POOL, label(name))],
        );
    let conn = result?;
    Ok(Checkout::new(conn, key, Instant::now(), name))
}

/// A request asks a pool for a second connection while it has one of that
/// pool checked out: once every connection is held so, each such request
/// waits for one only another of them could give back. Debug builds refuse
/// it, so a test that reaches such a site fails; a release build counts it
/// on [`DB_POOL_NESTED_ACQUIRES`](crate::telemetry::metrics::DB_POOL_NESTED_ACQUIRES)
/// and warns. No openEHR spec governs this: our own design (#5879).
fn nested_acquire(label: &'static str) {
    debug_assert!(
        false,
        "a request asked a pool for a second connection while it held one of it (#5879)"
    );
    crate::telemetry::metrics::metrics()
        .db_pool_nested_acquires
        .add(1, &[KeyValue::new(POOL, label)]);
    tracing::warn!(
        pool = label,
        "a request asked a pool for a second connection while it held one of it"
    );
}

/// A request asks the audit pool for a connection while it has a clinical or
/// party connection checked out: the pools are taken audit first, then the
/// domain, so a write never waits for the audit pool inside its transaction
/// and no two requests wait for each other across pools. Debug builds refuse
/// it; a release build counts it on
/// [`DB_POOL_NESTED_ACQUIRES`](crate::telemetry::metrics::DB_POOL_NESTED_ACQUIRES)
/// with `order="audit_after_domain"` and warns. No openEHR spec governs
/// this: our own design (#5964).
fn out_of_order(label: &'static str) {
    debug_assert!(
        false,
        "a request asked the audit pool for a connection while it held a clinical or party one (#5964)"
    );
    crate::telemetry::metrics::metrics()
        .db_pool_nested_acquires
        .add(
            1,
            &[
                KeyValue::new(POOL, label),
                KeyValue::new("order", "audit_after_domain"),
            ],
        );
    tracing::warn!(
        pool = label,
        "a request asked the audit pool for a connection while it held a clinical or party one"
    );
}

/// A request asks one storage domain's pool for a connection inside a write
/// of the other domain, holding the write's audit and domain connections:
/// once each pool is held so, the writes wait for each other until the
/// acquire timeout. A write reads what it needs of the other domain before
/// it takes its connections. Debug builds refuse it; a release build counts
/// it on
/// [`DB_POOL_NESTED_ACQUIRES`](crate::telemetry::metrics::DB_POOL_NESTED_ACQUIRES)
/// with `order="inside_a_write"` and warns. No openEHR spec governs this:
/// our own design (#6186).
fn inside_a_write(label: &'static str) {
    debug_assert!(
        false,
        "a request asked one domain's pool for a connection inside a write of the other (#6186)"
    );
    crate::telemetry::metrics::metrics()
        .db_pool_nested_acquires
        .add(
            1,
            &[
                KeyValue::new(POOL, label),
                KeyValue::new("order", "inside_a_write"),
            ],
        );
    tracing::warn!(
        pool = label,
        "a request asked one domain's pool for a connection inside a write of the other"
    );
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
    conn: Checkout,
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
