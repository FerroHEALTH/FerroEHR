// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/` dashboard: headline stat tiles, per-namespace stored-query match
//! tiles, and a commit-activity trend chart.
//!
//! No openEHR spec governs the viewer — our own design / product extension.
//! The wire it reads IS spec-bound: every count and the trend run against
//! `POST query/aql` (`docs/specs/openehr/ITS-REST/docs/query/`) with fixed,
//! parse-validated AQL consts — user input never reaches the query text.
//!
//! Every read guards with
//! [`require_session`](crate::session::require_session) first, and the CDR
//! credential never reaches the page. Each tile draws its own `Result`, so
//! one failing section never blanks the dashboard.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST, not the CDR internal seams \
              (#1694)"
)]

use crate::activity::ActivityPoint;
use crate::error::ViewerError;
use crate::locale::{Msg, tr};

/// Total EHRs — a fixed count AQL. Validated by the `dashboard_aql_consts_parse` test.
const EHR_COUNT_AQL: &str = "SELECT COUNT(*) FROM EHR e";
/// Total compositions — a fixed count AQL.
const COMPOSITION_COUNT_AQL: &str = "SELECT COUNT(*) FROM EHR e CONTAINS COMPOSITION c";
/// Recent composition commit times, newest first, for the day-bucketed trend.
const TREND_AQL: &str = "SELECT v/commit_audit/time_committed/value FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c ORDER BY v/commit_audit/time_committed/value DESC";
/// Rows pulled for the commit-activity trend before day-bucketing.
const TREND_FETCH: u32 = 500;

/// Why the CDR's answer to a `SELECT COUNT(*)` query is no count.
///
/// A count answers one row holding one cell, a whole number zero or above;
/// any other result set is a defective answer, kept as this error and never
/// read as 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CountShape {
    /// The result set holds no row.
    #[error("the count answer holds no row")]
    NoRow,
    /// The result set holds more than one row.
    #[error("the count answer holds more than one row")]
    SeveralRows,
    /// The row holds no cell.
    #[error("the count answer's row holds no cell")]
    NoCell,
    /// The row holds more than one cell.
    #[error("the count answer's row holds more than one cell")]
    SeveralCells,
    /// The cell is not a whole number zero or above.
    #[error("the count answer's cell is not a whole number zero or above")]
    NotACount,
}

impl CountShape {
    /// Why the count is unavailable, in the reader's words.
    #[must_use]
    pub fn reason(self) -> String {
        tr(match self {
            Self::NoRow => Msg::CountNoRow,
            Self::SeveralRows => Msg::CountSeveralRows,
            Self::NoCell => Msg::CountNoCell,
            Self::SeveralCells => Msg::CountSeveralCells,
            Self::NotACount => Msg::CountNotACount,
        })
    }
}

/// A count the CDR answered: the number, or why its answer held none.
pub type Count = Result<u64, CountShape>;

/// Reads the one cell of a count's result set.
///
/// # Errors
/// The [`CountShape`] the result set has when it is not one row of one
/// whole-number cell.
pub(crate) fn read_count(page: &crate::bff::ehrs::ResultPage) -> Count {
    let row = match page.rows.as_slice() {
        [] => return Err(CountShape::NoRow),
        [row] => row,
        _ => return Err(CountShape::SeveralRows),
    };
    match row.as_slice() {
        [] => Err(CountShape::NoCell),
        [cell] => cell.as_u64().ok_or(CountShape::NotACount),
        _ => Err(CountShape::SeveralCells),
    }
}

/// Run a `SELECT COUNT(*)`-shaped AQL with `parameters` and read its single
/// cell ([`read_count`]).
///
/// # Errors
/// CDR errors normalized via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the result set is not JSON. A result set of
/// another shape is the `Ok` value's [`CountShape`].
pub(crate) async fn run_count_aql(
    state: &crate::state::AppState,
    session: &crate::session::ViewerSession,
    aql: &str,
    parameters: serde_json::Value,
) -> Result<Count, ViewerError> {
    let url = state.cdr.rest_v1("query/aql");
    let body = serde_json::json!({ "q": aql, "query_parameters": parameters }).to_string();
    let response = state
        .cdr
        .post(
            &session.credential,
            &url,
            "application/json",
            "application/json",
            &[],
            body,
        )
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    let page = crate::bff::ehrs::parse_result_set(&response.body, 0)?;
    Ok(read_count(&page))
}

