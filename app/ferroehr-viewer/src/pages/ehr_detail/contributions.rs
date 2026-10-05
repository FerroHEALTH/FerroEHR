// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR-detail Contributions tab: the per-EHR activity timeline, the paged
//! contribution list, and the by-uid CONTRIBUTION lookup box.

#![allow(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694); the carriers here are ssr-only, so #[expect] would be unfulfilled on the \
              hydrate target"
)]

use leptos::prelude::*;
use leptos::server;
use serde::{Deserialize, Serialize};
#[cfg(feature = "ssr")]
use serde_json::Value;

use crate::activity::ActivityPoint;
use crate::components::activity_chart::activity_chart;
use crate::components::data_table::{CELL, CELL_MONO, ROW, table_shell, table_skeleton};
use crate::components::empty_state::EmptyState;
use crate::components::field::{BTN_SECONDARY, INPUT, LABEL};
use crate::components::format_view::DocumentPane;
use crate::components::surface::{CARD_PAD, CARD_TITLE};
use crate::error::ViewerError;

/// Rows fetched per page of the contribution list (fixed, per the tab's
/// prev/next paging).
const CONTRIBUTION_FETCH: u32 = 20;

/// Contributions read for the activity timeline before day-bucketing. The
/// timeline answers a different question than the list ("when was this EHR
/// written to", not "which contributions are on this page"), so it reads its own
/// window of the SAME endpoint — one reader per claim, never a second endpoint
/// for the same fact.
#[cfg(feature = "ssr")]
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
/// prev/next). Carries only fixed-size ints so it is WASM-safe over the
/// server-fn boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributionPage {
    /// The contribution rows on this page.
    pub rows: Vec<ContributionRow>,
    /// The total number of contributions in the EHR.
    pub total: u32,
}

