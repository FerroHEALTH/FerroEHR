// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Whether [`LiveGrants`] holds the revisions in force in the store.
//!
//! A replica learns of a change another one committed from its notification,
//! which arrives after the commit. A decision that care admits compares the
//! revisions it was judged under with the store's first, so a removal another
//! replica committed is never read from an older copy (#5806). No openEHR
//! spec governs the copy: our own design.

use super::{GrantsUnavailable, LiveGrants, store};

/// The grant revision and the facts revision of the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Revisions {
    grants: i64,
    facts: i64,
}

impl LiveGrants {
    /// The revision this replica holds; `None` before its first read.
    #[must_use]
    pub fn revision(&self) -> Option<i64> {
        self.held().revision
    }

    /// The facts revision this replica holds; `None` before its first read.
    #[must_use]
    pub fn facts_revision(&self) -> Option<i64> {
        self.held_facts().revision
    }

    /// The revisions in force in the store when this replica's copy is older
    /// than them, read in one indexed statement; `None` when the copy holds
    /// them.
    ///
    /// # Errors
    /// [`GrantsUnavailable`] when the party store does not answer.
    pub async fn behind(&self) -> Result<Option<Revisions>, GrantsUnavailable> {
        let read = async {
            let mut conn = self.pool.acquire().await?;
            store::revisions(&mut conn).await
        };
        let (grants, facts) = read.await?;
        let store = Revisions { grants, facts };
        Ok((!self.holds(store)).then_some(store))
    }

    /// Whether the copy holds at least the revisions `store`.
    #[must_use]
    pub fn holds(&self, store: Revisions) -> bool {
        self.revision().is_some_and(|held| held >= store.grants)
            && self
                .facts_revision()
                .is_some_and(|held| held >= store.facts)
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
