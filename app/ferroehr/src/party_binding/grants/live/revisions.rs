// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Whether [`LiveGrants`] holds the revisions in force in the store.
//!
//! A replica learns of a change another one committed from its notification,
//! which arrives after the commit. A decision that care admits compares the
//! revisions it was judged under with the store's first, so a removal another
//! replica committed is never read from an older copy (#5806). No openEHR
//! spec governs the copy: our own design.

use super::{GrantsUnavailable, LiveGrants};

/// The revisions of the party store: of the grants, of the caller facts and
/// of the binding side table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Revisions {
    pub(crate) grants: i64,
    pub(crate) facts: i64,
    pub(crate) bindings: i64,
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
    /// than them, from the indexed read the request makes
    /// ([`crate::party_binding::request`]); `None` when the copy
    /// holds them, or when the request rides and the copy is noted as held
    /// for the ride to be judged by.
    ///
    /// # Errors
    /// [`GrantsUnavailable`] when the party store does not answer.
    pub async fn behind(&self) -> Result<Option<Revisions>, GrantsUnavailable> {
        // NOTE: our own design (#6320), a ride decides on the copy as it stands and is answered
        // only once the revisions its clinical statement read are the copy's or older.
        if crate::party_binding::request::riding()
            && !self.listened.must_read()
            && let (Some(grants), Some(facts)) = (self.revision(), self.facts_revision())
        {
            crate::party_binding::request::held(grants, facts);
            return Ok(None);
        }
        let store = crate::party_binding::request::revisions(&self.pool).await?;
        Ok((!self.holds(store)).then_some(store))
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
