// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The caller's own records a query withholds held data on, by the party
//! binding (EHDS Art. 3(3), #6108).

use std::sync::Arc;

/// The subject references of the caller's own records by the party binding.
///
/// A query compares each row's EHR subject with them. No openEHR spec
/// governs this: our own design under EHDS Art. 3(3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnRecords {
    /// Each reference as its `(id, namespace)`, as the `ehr` row stores the
    /// subject, read through the subject's unique index.
    pub subjects: Arc<[(String, String)]>,
}