/// The dashboard's three headline counts: total EHRs, total compositions, and
/// the number of stored operational templates.
///
/// Runs the two fixed count AQLs against `POST query/aql` and reads the
/// template count from
/// [`list_templates`](crate::bff::templates::list_templates). A count whose
/// answer has another shape is its [`CountShape`], so its tile draws as
/// unavailable.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR errors
/// normalized via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] on a result set that is not JSON.
pub async fn dashboard_counts() -> Result<(Count, Count, u32), ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let none = || serde_json::json!({});
    let ehrs = run_count_aql(&state, &session, EHR_COUNT_AQL, none()).await?;
    let compositions = run_count_aql(&state, &session, COMPOSITION_COUNT_AQL, none()).await?;
    let templates =
        u32::try_from(crate::bff::templates::list_templates().await?.len()).unwrap_or(u32::MAX);
    Ok((ehrs, compositions, templates))
}

/// One dashboard namespace tile: the derived group's heading, the summed match
/// count of its member stored queries, and how many members that sum covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceTile {
    /// The namespace, or the label for the bucket of names that carry none
    /// ([`group_label`](crate::query_namespace::group_label)).
    pub label: String,
    /// The summed match count of the member queries, or `None` when a member
    /// failed to run — the tile then reads as an error instead of showing a
    /// silently short count.
    pub matches: Option<i64>,
    /// How many member stored queries the group holds.
    pub members: u32,
}

/// The dashboard's per-namespace match tiles: the CDR's stored queries grouped
/// by the namespace of their qualified name
/// ([`group_by_namespace`](crate::query_namespace::group_by_namespace)), each
/// group's members run through
/// [`run_stored_count`](crate::queries_api::run_stored_count) and summed.
///
/// One round trip for the whole tile row (and no per-tile resource created
/// inside a `Suspend`). A member query the CDR refuses degrades only its
/// own tile.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR errors from
/// the stored-query LISTING normalized (a failure there means no grouping at
/// all); a failing member RUN is confined to its own tile's `matches: None`.
pub async fn namespace_tiles() -> Result<Vec<NamespaceTile>, ViewerError> {
    use futures::stream::StreamExt;

    crate::session::require_session()?;
    let rows = crate::queries_api::list_stored_queries().await?;
    let groups = crate::query_namespace::group_by_namespace(&rows);

    // The grouping is DERIVED — there is no stored member list to narrow by —
    // so every member runs, and ONE bounded stream drives every count across
    // ALL groups. The identifiers are collected into owned, group-indexed
    // triples BEFORE the stream: an async block borrowing out of `groups` is
    // not general enough over lifetimes for the page's future.
    let members: Vec<(usize, String, String)> = groups
        .iter()
        .enumerate()
        .flat_map(|(index, group)| {
            group
                .members
                .iter()
                .map(move |member| (index, member.name.clone(), member.version.clone()))
        })
        .collect();
    let counts = futures::stream::iter(members.into_iter().map(
        |(index, name, version)| async move {
            let counted = crate::queries_api::run_stored_count(name.clone(), version.clone())
                .await
                .ok();
            (index, name, version, counted)
        },
    ))
    .buffered(crate::cdr::FANOUT_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;

    let mut matches: Vec<Option<i64>> = vec![Some(0); groups.len()];
    for (index, name, version, counted) in counts {
        let Some(slot) = matches.get_mut(index) else {
            continue;
        };
        let Some(count) = counted else {
            // Identifiers only, never the diagnostic: a CDR error body can
            // quote the query text, and query text can carry clinical
            // values — logs name shapes, not payloads. Naming the query is
            // enough for an operator to run it and read the CDR's answer.
            tracing::warn!(
                query = %name,
                version = %version,
                "stored query failed to run for its dashboard namespace tile"
            );
            *slot = None;
            continue;
        };
        *slot = slot.map(|total| total.saturating_add(count));
    }

    Ok(groups
        .iter()
        .zip(matches)
        .map(|(group, matches)| NamespaceTile {
            label: crate::query_namespace::group_label(group.namespace.as_deref()).to_owned(),
            matches,
            members: u32::try_from(group.members.len()).unwrap_or(u32::MAX),
        })
        .collect())
}

/// The recent commit-activity trend, one [`ActivityPoint`] per calendar day
/// ascending.
///
/// Pulls the most recent composition commit times (fetch `TREND_FETCH`) and
/// buckets them per day BFF-side with the shared
/// [`bucket_by_day`](crate::activity::bucket_by_day).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR errors
/// normalized; [`ViewerError::Internal`] on an unparseable result set.
pub async fn commit_trend() -> Result<Vec<ActivityPoint>, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let url = state.cdr.rest_v1("query/aql");
    let body = serde_json::json!({
        "q": TREND_AQL,
        "query_parameters": {},
        "fetch": TREND_FETCH,
    })
    .to_string();
    let response = state
        .cdr
        .post(
            &session.credential,
            &url,
            "application/json",
            "application/json",
            &[],
            body,
        )
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    let page = crate::bff::ehrs::parse_result_set(&response.body, 0)?;
    let times: Vec<String> = page
        .rows
        .iter()
        .filter_map(|row| row.first())
        .map(crate::bff::ehrs::cell_text)
        .collect();
    Ok(crate::activity::bucket_by_day(&times))
}

