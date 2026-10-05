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

use leptos::prelude::*;
use leptos::{component, server};
use leptos_meta::Title;
use leptos_router::components::A;
use serde::{Deserialize, Serialize};

use crate::activity::ActivityPoint;
use crate::components::activity_chart::activity_chart;
use crate::components::empty_state::EmptyState;
use crate::components::field::BTN_SECONDARY;
use crate::components::page_header::PageHeader;
use crate::components::stat_card::StatCard;
use crate::components::surface::{CARD_PAD, CARD_TITLE};
use crate::error::ViewerError;

/// Total EHRs — a fixed count AQL. Validated by [`tests::dashboard_aql_consts_parse`].
#[cfg(feature = "ssr")]
const EHR_COUNT_AQL: &str = "SELECT COUNT(*) FROM EHR e";
/// Total compositions — a fixed count AQL.
#[cfg(feature = "ssr")]
const COMPOSITION_COUNT_AQL: &str = "SELECT COUNT(*) FROM EHR e CONTAINS COMPOSITION c";
/// Recent composition commit times, newest first, for the day-bucketed trend.
#[cfg(feature = "ssr")]
const TREND_AQL: &str = "SELECT c/context/start_time/value FROM EHR e CONTAINS COMPOSITION c ORDER BY c/context/start_time/value DESC";
/// Rows pulled for the commit-activity trend before day-bucketing.
#[cfg(feature = "ssr")]
const TREND_FETCH: u32 = 500;

/// Run a `SELECT COUNT(*)`-shaped AQL and read its single numeric cell (0 when
/// the result set is not the expected 1×1 numeric shape).
#[cfg(feature = "ssr")]
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
    let page = crate::pages::ehrs::parse_result_set(&response.body, 0)?;
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
/// [`list_templates`](crate::pages::templates::list_templates).
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
        u32::try_from(crate::pages::templates::list_templates().await?.len()).unwrap_or(u32::MAX);
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
    let page = crate::pages::ehrs::parse_result_set(&response.body, 0)?;
    let times: Vec<String> = page
        .rows
        .iter()
        .filter_map(|row| row.first())
        .map(crate::pages::ehrs::cell_text)
        .collect();
    Ok(crate::activity::bucket_by_day(&times))
}

/// The `/` dashboard: headline counts, per-namespace stored-query match tiles,
/// and a commit-activity trend — each an independently-failing section.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn DashboardPage() -> impl IntoView {
    let counts = counts_section();
    let stored = stored_queries_tile();
    let namespaces = namespaces_section();
    let trend = trend_section();

    view! {
        <Title text="Dashboard" />
        <div class="p-6">
            <PageHeader
                title="Dashboard"
                subtitle="Repository overview, query cohorts, and recent commit activity."
            />
            <div class="grid grid-cols-2 md:grid-cols-4 gap-3 mb-6">{counts} {stored}</div>
            <section class=format!("{CARD_PAD} mb-6")>
                <h2 class=CARD_TITLE>"Query namespaces"</h2>
                {namespaces}
            </section>
            <section class=CARD_PAD>
                <h2 class=CARD_TITLE>"Commit activity"</h2>
                {trend}
            </section>
        </div>
    }
}

/// One headline stat tile as a design-system [`StatCard`] — an icon, the
/// tabular-nums value, and a muted label; the whole tile navigates when a
/// `href` is given.
fn stat_tile(
    label: &'static str,
    value: String,
    icon: icondata_core::Icon,
    href: Option<String>,
) -> AnyView {
    match href {
        Some(href) => view! { <StatCard label=label value=value icon=icon href=href /> }.into_any(),
        None => view! { <StatCard label=label value=value icon=icon /> }.into_any(),
    }
}

/// A single stat-tile skeleton (a card-shaped placeholder).
fn tile_skeleton() -> AnyView {
    view! {
        <div class=CARD_PAD>
            <thaw::Skeleton>
                <thaw::SkeletonItem class="h-8 mb-2" />
                <thaw::SkeletonItem class="h-4 w-2/3" />
            </thaw::Skeleton>
        </div>
    }
    .into_any()
}

/// Three stat-tile skeletons for the counts section's loading state (keeps the
/// tile grid stable while the counts load).
fn tiles_skeleton() -> impl IntoView {
    view! { <>{tile_skeleton()} {tile_skeleton()} {tile_skeleton()}</> }
}

/// The EHR / composition / template count tiles, all from one
/// [`dashboard_counts`] round trip.
fn counts_section() -> AnyView {
    let resource = Resource::new(|| (), |()| async move { dashboard_counts().await });
    view! {
        <Suspense fallback=tiles_skeleton>
            {move || Suspend::new(async move {
                match resource.await {
                    Ok((ehrs, compositions, templates)) => {
                        // Resolve the Result INSIDE the suspense and render
                        // either branch as one erased view: hydrating an SSR'd
                        // ErrorBoundary fallback mismatches in leptos 0.8, and
                        // this keeps server/client structure identical while
                        // errors stay visible.
                        view! {
                            <>
                                {stat_tile(
                                    "EHRs",
                                    ehrs.to_string(),
                                    icondata_lu::LuDatabase,
                                    Some("/ehrs".to_owned()),
                                )}
                                {stat_tile(
                                    "Compositions",
                                    compositions.to_string(),
                                    icondata_lu::LuFileText,
                                    None,
                                )}
                                {stat_tile(
                                    "Templates",
                                    templates.to_string(),
                                    icondata_lu::LuFileCode2,
                                    Some("/templates".to_owned()),
                                )}
                            </>
                        }
                            .into_any()
                    }
                    Err(e) => crate::components::notice::inline_error(&e),
                }
            })}
        </Suspense>
    }
    .into_any()
}

