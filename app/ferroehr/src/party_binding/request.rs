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
//! through the one function `party_door.party_revisions(held)`, which judges
//! the caller facts against the snapshot the facts copy held when the ride
//! began (`party.facts_behind`, #6074), so the request is
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

use std::sync::Arc;

use crate::db::acquire::acquire;
use crate::db::snapshot::Snapshot;
use crate::party_binding::grants::live::revisions::{FactsRead, HeldSnapshot, Revisions};
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
    held: RefCell<Held>,
    observed: RefCell<Option<Revisions>>,
    read: RefCell<Option<Revisions>>,
    /// The snapshot the facts copy held when the ride began, which its door
    /// reads judge the caller facts against.
    door: Option<HeldSnapshot>,
    op: Cell<Option<&'static str>>,
    deferred: RefCell<Vec<Deferred>>,
}

/// The least revisions the copies a ride decided on held; `None` where it
/// took no copy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Held {
    grants: Option<i64>,
    facts: Option<HeldSnapshot>,
    bindings: Option<i64>,
}

/// The least of `held` and `at`.
fn least(held: Option<i64>, at: i64) -> i64 {
    held.map_or(at, |held| held.min(at))
}

impl Held {
    /// Whether the copies held every change up to `store`.
    fn covers(&self, store: &Revisions) -> bool {
        self.grants.is_none_or(|held| held >= store.grants)
            && self
                .facts
                .as_ref()
                .is_none_or(|held| held.holds(&store.facts))
            && self.bindings.is_none_or(|held| held >= store.bindings)
    }

    /// Whether the run took any copy.
    fn any(&self) -> bool {
        self.grants.is_some() || self.facts.is_some() || self.bindings.is_some()
    }
}

/// The facts a door read found against the install `door`: none unseen, or
/// some, under a snapshot the read does not name.
fn door_read(door: Option<&HeldSnapshot>, behind: bool) -> FactsRead {
    match door {
        Some(door) if !behind => FactsRead::Seen {
            generation: door.generation,
        },
        _ => FactsRead::Behind { store: None },
    }
}

