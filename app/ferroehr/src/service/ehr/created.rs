// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The representation of each EHR this replica just created, kept so the
//! `Prefer: return=representation` answer of its create reads nothing
//! again. An entry is taken by that answer, and one the cap or its
//! time-to-live dropped is read from the store, which holds the same body
//! (#5926). No openEHR spec governs the copy: our own design.

use std::time::Duration;

use moka::future::Cache;
use serde_json::Value;

use crate::ids::EhrId;

/// The most created EHRs whose representation a replica keeps.
pub(crate) const CREATED_EHRS: u64 = 4096;

/// How long a created EHR's representation is kept for its answer.
const KEPT: Duration = Duration::from_secs(30);

/// The kept representations, by EHR.
pub(crate) type CreatedEhrs = Cache<EhrId, Value>;

/// An empty copy of at most `capacity` representations.
pub(crate) fn created_ehrs(capacity: u64) -> CreatedEhrs {
    Cache::builder()
        .max_capacity(capacity)
        .time_to_live(KEPT)
        .build()
}

#[cfg(test)]
#[path = "created_tests.rs"]
mod tests;
