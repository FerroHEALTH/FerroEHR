// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The copies a read found older than the store, and how long it waits.
//!
//! A copy of the EHR access settings a read found older than the row in force
//! is dropped before its next read of the EHR (#5589); a request waits a
//! bounded time for a copy older than the store (#5794, #5806).

use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use crate::ids::EhrId;

/// How long a request waits for a copy older than the store before it is
/// refused with `503`.
pub const STALE_WAIT: Duration = Duration::from_millis(100);

/// The pause between two reads that found the copy still older.
pub const STALE_POLL: Duration = Duration::from_millis(10);

/// The EHRs whose copy on this replica a read found older than the row in
/// force: the copy drops each before its next read of the EHR.
///
/// A replica learns of a change committed on another one by its
/// notification, which arrives after the commit. A read in between compares
/// the revision its verdict was judged on with the row it reads, refuses on
/// a mismatch, and marks the copy here, so the next request reads the row.
/// No openEHR spec governs this: our own design (#5589).
#[derive(Debug, Default)]
pub struct StaleCopies(Mutex<BTreeSet<EhrId>>);

impl StaleCopies {
    /// Marks the copy of `ehr_id` stale.
    pub fn mark(&self, ehr_id: EhrId) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(ehr_id);
    }

    /// The EHRs marked since the last call, which are no longer marked.
    #[must_use]
    pub fn take(&self) -> BTreeSet<EhrId> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}
