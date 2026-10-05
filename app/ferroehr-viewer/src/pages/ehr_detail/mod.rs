// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/ehrs/{ehr_id}` screen — EHR detail: status / status history /
//! directory / compositions / contributions / commit / tags tabs.
//!
//! Seven URL-driven tabs (`?tab=`) over one EHR. Each tab's data is a
//! `#[server]` fn co-located with the tab in its own submodule: [`status`],
//! which owns BOTH status tabs, then [`directory`], [`compositions`],
//! [`contributions`], [`commit`] (the atomic multi-change CONTRIBUTION staging
//! area), and the EHR-wide tag browser in [`crate::pages::ehr_tags`]. The
//! resources are created once and their sources are gated on the active tab, so
//! only the visible tab fetches. The tab bodies are always mounted and toggled
//! with `class:hidden`, keeping the server and client view structure identical.
//!
//! This module owns the [`EhrDetailPage`] shell, the EHR summary header and the
//! shared `tab_bar` strip. The header carries two reads, neither duplicated
//! elsewhere on the screen: the EHR resource itself (`GET /ehr/{ehr_id}`, so an
//! unknown id fails once at the top instead of once per tab) and — through
//! [`status_feed`] — the current `EHR_STATUS`, which is the ONE read of the
//! subject and the capability flags the Status tab also renders. That one is
//! therefore NOT tab-gated: the header shows on every tab.
//!
//! No openEHR spec governs the viewer — our own design; the wire it reads is
//! spec-bound (ITS-REST EHR + Query APIs). User input NEVER concatenates into
//! AQL — the compositions listing's statement is assembled from compile-time
//! fragments by [`composition_filter`] and every value travels as an AQL
//! `query_parameters` binding; path segments are percent-encoded server-side.
//! Every co-located `#[server]` fn guards with
//! [`require_session`](crate::session::require_session) first, and the CDR
//! credential never reaches client-visible state.

#![allow(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694); the carriers here are ssr-only, so #[expect] would be unfulfilled on the \
              hydrate target"
)]

pub mod commit;
pub mod composition_filter;
pub mod compositions;
pub mod contributions;
pub mod directory;
pub mod status;

use leptos::prelude::*;
use leptos::{component, server};
use leptos_meta::Title;
use serde::{Deserialize, Serialize};
#[cfg(feature = "ssr")]
use serde_json::Value;

use crate::components::field::BTN_DANGER;
use crate::components::page_header::{Crumb, PageHeader};
use crate::components::surface::CARD_PAD;
use crate::components::toast::{toast_error, toast_success};
use crate::error::ViewerError;
use crate::pages::ehr_detail::commit::commit_section;
use crate::pages::ehr_detail::compositions::compositions_section;
use crate::pages::ehr_detail::contributions::contributions_section;
use crate::pages::ehr_detail::directory::directory_section;
use crate::pages::ehr_detail::status::history::status_history_section;
use crate::pages::ehr_detail::status::{
    StatusFeed, capability_badge, status_feed, status_section, subject_label,
};

/// The EHR resource's own summary facts, flattened for the detail header.
///
/// All fields are plain strings (no `usize`), so the type is safe across the
/// server-fn boundary on the 32-bit WASM target.
///
/// The attributes are the RM `EHR` class's own: `system_id` ("the identifier of
/// the logical EHR management system in which this EHR was created"),
/// `time_created`, and the `ehr_status` reference — an `OBJECT_REF` whose `id`
/// addresses the CURRENT `EHR_STATUS` version and whose `type` names the
/// versioned container (invariant `Ehr_status_valid`:
/// `ehr_status.type.is_equal("VERSIONED_EHR_STATUS")`) — all from
/// `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.ehr.ehr.adoc`
/// §`EHR Class`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EhrSummary {
    /// `EHR.ehr_id.value` as the CDR reports it.
    pub ehr_id: String,
    /// `EHR.system_id.value` — recorded at creation, immutable thereafter.
    pub system_id: String,
    /// `EHR.time_created.value`.
    pub time_created: String,
    /// `EHR.ehr_status.id.value` — the current `EHR_STATUS` version.
    pub ehr_status_uid: String,
    /// `EHR.ehr_status.type` — the referenced versioned container's RM type.
    pub ehr_status_type: String,
}