/// What two reads of the facts within one ride require together. Every
/// door read is judged against the install the ride began at, which every
/// copy it decided on is no older than, so one that found a stamp unseen
/// decides; of two snapshots read in turn, the later covers the earlier.
fn newer(seen: FactsRead, store: FactsRead) -> FactsRead {
    match (seen, store) {
        (FactsRead::Seen { generation: a }, FactsRead::Seen { generation: b }) => FactsRead::Seen {
            generation: a.max(b),
        },
        (FactsRead::Behind { store: Some(a) }, FactsRead::Behind { store: Some(b) }) => {
            FactsRead::Behind {
                store: Some(if b.covers(&a) { b } else { a }),
            }
        }
        (FactsRead::Behind { store: None }, _) | (_, FactsRead::Behind { store: None }) => {
            FactsRead::Behind { store: None }
        }
        (behind @ FactsRead::Behind { .. }, FactsRead::Seen { .. })
        | (FactsRead::Seen { .. }, behind @ FactsRead::Behind { .. }) => behind,
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
    pub async fn held(mut self, pool: &PgPool) -> Result<bool, StorageError> {
        if !self.held.any() {
            self.count();
            return Ok(true);
        }
        let store = if let Some(store) = self.reference.take() {
            store
        } else {
            let mut conn = acquire(pool).await?;
            store::revisions(&mut conn, self.held.facts.as_ref()).await?
        };
        let held = self.held.covers(&store);
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
/// revisions are taken from the statements that read them, the caller facts
/// judged against `door`, the snapshot the facts copy holds as it begins.
/// Inside a [`scope`] only.
pub async fn ride<F: Future>(door: Option<HeldSnapshot>, fut: F) -> (F::Output, Ridden) {
    // NOTE: our own design (#6320), revisions the request read before its ride (the token's
    // standing) are what its copies are judged by, and no statement of the ride reads them again.
    let ride = Ride {
        read: RefCell::new(READ.try_with(|read| read.get().cloned()).ok().flatten()),
        door,
        ..Ride::default()
    };
    RIDE.scope(ride, async move {
        let output = fut.await;
        let ridden = RIDE.with(|ride| {
            let observed = ride.observed.take();
            Ridden {
                held: ride.held.take(),
                reference: observed.or_else(|| ride.read.take()),
                op: ride.op.get(),
                deferred: ride.deferred.take(),
            }
        });
        (output, ridden)
    })
    .await
}

/// Whether the running request rides, so its clinical statement reads the
/// party store's revisions through `party_door.party_revisions(held)`.
#[must_use]
pub fn riding() -> bool {
    RIDE.try_with(|_| ()).is_ok()
}

/// Whether a statement of the running request selects the party store's
/// revisions through `party_door.party_revisions(held)`: it rides and has not
/// read them yet.
#[must_use]
pub fn reads_door() -> bool {
    RIDE.try_with(|ride| ride.read.borrow().is_none())
        .unwrap_or(false)
}

/// The snapshot a door read of the running ride binds as `held`; `None`
/// outside a ride or before the facts copy's first read.
#[must_use]
pub fn door() -> Option<Arc<Snapshot>> {
    RIDE.try_with(|ride| ride.door.as_ref().map(|door| Arc::clone(&door.snapshot)))
        .ok()
        .flatten()
}

/// Notes that the running ride decided on the grants copy at `grants` and
/// the facts copy at `facts`.
pub(crate) fn held(grants: i64, facts: &HeldSnapshot) {
    // NOTE: tokio LocalKey::try_with, outside a ride every copy is confirmed by its read.
    let _outside = RIDE.try_with(|ride| {
        let mut held = ride.held.borrow_mut();
        held.grants = Some(least(held.grants, grants));
        if held
            .facts
            .as_ref()
            .is_none_or(|older| older.generation > facts.generation)
        {
            held.facts = Some(facts.clone());
        }
    });
}

/// Notes that the running ride decided on a binding lookup held at `at`.
pub(crate) fn held_binding(at: i64) {
    // NOTE: tokio LocalKey::try_with, outside a ride every copy is confirmed by its read.
    let _outside = RIDE.try_with(|ride| {
        let mut held = ride.held.borrow_mut();
        held.bindings = Some(least(held.bindings, at));
    });
}

/// Notes the party store's grant and binding revisions a statement of the
/// running ride read through its door, and whether it found a caller facts
/// stamp the ride's door snapshot did not see; the newest read stands.
pub(crate) fn observe_door(grants: i64, facts_behind: bool, bindings: i64) {
    // NOTE: tokio LocalKey::try_with, outside a ride no statement reads them.
    let _outside = RIDE.try_with(|ride| {
        let store = Revisions {
            grants,
            facts: door_read(ride.door.as_ref(), facts_behind),
            bindings,
        };
        let mut observed = ride.observed.borrow_mut();
        let next = match observed.take() {
            Some(seen) => Revisions {
                grants: seen.grants.max(store.grants),
                facts: newer(seen.facts, store.facts),
                bindings: seen.bindings.max(store.bindings),
            },
            None => store,
        };
        *observed = Some(next);
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
pub async fn scope<F: Future>(fut: F) -> F::Output {
    READ.scope(OnceCell::new(), fut).await
}

/// The revisions in force in the party store, the caller facts judged
/// against `held`, the snapshot the facts copy holds: the ones the running
/// request read, or one indexed read on a connection of `pool` that is given
/// back before this returns.
///
/// # Errors
/// [`StorageError`] when the party store does not answer; a failed read is
/// not kept, and the next caller reads again.
pub(crate) async fn revisions(
    pool: &PgPool,
    held: Option<&HeldSnapshot>,
) -> Result<Revisions, StorageError> {
    let read = || async {
        let mut conn = acquire(pool).await?;
        store::revisions(&mut conn, held).await
    };
    // NOTE: tokio LocalKey::try_with, outside a request (a background task, a
    // test of a service) there is nothing to share the read with.
    let Ok(read_before) = READ.try_with(|read| read.get().cloned()) else {
        return read().await;
    };
    if let Some(read_before) = read_before {
        noted_read(&read_before);
        return Ok(read_before);
    }
    let fresh = read().await?;
    noted_read(&fresh);
    // NOTE: tokio OnceCell::set, a read a sibling future of the request
    // finished first stands, and both are revisions the store held.
    Ok(READ
        .try_with(|read| {
            match read.set(fresh.clone()) {
                Ok(()) | Err(_) => {}
            }
            read.get().cloned().unwrap_or_else(|| fresh.clone())
        })
        .unwrap_or(fresh))
}

/// Notes that the running ride read the store's revisions as `store`, which
/// its copies are then judged against when no statement read them.
fn noted_read(store: &Revisions) {
    // NOTE: tokio LocalKey::try_with, outside a ride the read confirms the copies at once.
    let _outside = RIDE.try_with(|ride| {
        let mut read = ride.read.borrow_mut();
        if read.is_none() {
            *read = Some(store.clone());
        }
    });
}