/// List an EHR's contributions, one page at `offset`
/// (`GET /ehr/{ehr_id}/contribution?offset&fetch`). `fetch` is fixed at
/// `CONTRIBUTION_FETCH`.
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
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn list_contributions(
    /// The EHR whose contributions to list.
    ehr_id: String,
    /// First row of the page to return.
    offset: u32,
) -> Result<ContributionPage, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1(&format!(
        "ehr/{}/contribution?offset={offset}&fetch={CONTRIBUTION_FETCH}",
        urlencoding::encode(&ehr_id),
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    parse_contributions(&response.body)
}

#[cfg(feature = "ssr")]
/// Parse the contribution-list body (`{ "rows": [...], "total": N }`) into a
/// [`ContributionPage`]. Defensive throughout — a missing field reads as empty
/// / zero rather than failing.
///
/// # Errors
/// [`ViewerError::Internal`] when the body is not valid JSON.
fn parse_contributions(body: &str) -> Result<ContributionPage, ViewerError> {
    let doc: Value = serde_json::from_str(body)
        .map_err(|e| ViewerError::Internal(format!("contributions JSON: {e}")))?;
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

#[cfg(feature = "ssr")]
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

#[cfg(feature = "ssr")]
/// Read a top-level string field, or an empty string when absent / not a string.
fn contribution_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// The EHR's contribution activity, one [`ActivityPoint`] per calendar day
/// ascending (`GET /ehr/{ehr_id}/contribution?offset=0&fetch` over
/// `ACTIVITY_FETCH`, bucketed BFF-side by
/// [`bucket_by_day`](crate::activity::bucket_by_day) on each contribution's
/// `AUDIT_DETAILS.time_committed`).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the page is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn contribution_activity(
    /// The EHR whose contribution instants feed the timeline.
    ehr_id: String,
) -> Result<Vec<ActivityPoint>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
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

/// Look up a single CONTRIBUTION by uid
/// (`GET /ehr/{ehr_id}/contribution/{contribution_uid}`) — the by-uid lookup
/// box the Contributions tab keeps below its list (see
/// `contributions_section`). Returns the raw canonical JSON.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx CDR answer (a `404` for an unknown uid
/// included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_contribution(
    /// The EHR holding the contribution.
    ehr_id: String,
    /// The CONTRIBUTION uid to read.
    contribution_uid: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
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

/// The per-EHR activity timeline: contributions per day, drawn with the shared
/// [`activity_chart`] kit (the same chart the dashboard's commit trend uses).
/// Its resource is created in setup and gated on the tab being active, so it
/// fetches only when shown (never fetch-in-effect).
fn activity_section(ehr_id: Signal<String>, selected: Memo<String>) -> AnyView {
    let resource = Resource::new(
        move || (selected.get() == "contributions").then(|| ehr_id.get()),
        |active| async move {
            match active {
                Some(id) => contribution_activity(id).await.map(Some),
                None => Ok(None),
            }
        },
    );
    let body = view! {
        <Suspense fallback=|| {
            view! {
                <thaw::Skeleton>
                    <thaw::SkeletonItem class="h-40" />
                </thaw::Skeleton>
            }
        }>
            {move || Suspend::new(async move {
                match resource.await {
                    Ok(Some(points)) => {
                        activity_chart(
                            &points,
                            "contributions",
                            "No contribution activity yet",
                            "Every write to this EHR is a contribution; the timeline fills as they are committed.",
                        )
                    }
                    Ok(None) => ().into_any(),
                    Err(e) => crate::components::notice::inline_error(&e),
                }
            })}
        </Suspense>
    }
    .into_any();
    view! {
        <section class=CARD_PAD>
            <h2 class=CARD_TITLE>"Contribution activity"</h2>
            {body}
        </section>
    }
    .into_any()
}

/// Contributions tab: the activity timeline, a paged list of the EHR's
/// contributions (`list_contributions` → `GET
/// /ehr/{ehr_id}/contribution?offset&fetch`) under a `<Transition>`, plus the
/// by-uid lookup box kept below it. An older CDR that lacks the list route (a
/// `404`/`405`) renders inline via the normal error path — the lookup box still
/// works.
pub(super) fn contributions_section(ehr_id: Signal<String>, selected: Memo<String>) -> AnyView {
    // Paging state is a local signal — the tab itself is already URL-state.
    let offset = RwSignal::new(0_u32);
    let list = Resource::new(
        move || (selected.get() == "contributions").then(|| (ehr_id.get(), offset.get())),
        |active| async move {
            match active {
                Some((id, off)) => list_contributions(id, off).await.map(Some),
                None => Ok(None),
            }
        },
    );
    let table = view! {
        <Transition fallback=table_skeleton>
            {move || Suspend::new(async move {
                match list.await {
                    Ok(Some(page)) => contributions_table(&page, offset),
                    Ok(None) => ().into_any(),
                    Err(e) => crate::components::notice::inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any();
    let activity = activity_section(ehr_id, selected);
    let lookup = contribution_lookup(ehr_id, selected);
    view! { <div class="flex flex-col gap-4">{activity} {table} {lookup}</div> }.into_any()
}

/// Render one page of contributions as the shared [`table_shell`] plus
/// prev/next paging (local-signal driven). An empty page is an
/// [`EmptyState`], not bare muted text.
fn contributions_table(page: &ContributionPage, offset: RwSignal<u32>) -> AnyView {
    if page.rows.is_empty() {
        return view! {
            <EmptyState
                icon=icondata_lu::LuInbox
                message="No contributions"
                hint="This EHR has no contributions on this page."
            />
        }
        .into_any();
    }
    let rows = page.rows.clone();
    let body = view! {
        <For each=move || rows.clone() key=|row| row.uid.clone() let:row>
            {contribution_row_view(&row)}
        </For>
    }
    .into_any();
    // `offset` is the offset that produced this page; the Suspend re-runs on any
    // change (the list resource depends on it), so an untracked read is exact.
    let current = offset.get_untracked();
    let total = page.total;
    let shown = u32::try_from(page.rows.len()).unwrap_or(u32::MAX);
    let prev = (current > 0).then(|| {
        view! {
            <button
                type="button"
                class=BTN_SECONDARY
                on:click=move |_| offset.set(current.saturating_sub(CONTRIBUTION_FETCH))
            >
                "← Previous"
            </button>
        }
        .into_any()
    });
    let next = (current.saturating_add(shown) < total).then(|| {
        view! {
            <button
                type="button"
                class=BTN_SECONDARY
                on:click=move |_| offset.set(current.saturating_add(CONTRIBUTION_FETCH))
            >
                "Next →"
            </button>
        }
        .into_any()
    });
    view! {
        {table_shell(&["Contribution", "Committed", "Committer", "Change type"], body)}
        <p id="contribution-total" class="mt-2 text-xs text-ink-muted">
            {total}
            " contribution(s) total"
        </p>
        <div class="mt-3 flex gap-2">{prev} {next}</div>
    }
    .into_any()
}

/// One contribution row: the uid (mono) plus its commit metadata. The change
/// type shows the CDR-resolved rubric (falling back to the raw group code for
/// an older CDR that sends none), with the code kept as the hover title.
fn contribution_row_view(row: &ContributionRow) -> AnyView {
    let change_type = if row.change_type_rubric.is_empty() {
        row.change_type.clone()
    } else {
        row.change_type_rubric.clone()
    };
    view! {
        <tr class=ROW>
            <td class=CELL_MONO>{row.uid.clone()}</td>
            <td class=CELL>{row.time_committed.clone()}</td>
            <td class=CELL>{row.committer.clone()}</td>
            <td class=CELL title=row.change_type.clone()>
                {change_type}
            </td>
        </tr>
    }
    .into_any()
}

/// The by-uid CONTRIBUTION lookup box kept below the list: submitting a uid
/// drives [`fetch_contribution`] under a `<Transition>` and renders the raw
/// canonical JSON.
fn contribution_lookup(ehr_id: Signal<String>, selected: Memo<String>) -> AnyView {
    let uid_input = RwSignal::new(String::new());
    let submitted = RwSignal::new(String::new());
    let on_click = move |_| submitted.set(uid_input.get().trim().to_owned());
    let resource = Resource::new(
        move || {
            let uid = submitted.get();
            (selected.get() == "contributions" && !uid.is_empty()).then(|| (ehr_id.get(), uid))
        },
        |active| async move {
            match active {
                Some((id, uid)) => fetch_contribution(id, uid).await.map(Some),
                None => Ok(None),
            }
        },
    );
    let lookup = view! {
        <section class=format!("{CARD_PAD} mb-4")>
            <div class="flex flex-wrap items-end gap-3">
                <div class="flex flex-col gap-1">
                    <label class=LABEL r#for="contribution-uid">
                        "Contribution uid"
                    </label>
                    <input
                        id="contribution-uid"
                        type="text"
                        class=INPUT
                        placeholder="contribution uid (UUID)"
                        prop:value=move || uid_input.get()
                        on:input:target=move |ev| uid_input.set(ev.target().value())
                    />
                </div>
                <button type="button" class=BTN_SECONDARY on:click=on_click>
                    "Look up"
                </button>
            </div>
        </section>
    }
    .into_any();
    let result = view! {
        <Transition fallback=table_skeleton>
            {move || Suspend::new(async move {
                match resource.await {
                    Ok(Some(body)) => contribution_body(&body),
                    Ok(None) => {
                        // Resolve inside the Transition: an SSR'd ErrorBoundary fallback
                        // mismatches at hydration in leptos 0.8.
                        view! {
                            <p class="text-sm text-ink-muted">
                                "Enter a contribution uid to look it up."
                            </p>
                        }
                            .into_any()
                    }
                    Err(e) => crate::components::notice::inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any();
    view! { <div>{lookup} {result}</div> }.into_any()
}

/// Render a fetched CONTRIBUTION as pretty JSON in the shared document pane.
fn contribution_body(body: &str) -> AnyView {
    let pretty =
        crate::components::format_view::pretty_body(body, crate::format::ReprFormat::CanonicalJson);
    let doc_sig = RwSignal::new(pretty);
    view! { <DocumentPane body=doc_sig /> }.into_any()
}

#[cfg(all(test, feature = "ssr"))]
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
        assert!(page.rows.is_empty());
        assert_eq!(page.total, 0);
    }

    #[test]
    fn parse_contributions_rejects_non_json() {
        assert!(parse_contributions("not json").is_err());
    }
}