/// Read the EHR resource itself (`GET /ehr/{ehr_id}`) for the detail header.
///
/// This is the viewer's ONE reader of the EHR resource's own FACTS: an
/// unknown or mistyped id surfaces here, once, at the top of the screen — the
/// tabs read their own sub-resources. (The status tag panel reads the same
/// endpoint server-side for the one identifier that addresses the status's tag
/// collection — a second window of one endpoint, not a second reader of a
/// claim.)
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx CDR answer (the `404` for an unknown
/// `ehr_id` included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the body is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_ehr_summary(
    /// The EHR whose summary facts to read.
    ehr_id: String,
) -> Result<EhrSummary, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state
        .cdr
        .rest_v1(&format!("ehr/{}", urlencoding::encode(&ehr_id)));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    parse_ehr_summary(&response.body)
}

#[cfg(feature = "ssr")]
/// Flatten an `EHR` resource body into an [`EhrSummary`]. Defensive throughout —
/// a missing attribute reads as empty rather than failing, so a header still
/// renders what the CDR did send.
///
/// # Errors
/// [`ViewerError::Internal`] when the body is not valid JSON.
fn parse_ehr_summary(body: &str) -> Result<EhrSummary, ViewerError> {
    let doc: Value =
        serde_json::from_str(body).map_err(|e| ViewerError::Internal(format!("EHR JSON: {e}")))?;
    Ok(EhrSummary {
        ehr_id: json_path(&doc, &["ehr_id", "value"]),
        system_id: json_path(&doc, &["system_id", "value"]),
        time_created: json_path(&doc, &["time_created", "value"]),
        ehr_status_uid: json_path(&doc, &["ehr_status", "id", "value"]),
        ehr_status_type: json_path(&doc, &["ehr_status", "type"]),
    })
}

#[cfg(feature = "ssr")]
/// Follow a chain of object keys to a string leaf, or an empty string when any
/// hop is absent or not a string.
fn json_path(value: &Value, path: &[&str]) -> String {
    let mut current = value;
    for key in path {
        match current.get(*key) {
            Some(next) => current = next,
            None => return String::new(),
        }
    }
    current.as_str().unwrap_or_default().to_owned()
}

/// The EHR summary header: WHO this EHR is about and what may be done with it,
/// above the EHR resource's own facts, above the tabs.
///
/// Two reads, two claims, one card. The identity strip is the screen's SHARED
/// current-`EHR_STATUS` read ([`StatusFeed`]) — the subject and the two
/// capability flags live on that document and nowhere else, and the Status tab
/// shows them from this very same resource rather than fetching them again. The
/// fact grid is the EHR resource itself. Both resolve their `Result` INSIDE
/// their own boundary (an SSR'd `ErrorBoundary` fallback mismatches at
/// hydration in leptos 0.8), and a `404` renders as the explicit "no such EHR"
/// state: this is where a mistyped id is reported, once.
fn summary_section(ehr_id: Signal<String>, status: StatusFeed) -> AnyView {
    let resource = Resource::new(
        move || ehr_id.get(),
        |id| async move { fetch_ehr_summary(id).await },
    );
    let identity = identity_strip(status);
    view! {
        <div class="mb-4">
            <section class=CARD_PAD id="ehr-summary">
                {identity}
                <Suspense fallback=|| {
                    view! {
                        <thaw::Skeleton>
                            <thaw::SkeletonItem class="h-16" />
                        </thaw::Skeleton>
                    }
                }>
                    {move || Suspend::new(async move {
                        match resource.await {
                            Ok(summary) => summary_facts(&summary),
                            Err(e) if e.status_code() == Some(http::StatusCode::NOT_FOUND) => {
                                view! {
                                    <div
                                        role="alert"
                                        id="ehr-not-found"
                                        class="rounded-card border border-danger/40 bg-danger-subtle px-3 py-2 text-sm text-danger"
                                    >
                                        "The CDR holds no EHR with this id — check the id, or create it on the EHRs screen."
                                    </div>
                                }
                                    .into_any()
                            }
                            Err(e) => crate::components::notice::inline_error(&e),
                        }
                    })}
                </Suspense>
            </section>
        </div>
    }
    .into_any()
}

