// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Wire-facing EHR-service types.
//!
//! The EHR summary return shape and the contribution-list time-range argument.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use serde_json::Value;

/// The summary of one EHR — the `get_ehr` / `get_ehrs_for_subject` return
/// shape, all six attributes populated.
#[derive(Debug, Clone)]
pub struct EhrSummary {
    /// `ehr_id: UUID` — "EHR identifier of this EHR."
    pub ehr_id: String,
    /// `system_id: String` — "Copy of `EHR.system_id`."
    pub system_id: String,
    /// `ehr_status: EHR_STATUS` — "Copy of `EHR.ehr_status`" (canonical JSON).
    pub ehr_status: Value,
    /// `time_created: Iso8601_date_time` — "Copy of `EHR.time_created`."
    pub time_created: String,
    /// `contribution_count: Integer` — "Number of Contributions in this EHR."
    pub contribution_count: i64,
    /// `composition_count: Integer` — "Number of (versioned) Compositions in
    /// this EHR."
    pub composition_count: i64,
}

/// The optional inclusive `(lower, upper)` ISO-8601 bounds of a time interval.
///
/// The filter of `list_contributions` / `contribution_count`; either side open
/// (`None`) means unbounded.
pub type TimeRange = Option<(Option<String>, Option<String>)>;