/// The stored-query count tile (its own round trip so a Definition-API failure
/// doesn't take the count tiles down with it).
fn stored_queries_tile() -> AnyView {
    let resource = Resource::new(
        || (),
        |()| async move { crate::queries_api::list_stored_queries().await },
    );
    view! {
        <Suspense fallback=tile_skeleton>
            {move || Suspend::new(async move {
                match resource.await {
                    Ok(rows) => {
                        let count = u32::try_from(rows.len()).unwrap_or(u32::MAX);
                        stat_tile(
                            "Stored queries",
                            count.to_string(),
                            icondata_lu::LuSearchCode,
                            Some("/queries".to_owned()),
                        )
                    }
                    Err(e) => crate::components::notice::inline_error(&e),
                }
            })}
        </Suspense>
    }
    .into_any()
}

/// The per-namespace match-count tiles, each linking to the stored-queries
/// screen. ONE round trip builds the whole row ([`namespace_tiles`]), so no
/// resource is created inside the `Suspend`.
fn namespaces_section() -> AnyView {
    let resource = Resource::new(|| (), |()| async move { namespace_tiles().await });
    view! {
        <Transition fallback=tile_skeleton>
            {move || Suspend::new(async move {
                match resource.await {
                    Ok(tiles) => namespace_tiles_view(tiles),
                    Err(e) => crate::components::notice::inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// Render the namespace tiles, or the empty-state hint.
fn namespace_tiles_view(tiles: Vec<NamespaceTile>) -> AnyView {
    if tiles.is_empty() {
        return view! {
            <EmptyState
                icon=icondata_lu::LuSearchCode
                message="No stored queries yet"
                hint="Save a query as namespace::name — each namespace becomes a cohort tile here."
            >
                <A href="/queries" attr:class=BTN_SECONDARY>
                    "Go to stored queries"
                </A>
            </EmptyState>
        }
        .into_any();
    }
    let tiles = tiles.into_iter().map(namespace_tile).collect::<Vec<_>>();
    view! { <div class="grid grid-cols-2 md:grid-cols-4 gap-3">{tiles}</div> }.into_any()
}

/// One namespace tile: the namespace (or the unqualified-bucket label), the
/// summed match count of its stored queries, and a link to the stored-queries
/// screen. The whole tile is an `<A>` styled as a design-system card (block
/// content only, so the anchor stays valid HTML). A group whose member query
/// the CDR refused shows "error" instead of a short count.
/// `data-namespace-tile` is the stable E2E hook.
fn namespace_tile(tile: NamespaceTile) -> AnyView {
    let value = tile.matches.map_or_else(
        || view! { <span class="text-sm text-danger">"error"</span> }.into_any(),
        |total| view! { <span>{total.to_string()}</span> }.into_any(),
    );
    let hook = tile.label.clone();
    let members = tile.members;
    let summary = if members == 1 {
        "1 query".to_owned()
    } else {
        format!("{members} queries")
    };
    view! {
        <A
            href="/queries"
            attr:class=format!("block {CARD_PAD} transition-colors hover:border-accent")
        >
            <div class="font-mono font-medium truncate text-ink" data-namespace-tile=hook>
                {tile.label}
            </div>
            <div class="text-2xl font-semibold tabular-nums mt-1 text-ink">{value}</div>
            <div class="text-xs text-ink-muted mt-1">{summary}</div>
        </A>
    }
    .into_any()
}

/// The commit-activity trend section: commits per day through the shared
/// [`activity_chart`] kit (the same chart an EHR's contribution timeline draws).
fn trend_section() -> AnyView {
    let resource = Resource::new(|| (), |()| async move { commit_trend().await });
    view! {
        <Suspense fallback=|| {
            view! {
                <thaw::Skeleton>
                    <thaw::SkeletonItem class="h-40" />
                </thaw::Skeleton>
            }
        }>
            {move || Suspend::new(async move {
                match resource.await {
                    Ok(points) => {
                        activity_chart(
                            &points,
                            "commits",
                            "No commit activity yet",
                            "Commit a composition and the trend appears here from the next day onward.",
                        )
                    }
                    Err(e) => crate::components::notice::inline_error(&e),
                }
            })}
        </Suspense>
    }
    .into_any()
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::{COMPOSITION_COUNT_AQL, EHR_COUNT_AQL, TREND_AQL};

    #[test]
    fn dashboard_aql_consts_parse() {
        for aql in [EHR_COUNT_AQL, COMPOSITION_COUNT_AQL, TREND_AQL] {
            openehr_query::parser::parse_str(aql).expect("dashboard AQL const must parse");
        }
    }
}