/// The header's identity line: the EHR's subject and its two capability
/// badges, from the shared current-`EHR_STATUS` read.
///
/// A `<Transition>`, because the same resource reloads after a status save and
/// the header must not flash a skeleton while it does. A failed read renders
/// NOTHING here: an unknown `ehr_id` fails both reads, and the fact grid below
/// already reports it once — the screen never renders an error as nothing, it
/// renders it in one place.
fn identity_strip(status: StatusFeed) -> AnyView {
    let resource = status.resource;
    view! {
        <Transition fallback=|| {
            view! {
                <thaw::Skeleton>
                    <thaw::SkeletonItem class="h-6 mb-3" />
                </thaw::Skeleton>
            }
        }>
            {move || Suspend::new(async move {
                match resource.await {
                    Ok(state) => {
                        let subject = subject_label(&state);
                        view! {
                            <div
                                id="ehr-identity"
                                class="mb-3 flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-edge pb-3"
                            >
                                <span class="text-sm">
                                    <span class="font-medium text-ink-muted mr-1">"subject:"</span>
                                    <span
                                        class="font-mono break-all text-ink"
                                        data-ehr-fact="subject"
                                    >
                                        {subject}
                                    </span>
                                </span>
                                {capability_badge("header", "queryable", state.is_queryable)}
                                {capability_badge("header", "modifiable", state.is_modifiable)}
                            </div>
                        }
                            .into_any()
                    }
                    Err(_) => ().into_any(),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// Render the EHR resource's own facts as the header's grid.
fn summary_facts(summary: &EhrSummary) -> AnyView {
    let status_label = if summary.ehr_status_type.is_empty() {
        "ehr_status".to_owned()
    } else {
        format!("ehr_status ({})", summary.ehr_status_type)
    };
    let facts = vec![
        summary_fact("ehr_id".to_owned(), summary.ehr_id.clone()),
        summary_fact("system_id".to_owned(), summary.system_id.clone()),
        summary_fact("time_created".to_owned(), summary.time_created.clone()),
        summary_fact(status_label, summary.ehr_status_uid.clone()),
    ];
    view! {
        <div class="grid grid-cols-1 sm:grid-cols-2 items-start gap-x-4 gap-y-2 text-sm">
            {facts}
        </div>
    }
    .into_any()
}

/// One label/value line of the summary card (an absent value shows an em dash).
fn summary_fact(label: String, value: String) -> AnyView {
    let shown = if value.is_empty() {
        "—".to_owned()
    } else {
        value
    };
    view! {
        <div>
            <span class="font-medium text-ink-muted mr-1">{label}":"</span>
            <span class="font-mono break-all text-ink">{shown}</span>
        </div>
    }
    .into_any()
}

/// The `/ehrs/{ehr_id}` screen: the tab bar plus five always-mounted,
/// visibility-toggled tab bodies.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn EhrDetailPage() -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();
    let ehr_id = Signal::derive(move || params.with(|p| p.get("ehr_id").unwrap_or_default()));
    let query = leptos_router::hooks::use_query_map();
    let offset = Signal::derive(move || {
        query
            .with(|q| q.get("offset"))
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0)
    });
    // Tab state lives in the URL (`?tab=`): shareable and refresh-safe. A Memo
    // (not an Effect) derives the active tab, defaulting to "status".
    let selected: Memo<String> = Memo::new(move |_| {
        query
            .with(|q| q.get("tab"))
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "status".to_owned())
    });

    // The screen's ONE current-`EHR_STATUS` read, created before every consumer
    // so its resource id is allocated in the same place on both sides of
    // hydration: the header's identity strip and the Status tab both take this
    // handle.
    let status_feed = status_feed(ehr_id);

    let status = status_section(status_feed, ehr_id, selected);
    let status_history = status_history_section(ehr_id, selected);
    let directory = directory_section(ehr_id, selected);
    let compositions = compositions_section(ehr_id, offset, selected);
    let contributions = contributions_section(ehr_id, selected);
    let commit = commit_section(ehr_id, selected);
    let tag_browser = crate::pages::ehr_tags::ehr_tags_section(ehr_id, selected);

    let heading = Signal::derive(move || {
        let id = ehr_id.get();
        let short: String = id.chars().take(8).collect();
        format!("EHR {short}…")
    });

    let summary = summary_section(ehr_id, status_feed);
    let tabs = tab_bar(ehr_id, selected);
    let delete_action = delete_section(ehr_id);

    view! {
        <Title text="EHR detail" />
        <div class="p-6">
            <PageHeader
                title=Signal::derive(move || heading.get())
                crumbs=vec![Crumb::new("EHRs", "/ehrs")]
                mono=true
            />
            {delete_action}
            {summary}
            {tabs}
            <div class="mt-4">
                <div class:hidden=move || selected.get() != "status">{status}</div>
                <div class:hidden=move || {
                    selected.get() != "status-history"
                }>{status_history}</div>
                <div class:hidden=move || selected.get() != "directory">{directory}</div>
                <div class:hidden=move || selected.get() != "compositions">{compositions}</div>
                <div class:hidden=move || {
                    selected.get() != "contributions"
                }>{contributions}</div>
                <div class:hidden=move || selected.get() != "commit">{commit}</div>
                <div class:hidden=move || selected.get() != "tags">{tag_browser}</div>
            </div>
        </div>
    }
}

/// The admin **Delete EHR** affordance above the tab bar.
///
/// Probe-gated (`crate::admin::when_admin_usable` — nothing renders unless the
/// CDR advertises its Admin API as mounted). The click opens the
/// shared confirmation modal
/// ([`ConfirmDialog`](crate::components::confirm_dialog::ConfirmDialog)), whose
/// copy spells out the full EHR id and what the delete destroys, because this is
/// the CDR's PHYSICAL delete (SM `I_ADMIN_SERVICE.physical_ehr_delete` —
/// `docs/specs/openehr/SM/docs/UML/classes/i_admin_service.adoc`), not the
/// openEHR logical delete: the versions, contributions and audit trail go with
/// it. On success the viewer returns to `/ehrs` (this screen's subject is
/// gone) with a toast naming the deleted id.
fn delete_section(ehr_id: Signal<String>) -> AnyView {
    let toaster = thaw::ToasterInjection::expect_context();
    let gate = crate::admin::admin_gate();
    let delete: Action<String, (String, Result<(), ViewerError>)> = Action::new(|id: &String| {
        let id = id.clone();
        async move {
            let outcome = crate::admin::admin_delete_ehr(id.clone()).await;
            (id, outcome)
        }
    });
    // Whether the confirmation modal is open (one deletable object per screen,
    // so a bool IS the "which object" state).
    let confirming = RwSignal::new(false);

    let navigate = leptos_router::hooks::use_navigate();
    Effect::new(move |_| match delete.value().get() {
        Some((id, Ok(()))) => {
            toast_success(
                toaster,
                "EHR deleted",
                &format!("EHR {id} and all of its data were removed from the CDR."),
            );
            navigate("/ehrs", leptos_router::NavigateOptions::default());
        }
        Some((id, Err(error))) => toast_error(
            toaster,
            "Delete failed",
            &crate::admin::delete_failure_copy(&format!("EHR {id}"), &error),
        ),
        None => {}
    });

    let message = Signal::derive(move || {
        format!(
            "Permanently delete EHR {} — every composition, contribution and audit record in \
             it? This is the CDR's physical delete: nothing stays readable afterwards, and it \
             cannot be undone.",
            ehr_id.get()
        )
    });

    crate::admin::when_admin_usable(gate, move || {
        view! {
            <div class="mb-4 flex flex-wrap items-center justify-end gap-3">
                <button
                    id="ehr-delete"
                    type="button"
                    class=BTN_DANGER
                    disabled=Signal::derive(move || delete.pending().get())
                    on:click=move |_| confirming.set(true)
                >
                    <leptos_icons::Icon icon=icondata_lu::LuTrash width="14" height="14" />
                    "Delete EHR"
                </button>
                <crate::components::confirm_dialog::ConfirmDialog
                    open=confirming
                    title="Delete EHR"
                    message=message
                    confirm_label="Delete EHR"
                    confirm_id="ehr-delete-confirm"
                    on_cancel=Callback::new(move |()| confirming.set(false))
                    on_confirm=Callback::new(move |()| {
                        delete.dispatch(ehr_id.get_untracked());
                        confirming.set(false);
                    })
                />
            </div>
        }
        .into_any()
    })
}

/// The URL-driven tab bar: seven pill anchors (`?tab=…`) replacing the thaw
/// `TabList`. Selected = `bg-accent-subtle text-accent-ink`; idle =
/// `text-ink-muted hover:bg-sunken`. Plain anchors keep the tabs working
/// before hydration (the router intercepts them once WASM loads).
fn tab_bar(ehr_id: Signal<String>, selected: Memo<String>) -> AnyView {
    let link = move |value: &'static str, label: &'static str| {
        let href = move || format!("/ehrs/{}?tab={value}", ehr_id.get());
        let class = move || {
            if selected.get() == value {
                "rounded-control px-3 py-1.5 text-sm font-medium bg-accent-subtle text-accent-ink"
            } else {
                "rounded-control px-3 py-1.5 text-sm font-medium text-ink-muted hover:bg-sunken"
            }
        };
        view! {
            <a href=href class=class>
                {label}
            </a>
        }
    };
    view! {
        <div class="flex flex-wrap gap-1 border-b border-edge pb-2">
            {link("status", "Status")} {link("status-history", "Status history")}
            {link("directory", "Directory")} {link("compositions", "Compositions")}
            {link("contributions", "Contributions")} {link("commit", "Commit")}
            {link("tags", "Tags")}
        </div>
    }
    .into_any()
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::parse_ehr_summary;

    #[test]
    fn parses_the_ehr_resources_summary_attributes() {
        // The RM `EHR` wire shape: `system_id`/`ehr_id` HIER_OBJECT_IDs, a
        // DV_DATE_TIME `time_created`, and the `ehr_status` OBJECT_REF whose id
        // is the CURRENT EHR_STATUS OBJECT_VERSION_ID.
        let body = r#"{
            "_type": "EHR",
            "system_id": {"_type": "HIER_OBJECT_ID", "value": "example.ferroehr.eu"},
            "ehr_id": {"_type": "HIER_OBJECT_ID", "value": "7d44aa01-0f9e-4a2c-9a0f-2a6a5f9b1c3d"},
            "ehr_status": {
                "_type": "OBJECT_REF",
                "namespace": "local",
                "type": "VERSIONED_EHR_STATUS",
                "id": {"_type": "OBJECT_VERSION_ID", "value": "8849182c::example.ferroehr.eu::1"}
            },
            "time_created": {"_type": "DV_DATE_TIME", "value": "2026-07-12T10:00:00Z"}
        }"#;
        let summary = parse_ehr_summary(body).expect("valid EHR resource");
        assert_eq!(summary.ehr_id, "7d44aa01-0f9e-4a2c-9a0f-2a6a5f9b1c3d");
        assert_eq!(summary.system_id, "example.ferroehr.eu");
        assert_eq!(summary.time_created, "2026-07-12T10:00:00Z");
        assert_eq!(summary.ehr_status_uid, "8849182c::example.ferroehr.eu::1");
        assert_eq!(summary.ehr_status_type, "VERSIONED_EHR_STATUS");
    }

    #[test]
    fn a_sparse_ehr_body_reads_as_empty_facts_and_a_bad_body_errors() {
        let summary = parse_ehr_summary("{}").expect("an empty object parses");
        assert_eq!(summary.ehr_id, "");
        assert_eq!(summary.system_id, "");
        assert_eq!(summary.ehr_status_uid, "");
        assert!(parse_ehr_summary("not json").is_err());
    }
}
