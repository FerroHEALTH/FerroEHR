// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The openEHR access log: the draft `ACCESS_LOG_ENTRY` as FerroEHR's stored
//! access record (#5121).
//!
//! The types are the generated [`crate::access_log_draft`] module and their
//! invariants [`crate::access_log_invariants`]; [`store`] keeps the records in
//! the `audit` schema, chained over their content digests. The draft is
//! FerroEHR's change proposal and no openEHR release, so no openEHR spec
//! governs this module: our own design.

pub mod store;
