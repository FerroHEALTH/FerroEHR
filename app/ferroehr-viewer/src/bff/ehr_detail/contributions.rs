// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR-detail Contributions tab: the per-EHR activity timeline, the paged
//! contribution list, and the by-uid CONTRIBUTION lookup box.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST, not the CDR internal seams \
              (#1694)"
)]

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::activity::ActivityPoint;
use crate::error::ViewerError;
use crate::locale::records::RecordsMsg;
use crate::locale::tr_with;

/// Contributions read for the activity timeline before day-bucketing. The
/// timeline answers a different question than the list ("when was this EHR
/// written to", not "which contributions are on this page"), so it reads its own
/// window of the SAME endpoint — one reader per claim, never a second endpoint
/// for the same fact.
const ACTIVITY_FETCH: u32 = 200;

/// One row of the EHR's contribution list. Fixed-size-safe strings; the shape
/// is the CDR's contribution-list contract (`{uid, time_committed, committer,
/// change_type, change_type_rubric}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributionRow {
    /// The CONTRIBUTION uid.
    pub uid: String,
    /// `AUDIT_DETAILS.time_committed` value.
    pub time_committed: String,
    /// `AUDIT_DETAILS.committer` name.
    pub committer: String,
    /// `AUDIT_DETAILS.change_type` group code (e.g. `249`).
    pub change_type: String,
    /// The CDR-resolved display rubric for the code (e.g. `creation`) — the
    /// viewer never maps codes locally; empty when the CDR sent none.
    pub change_type_rubric: String,
}

/// One page of an EHR's contributions: the rows plus the total count (for
/// prev/next).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributionPage {
    /// The contribution rows on this page.
    pub rows: Vec<ContributionRow>,
    /// The total number of contributions in the EHR.
    pub total: u32,
}

/// List an EHR's contributions, one page at `offset`
/// (`GET /ehr/{ehr_id}/contribution?offset&fetch`), `fetch` rows long.
///
/// NOTE: no openEHR spec governs this — our own design/extension: the released
/// EHR API declares only `contribution_create` and `contribution_get` (one
/// CONTRIBUTION by uid), never a listing, so the paged route and its
/// `{rows, total}` envelope are the CDR's own.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success) — a
/// `404`/`405` from an older CDR that lacks the list route surfaces there as
/// [`ViewerError::Cdr`] (the tab renders it inline);
/// [`ViewerError::Internal`] when the page is not valid JSON.
pub async fn list_contributions(
    // The EHR whose contributions to list.
    ehr_id: String,
    // First row of the page to return.
    offset: u32,
    // How many rows the page holds.
    fetch: u32,
) -> Result<ContributionPage, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let url = state.cdr.extensions_v1(&format!(
        "ehr/{}/contribution?offset={offset}&fetch={fetch}",
        urlencoding::encode(&ehr_id),
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    parse_contributions(&response.body)
}

/// Parse the contribution-list body (`{ "rows": [...], "total": N }`) into a
/// [`ContributionPage`]. Defensive throughout — a missing field reads as empty
/// / zero rather than failing.
///
/// # Errors
/// [`ViewerError::Internal`] when the body is not valid JSON.
fn parse_contributions(body: &str) -> Result<ContributionPage, ViewerError> {
    let doc: Value = serde_json::from_str(body).map_err(|e| {
        ViewerError::Internal(tr_with(
            RecordsMsg::ContributionsJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
    let rows = doc
        .get("rows")
        .and_then(Value::as_array)
        .map(|rows| rows.iter().map(contribution_row).collect())
        .unwrap_or_default();
    let total = doc
        .get("total")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or_default();
    Ok(ContributionPage { rows, total })
}

/// Flatten one contribution-list entry into a [`ContributionRow`], each field
/// read defensively as a string.
fn contribution_row(value: &Value) -> ContributionRow {
    ContributionRow {
        uid: contribution_field(value, "uid"),
        time_committed: contribution_field(value, "time_committed"),
        committer: contribution_field(value, "committer"),
        change_type: contribution_field(value, "change_type"),
        change_type_rubric: contribution_field(value, "change_type_rubric"),
    }
}

/// Read a top-level string field, or an empty string when absent / not a string.
fn contribution_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// The EHR's contribution activity, one [`ActivityPoint`] per calendar day.
///
/// Ascending (`GET /ehr/{ehr_id}/contribution?offset=0&fetch` over
/// `ACTIVITY_FETCH`, bucketed BFF-side by
/// [`bucket_by_day`](crate::activity::bucket_by_day) on each contribution's
/// `AUDIT_DETAILS.time_committed`).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the page is not valid JSON.
pub async fn contribution_activity(
    // The EHR whose contribution instants feed the timeline.
    ehr_id: String,
) -> Result<Vec<ActivityPoint>, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let url = state.cdr.rest_v1(&format!(
        "ehr/{}/contribution?offset=0&fetch={ACTIVITY_FETCH}",
        urlencoding::encode(&ehr_id),
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    let page = parse_contributions(&response.body)?;
    let times: Vec<String> = page
        .rows
        .into_iter()
        .map(|row| row.time_committed)
        .collect();
    Ok(crate::activity::bucket_by_day(&times))
}

/// Look up a single CONTRIBUTION by uid (`GET
/// /ehr/{ehr_id}/contribution/{contribution_uid}`) — the by-uid lookup box the
/// Contributions tab keeps below its list (see `contributions_section`).
///
/// Returns the raw canonical JSON.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx CDR answer (a `404` for an unknown uid
/// included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
pub async fn fetch_contribution(
    // The EHR holding the contribution.
    ehr_id: String,
    // The CONTRIBUTION uid to read.
    contribution_uid: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let url = state.cdr.rest_v1(&format!(
        "ehr/{}/contribution/{}",
        urlencoding::encode(&ehr_id),
        urlencoding::encode(&contribution_uid)
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    Ok(crate::cdr::CdrClient::expect_success(response)?.body)
}

#[cfg(test)]
mod tests {
    use super::parse_contributions;

    #[test]
    fn parse_contributions_reads_rows_and_total_defensively() {
        let body = r#"{
            "rows": [
                {
                    "uid": "c1::sys::1",
                    "time_committed": "2026-07-12T10:00:00Z",
                    "committer": "Dr Bob",
                    "change_type": "249",
                    "change_type_rubric": "creation"
                },
                {"uid": "c2::sys::1"}
            ],
            "total": 42
        }"#;
        let page = parse_contributions(body).expect("valid contributions page");
        assert_eq!(page.total, 42);
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[0].uid, "c1::sys::1");
        assert_eq!(page.rows[0].time_committed, "2026-07-12T10:00:00Z");
        assert_eq!(page.rows[0].committer, "Dr Bob");
        assert_eq!(page.rows[0].change_type, "249");
        assert_eq!(page.rows[0].change_type_rubric, "creation");
        // A row missing fields reads as empty strings, never a parse failure.
        assert_eq!(page.rows[1].uid, "c2::sys::1");
        assert_eq!(page.rows[1].time_committed, "");
        assert_eq!(page.rows[1].committer, "");
        assert_eq!(page.rows[1].change_type, "");
        assert_eq!(page.rows[1].change_type_rubric, "");
    }

    #[test]
    fn parse_contributions_defaults_absent_rows_and_total() {
        let page = parse_contributions("{}").expect("empty object parses");
        assert_eq!(
            page.rows,
            Vec::<crate::bff::ehr_detail::contributions::ContributionRow>::new()
        );
        assert_eq!(page.total, 0);
    }

    #[test]
    fn parse_contributions_rejects_non_json() {
        assert!(parse_contributions("not json").is_err());
    }
}
