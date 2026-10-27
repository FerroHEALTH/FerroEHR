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
//! Every co-located `#[server]` fn guards with
//! [`require_session`](crate::session::require_session) first, and the CDR
//! credential never reaches client-visible state. Each tile resolves its
//! `Result` inside its own suspense rather than through an `<ErrorBoundary>`,
//! whose SSR'd fallback mismatches at hydration in leptos 0.8, so one failing
//! section never blanks the dashboard.

#![allow(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694); the carriers here are ssr-only, so #[expect] would be unfulfilled on the \
              hydrate target"
)]

#[cfg(not(target_arch = "wasm32"))]
use leptos::prelude::*;
use leptos::server;
use serde::{Deserialize, Serialize};

use crate::activity::ActivityPoint;
use crate::error::ViewerError;

/// Total EHRs — a fixed count AQL. Validated by the `dashboard_aql_consts_parse` test.
#[cfg(not(target_arch = "wasm32"))]
const EHR_COUNT_AQL: &str = "SELECT COUNT(*) FROM EHR e";
/// Total compositions — a fixed count AQL.
#[cfg(not(target_arch = "wasm32"))]
const COMPOSITION_COUNT_AQL: &str = "SELECT COUNT(*) FROM EHR e CONTAINS COMPOSITION c";
/// Recent composition commit times, newest first, for the day-bucketed trend.
#[cfg(not(target_arch = "wasm32"))]
const TREND_AQL: &str = "SELECT v/commit_audit/time_committed/value FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c ORDER BY v/commit_audit/time_committed/value DESC";
/// Rows pulled for the commit-activity trend before day-bucketing.
#[cfg(not(target_arch = "wasm32"))]
const TREND_FETCH: u32 = 500;

/// Run a `SELECT COUNT(*)`-shaped AQL and read its single numeric cell (0 when
/// the result set is not the expected 1×1 numeric shape).
#[cfg(not(target_arch = "wasm32"))]
async fn run_count_aql(
    state: &crate::state::AppState,
    session: &crate::session::ViewerSession,
    aql: &str,
) -> Result<i64, ViewerError> {
    let url = state.cdr.rest_v1("query/aql");
    let body = serde_json::json!({ "q": aql, "query_parameters": {} }).to_string();
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
    Ok(page
        .rows
        .first()
        .and_then(|row| row.first())
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0))
}

/// The dashboard's three headline counts: total EHRs, total compositions, and
/// the number of stored operational templates.
///
/// Runs the two fixed count AQLs against `POST query/aql` and reads the
/// template count from
/// [`list_templates`](crate::bff::templates::list_templates).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR errors
/// normalized via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] on an unparseable result set.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn dashboard_counts() -> Result<(i64, i64, u32), ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let ehrs = run_count_aql(&state, &session, EHR_COUNT_AQL).await?;
    let compositions = run_count_aql(&state, &session, COMPOSITION_COUNT_AQL).await?;
    let templates =
        u32::try_from(crate::bff::templates::list_templates().await?.len()).unwrap_or(u32::MAX);
    Ok((ehrs, compositions, templates))
}

/// One dashboard namespace tile: the derived group's heading, the summed match
/// count of its member stored queries, and how many members that sum covers.
///
/// Fixed-size ints only, so it is WASM-safe over the server-fn boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn namespace_tiles() -> Result<Vec<NamespaceTile>, ViewerError> {
    use futures::stream::StreamExt;

    crate::session::require_session().await?;
    let rows = crate::queries_api::list_stored_queries().await?;
    let groups = crate::query_namespace::group_by_namespace(&rows);

    // The grouping is DERIVED — there is no stored member list to narrow by —
    // so every member runs, and ONE bounded stream drives every count across
    // ALL groups. The identifiers are collected into owned, group-indexed
    // triples BEFORE the stream: an async block borrowing out of `groups` is
    // not general enough over lifetimes for the `#[server]` boundary's future.
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
/// ascending. Pulls the most recent composition commit times (fetch
/// `TREND_FETCH`) and buckets them per day BFF-side with the shared
/// [`bucket_by_day`](crate::activity::bucket_by_day).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR errors
/// normalized; [`ViewerError::Internal`] on an unparseable result set.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn commit_trend() -> Result<Vec<ActivityPoint>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{COMPOSITION_COUNT_AQL, EHR_COUNT_AQL, TREND_AQL};

    #[test]
    fn dashboard_aql_consts_parse() {
        for aql in [EHR_COUNT_AQL, COMPOSITION_COUNT_AQL, TREND_AQL] {
            openehr_query::parser::parse_str(aql).expect("dashboard AQL const must parse");
        }
    }
}
