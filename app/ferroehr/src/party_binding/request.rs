// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The party store's revisions as one request reads them (#6048).
//!
//! Every fact of the party store a decision rests on is held in a copy this
//! replica keeps by notification: the binding lookups, the grants and the
//! caller facts. A notification arrives after the commit it reports, so each
//! copy is taken only once the request has read the store's revisions and
//! found the copy at them. A request reads them once, in one indexed read on
//! a connection of its own, at the first decision that needs them, and its
//! later decisions take the same read; outside a request every caller reads
//! them itself.
//!
//! A request may instead [`ride`] (#6320): where the party domain shares the
//! clinical database, its clinical statement reads the store's revisions
//! through the one function `party_door.party_revisions()`, so the request is
//! decided on the copies as they stand, each copy noting what it holds
//! (`held`, `held_binding`), and its answer leaves only once those cover
//! the revisions a statement read (`observe`), or, when none read them, one
//! indexed read finds. A run the store overtook leaves nothing: what it would
//! have counted waits in [`defer`] and goes with it. No openEHR spec governs
//! the copies: our own design.

use std::cell::{Cell, RefCell};
use std::future::Future;

use sqlx::PgPool;
use tokio::sync::OnceCell;

use crate::db::acquire::acquire;
use crate::party_binding::grants::live::revisions::Revisions;
use crate::party_binding::grants::store;
use crate::storage::error::StorageError;

tokio::task_local! {
    /// The revisions the running request read; empty until its first read.
    static READ: OnceCell<Revisions>;
    /// The running ride: what the copies held and what the store's
    /// revisions were read as.
    static RIDE: Ride;
}

/// A counter increment held back until the run it belongs to is answered.
type Deferred = Box<dyn FnOnce() + Send>;

/// What one ride's run left.
#[derive(Default)]
struct Ride {
    held: Cell<Held>,
    observed: Cell<Option<Revisions>>,
    read: Cell<Option<Revisions>>,
    op: Cell<Option<&'static str>>,
    deferred: RefCell<Vec<Deferred>>,
}

/// The least revisions the copies a ride decided on held; `None` where it
/// took no copy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Held {
    grants: Option<i64>,
    facts: Option<i64>,
    bindings: Option<i64>,
}

/// The least of `held` and `at`.
fn least(held: Option<i64>, at: i64) -> i64 {
    held.map_or(at, |held| held.min(at))
}

impl Held {
    /// Whether the copies held every change up to `store`.
    fn covers(self, store: Revisions) -> bool {
        self.grants.is_none_or(|held| held >= store.grants)
            && self.facts.is_none_or(|held| held >= store.facts)
            && self.bindings.is_none_or(|held| held >= store.bindings)
    }

    /// Whether the run took any copy.
    fn any(self) -> bool {
        self.grants.is_some() || self.facts.is_some() || self.bindings.is_some()
    }
}

/// What a ride's run comes to.
#[must_use = "a run the store overtook must not be answered"]
pub struct Ridden {
    held: Held,
    reference: Option<Revisions>,
    op: Option<&'static str>,
    deferred: Vec<Deferred>,
}

impl std::fmt::Debug for Ridden {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ridden")
            .field("held", &self.held)
            .field("reference", &self.reference)
            .field("op", &self.op)
            .field("deferred", &self.deferred.len())
            .finish()
    }
}

impl Ridden {
    /// Whether the run may be answered: the copies it decided on held the
    /// revisions its statements read, or, when none read them, the revisions
    /// one indexed read on a connection of `pool` finds now. An answered run
    /// counts what it held back.
    ///
    /// # Errors
    /// [`StorageError`] when the party store does not answer that read.
    pub async fn held(self, pool: &PgPool) -> Result<bool, StorageError> {
        if !self.held.any() {
            self.count();
            return Ok(true);
        }
        let store = if let Some(store) = self.reference {
            store
        } else {
            let mut conn = acquire(pool).await?;
            store::revisions(&mut conn).await?
        };
        let held = self.held.covers(store);
        if held {
            self.count();
        }
        Ok(held)
    }

    /// The operation the run served, when a guarded dispatch named it.
    #[must_use]
    pub fn op(&self) -> Option<&'static str> {
        self.op
    }

    /// Counts what the run held back.
    fn count(self) {
        for deferred in self.deferred {
            deferred();
        }
    }
}

/// Runs `fut` as a ride: the copies decide as they stand, and the store's
/// revisions are taken from the statements that read them. Inside a
/// [`scope`] only.
pub async fn ride<F: Future>(fut: F) -> (F::Output, Ridden) {
    // NOTE: our own design (#6320), revisions the request read before its ride (the token's
    // standing) are what its copies are judged by, and no statement of the ride reads them again.
    let ride = Ride {
        read: Cell::new(READ.try_with(|held| held.get().copied()).ok().flatten()),
        ..Ride::default()
    };
    RIDE.scope(ride, async move {
        let output = fut.await;
        let ridden = RIDE.with(|ride| {
            let observed = ride.observed.get();
            Ridden {
                held: ride.held.get(),
                reference: observed.or(ride.read.get()),
                op: ride.op.get(),
                deferred: ride.deferred.take(),
            }
        });
        (output, ridden)
    })
    .await
}

