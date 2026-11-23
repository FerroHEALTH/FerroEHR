// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Whether [`LiveGrants`] holds the revisions in force in the store.
//!
//! A replica learns of a grant change another one committed from its
//! notification, which arrives after the commit, and of a caller facts change
//! from no notification at all (#6074). A decision that care admits compares
//! the revisions it was judged under with the store's first, so a removal
//! another replica committed is never read from an older copy (#5806). No
//! openEHR spec governs the copy: our own design.

use std::sync::Arc;

use super::{GrantsUnavailable, LiveGrants};
use crate::db::snapshot::Snapshot;

/// The revisions of the party store: of the grants and of the binding side
/// table, and whether the caller facts copy saw every stamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revisions {
    pub(crate) grants: i64,
    pub(crate) facts: FactsRead,
    pub(crate) bindings: i64,
}

/// What a read of the store's revisions found of the caller facts, against
/// the snapshot the facts copy held when it was asked (`party.facts_behind`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactsRead {
    /// The store holds no stamp the copy's snapshot of `generation` did not
    /// see.
    Seen {
        /// The install of the copy that was asked.
        generation: u64,
    },
    /// It holds one, or the copy held no snapshot: a copy holds the facts once
    /// its snapshot covers `store`, the read's own, and never when the read
    /// names none.
    Behind {
        /// The snapshot the read ran under.
        store: Option<Arc<Snapshot>>,
    },
}

/// The snapshot the facts copy holds and the install it came with, which the
/// store's revisions are read against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldSnapshot {
    pub(crate) generation: u64,
    pub(crate) snapshot: Arc<Snapshot>,
}

impl HeldSnapshot {
    /// Whether a copy at this install holds every stamp `read` found: the
    /// install asked or a later one when the read found none unseen, and a
    /// snapshot that covers the read's own when it found one.
    pub(crate) fn holds(&self, read: &FactsRead) -> bool {
        match read {
            FactsRead::Seen { generation } => self.generation >= *generation,
            FactsRead::Behind { store } => store
                .as_ref()
                .is_some_and(|store| self.snapshot.covers(store)),
        }
    }
}

impl LiveGrants {
    /// The revision this replica holds; `None` before its first read.
    #[must_use]
    pub fn revision(&self) -> Option<i64> {
        self.held().revision
    }

    /// The snapshot the facts this replica holds were read under; `None`
    /// before its first read.
    #[must_use]
    pub fn facts_snapshot(&self) -> Option<Arc<Snapshot>> {
        self.held_facts().snapshot.clone()
    }

    /// The snapshot the facts copy holds with its install, for a read of the
    /// store's revisions ([`crate::party_binding::request`]).
    #[must_use]
    pub fn held_snapshot(&self) -> Option<HeldSnapshot> {
        let held = self.held_facts();
        held.snapshot.as_ref().map(|snapshot| HeldSnapshot {
            generation: held.generation,
            snapshot: Arc::clone(snapshot),
        })
    }

    /// The revisions in force in the store when this replica's copy is older
    /// than them, from the indexed read the request makes
    /// ([`crate::party_binding::request`]); `None` when the copy
    /// holds them, or when the request rides and the copy is noted as held
    /// for the ride to be judged by.
    ///
    /// # Errors
    /// [`GrantsUnavailable`] when the party store does not answer.
    pub async fn behind(&self) -> Result<Option<Revisions>, GrantsUnavailable> {
        let held = self.held_snapshot();
        // NOTE: our own design (#6320), a ride decides on the copy as it stands and is answered
        // only once the revisions its clinical statement read are the copy's or older.
        if crate::party_binding::request::riding()
            && !self.listened.must_read()
            && let (Some(grants), Some(facts)) = (self.revision(), held.as_ref())
        {
            crate::party_binding::request::held(grants, facts);
            return Ok(None);
        }
        let store = crate::party_binding::request::revisions(&self.pool, held.as_ref()).await?;
        Ok((!self.holds(&store)).then_some(store))
    }

    /// Bring the copy to the store's revisions as the request reads them
    /// ([`crate::party_binding::request`]), so the decision that
    /// follows rests on an indexed read of the store and on no notification's
    /// delay (#6048): the rows are read when a notification marked them
    /// stale, the listener is down, or the store's revisions are past the
    /// copy's; otherwise the copy stands at the revisions just read.
    ///
    /// # Errors
    /// As [`Self::refresh`].
    pub async fn confirm(&self) -> Result<(), GrantsUnavailable> {
        if self.listened.must_read() || self.behind().await?.is_some() {
            self.refresh().await?;
        }
        Ok(())
    }

    /// Whether the copy holds at least the revisions `store`.
    #[must_use]
    pub fn holds(&self, store: &Revisions) -> bool {
        self.revision().is_some_and(|held| held >= store.grants) && self.facts_hold(&store.facts)
    }

    /// Whether the facts copy holds every stamp `read` found
    /// ([`HeldSnapshot::holds`]); never before its first read.
    pub(crate) fn facts_hold(&self, read: &FactsRead) -> bool {
        self.held_snapshot().is_some_and(|held| held.holds(read))
    }
}

#[cfg(test)]
impl LiveGrants {
    /// Hold every later read of the copy back, as a store that keeps moving
    /// past each read would.
    pub(crate) fn hold_back(&self) {
        self.held_back
            .store(true, std::sync::atomic::Ordering::Release);
    }
}
