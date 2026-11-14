// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The copies of the EHR access settings a read found older than the row in
//! force, which the copy drops before its next read of the EHR (#5589).

use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};

use crate::ids::EhrId;

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