#[cfg(test)]
mod tests {
    use super::{COMPOSITION_COUNT_AQL, CountShape, EHR_COUNT_AQL, TREND_AQL, read_count};
    use crate::bff::ehrs::ResultPage;

    #[test]
    fn dashboard_aql_consts_parse() {
        for aql in [EHR_COUNT_AQL, COMPOSITION_COUNT_AQL, TREND_AQL] {
            openehr_query::parser::parse_str(aql).expect("dashboard AQL const must parse");
        }
    }

    fn page(rows: &serde_json::Value) -> ResultPage {
        let body = serde_json::json!({ "columns": [{ "name": "count" }], "rows": rows });
        crate::bff::ehrs::parse_result_set(&body.to_string(), 0).expect("a JSON result set")
    }

    #[test]
    fn a_real_zero_is_a_count_of_zero() {
        assert_eq!(read_count(&page(&serde_json::json!([[0]]))), Ok(0));
        assert_eq!(read_count(&page(&serde_json::json!([[42]]))), Ok(42));
    }

    #[test]
    fn a_result_set_without_a_row_is_no_count() {
        assert_eq!(
            read_count(&page(&serde_json::json!([]))),
            Err(CountShape::NoRow)
        );
        let absent =
            crate::bff::ehrs::parse_result_set(r#"{"columns":[]}"#, 0).expect("a JSON result set");
        assert_eq!(read_count(&absent), Err(CountShape::NoRow));
        assert_eq!(
            read_count(&page(&serde_json::json!([[1], [2]]))),
            Err(CountShape::SeveralRows)
        );
    }

    #[test]
    fn a_row_without_one_cell_is_no_count() {
        assert_eq!(
            read_count(&page(&serde_json::json!([[]]))),
            Err(CountShape::NoCell)
        );
        assert_eq!(
            read_count(&page(&serde_json::json!([[1, 2]]))),
            Err(CountShape::SeveralCells)
        );
    }

    #[test]
    fn a_cell_that_is_no_whole_number_is_no_count() {
        for cell in [
            serde_json::json!("0"),
            serde_json::json!(null),
            serde_json::json!(1.5),
            serde_json::json!(-1),
            serde_json::json!({ "value": 0 }),
        ] {
            assert_eq!(
                read_count(&page(&serde_json::json!([[cell]]))),
                Err(CountShape::NotACount),
                "{cell}"
            );
        }
    }

    #[test]
    fn every_shape_names_its_reason_in_plain_words() {
        for shape in [
            CountShape::NoRow,
            CountShape::SeveralRows,
            CountShape::NoCell,
            CountShape::SeveralCells,
            CountShape::NotACount,
        ] {
            let reason = shape.reason();
            assert!(reason.contains(' '), "{shape:?}: {reason}");
        }
    }
}