/// Whether the running request rides, so its clinical statement reads the
/// party store's revisions through `party_door.party_revisions()`.
#[must_use]
pub fn riding() -> bool {
    RIDE.try_with(|_| ()).is_ok()
}

/// Whether a statement of the running request selects the party store's
/// revisions through `party_door.party_revisions()`: it rides and has not
/// read them yet.
#[must_use]
pub fn reads_door() -> bool {
    RIDE.try_with(|ride| ride.read.get().is_none())
        .unwrap_or(false)
}

/// Notes that the running ride decided on the grants copy at `grants` and
/// the facts copy at `facts`.
pub(crate) fn held(grants: i64, facts: i64) {
    // NOTE: tokio LocalKey::try_with, outside a ride every copy is confirmed by its read.
    let _outside = RIDE.try_with(|ride| {
        let held = ride.held.get();
        ride.held.set(Held {
            grants: Some(least(held.grants, grants)),
            facts: Some(least(held.facts, facts)),
            ..held
        });
    });
}

/// Notes that the running ride decided on a binding lookup held at `at`.
pub(crate) fn held_binding(at: i64) {
    // NOTE: tokio LocalKey::try_with, outside a ride every copy is confirmed by its read.
    let _outside = RIDE.try_with(|ride| {
        let held = ride.held.get();
        ride.held.set(Held {
            bindings: Some(least(held.bindings, at)),
            ..held
        });
    });
}

/// Notes the party store's revisions a statement of the running ride read;
/// the newest read stands.
pub(crate) fn observe(store: Revisions) {
    // NOTE: tokio LocalKey::try_with, outside a ride no statement reads them.
    let _outside = RIDE.try_with(|ride| {
        let next = match ride.observed.get() {
            Some(seen) => Revisions {
                grants: seen.grants.max(store.grants),
                facts: seen.facts.max(store.facts),
                bindings: seen.bindings.max(store.bindings),
            },
            None => store,
        };
        ride.observed.set(Some(next));
    });
}

/// Names the operation the running ride serves.
pub fn serving(op: &'static str) {
    // NOTE: tokio LocalKey::try_with, outside a ride nothing is named.
    let _outside = RIDE.try_with(|ride| ride.op.set(Some(op)));
}

/// Counts `count` now, or, inside a ride, once its run is answered.
pub fn defer(count: impl FnOnce() + Send + 'static) {
    let mut count = Some(count);
    // NOTE: tokio LocalKey::try_with, outside a ride every count is made at once.
    let _outside = RIDE.try_with(|ride| {
        if let Some(count) = count.take() {
            ride.deferred.borrow_mut().push(Box::new(count));
        }
    });
    if let Some(count) = count {
        count();
    }
}

/// Runs `fut` as one request: the first read of the store's revisions inside
/// it serves every later one.
pub fn scope<F: Future>(fut: F) -> impl Future<Output = F::Output> {
    READ.scope(OnceCell::new(), fut)
}

/// The revisions in force in the party store: the ones the running request
/// read, or one indexed read on a connection of `pool` that is given back
/// before this returns.
///
/// # Errors
/// [`StorageError`] when the party store does not answer; a failed read is
/// not kept, and the next caller reads again.
pub(crate) async fn revisions(pool: &PgPool) -> Result<Revisions, StorageError> {
    let read = || async {
        let mut conn = acquire(pool).await?;
        store::revisions(&mut conn).await
    };
    // NOTE: tokio LocalKey::try_with, outside a request (a background task, a
    // test of a service) there is nothing to share the read with.
    let Ok(held) = READ.try_with(|held| held.get().copied()) else {
        return read().await;
    };
    if let Some(held) = held {
        noted_read(held);
        return Ok(held);
    }
    let fresh = read().await?;
    noted_read(fresh);
    // NOTE: tokio OnceCell::set, a read a sibling future of the request
    // finished first stands, and both are revisions the store held.
    Ok(READ
        .try_with(|held| {
            match held.set(fresh) {
                Ok(()) | Err(_) => {}
            }
            held.get().copied().unwrap_or(fresh)
        })
        .unwrap_or(fresh))
}

/// Notes that the running ride read the store's revisions as `store`, which
/// its copies are then judged against when no statement read them.
fn noted_read(store: Revisions) {
    // NOTE: tokio LocalKey::try_with, outside a ride the read confirms the copies at once.
    let _outside = RIDE.try_with(|ride| {
        if ride.read.get().is_none() {
            ride.read.set(Some(store));
        }
    });
}
