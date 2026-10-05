// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shared **version history** kit: the four surfaces every versioned
//! object's History tab is made of.
//!
//! One versioned family differs from another only in which endpoints it reads
//! and in the copy + DOM hooks it renders; the SHAPE is the same everywhere —
//! the versioned-object card, the revision-history table whose rows pin a
//! version, the at-time lookup that resolves an instant to a version, and the
//! pinned version's document. The screens own their resources and their
//! per-family copy ([`VersionHistoryLabels`]); this module owns the views.
//!
//! Nothing here creates a resource: the caller creates them once in setup and
//! hands the handles over.

use leptos::prelude::*;

use crate::components::data_table::{CELL, CELL_MONO, ROW, table_shell, table_skeleton};
use crate::components::empty_state::EmptyState;
use crate::components::field::{BTN_SECONDARY, INPUT, LABEL};
use crate::components::format_view::DocumentPane;
use crate::components::notice::inline_error;
use crate::components::surface::{CARD_PAD, CARD_TITLE};
use crate::components::wire::VersionEntry;
use crate::error::ViewerError;

/// The revision-history resource: a versioned object's commits, newest-first.
pub type HistoryResource = Resource<Result<Vec<VersionEntry>, ViewerError>>;

/// The pinned-version document resource (`None` while no version is pinned).
pub type DocumentResource = Resource<Result<Option<String>, ViewerError>>;

/// The copy and DOM hooks one versioned family's History tab renders with.
///
/// Everything here is presentation: the operator-facing wording of a family
/// (an `EHR_STATUS` version versus a plain version) and the stable E2E hooks.
/// Nothing in this struct changes what any section DOES.
#[derive(Debug, Clone, Copy)]
pub struct VersionHistoryLabels {
    /// The attribute each row's "Open" button carries its version id in — the
    /// stable E2E hook (`data-status-version`, `data-demographic-version`).
    pub row_hook: &'static str,
    /// The empty-history message.
    pub empty_message: &'static str,
    /// The empty-history hint.
    pub empty_hint: &'static str,
    /// The revision-history card's lead paragraph.
    pub history_lead: &'static str,
    /// The at-time card's heading.
    pub at_time_title: &'static str,
    /// The at-time input's DOM id (also its label's `for`).
    pub at_time_field_id: &'static str,
    /// The at-time button's DOM id.
    pub at_time_button_id: &'static str,
    /// The neutral note a `404` from the at-time lookup renders.
    pub at_time_absent: &'static str,
    /// The pinned document pane's DOM id.
    pub document_id: &'static str,
}

/// The versioned-object card: the container's own facts plus the pinned
/// VERSION's envelope facts, rendered by the family's own `card`.
///
/// A `<Transition>` so switching version keeps the previous facts visible,
/// with the `Result` resolved inside it. `card` is a plain function pointer,
/// so each family lays its own facts out — the shape shared here is the
/// loading, absent and failed states around it.
#[must_use]
pub fn versioned_facts_section<T>(
    versioned: Resource<Result<Option<T>, ViewerError>>,
    card: fn(&T) -> AnyView,
) -> AnyView
where
    T: Clone + Send + Sync + 'static,
{
    view! {
        <Transition fallback=|| {
            view! {
                <thaw::Skeleton>
                    <thaw::SkeletonItem class="h-24" />
                </thaw::Skeleton>
            }
        }>
            {move || Suspend::new(async move {
                match versioned.await {
                    Ok(Some(facts)) => card(&facts),
                    Ok(None) => ().into_any(),
                    Err(e) => inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// The revision-history card: one row per committed version, newest-first, each
/// row pinning that version into `pinned` for the document pane below.
#[must_use]
pub fn revision_history_section(
    history: HistoryResource,
    pinned: RwSignal<String>,
    labels: VersionHistoryLabels,
) -> AnyView {
    let table = view! {
        <Transition fallback=table_skeleton>
            {move || Suspend::new(async move {
                match history.await {
                    Ok(entries) if entries.is_empty() => {
                        view! {
                            <EmptyState
                                icon=icondata_lu::LuHistory
                                message=labels.empty_message
                                hint=labels.empty_hint
                            />
                        }
                            .into_any()
                    }
                    Ok(entries) => history_table(entries, pinned, labels.row_hook),
                    Err(e) => inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any();
    view! {
        <section class=CARD_PAD>
            <h2 class=CARD_TITLE>"Revision history"</h2>
            <p class="mb-3 text-xs text-ink-muted">{labels.history_lead}</p>
            {table}
        </section>
    }
    .into_any()
}

/// Render the history rows in the shared table kit. `<For>` keyed on the
/// version's own `OBJECT_VERSION_ID` — stable, unique, data-derived.
fn history_table(
    entries: Vec<VersionEntry>,
    pinned: RwSignal<String>,
    row_hook: &'static str,
) -> AnyView {
    let rows = view! {
        <For each=move || entries.clone() key=|entry| entry.version_id.clone() let:entry>
            {history_row(&entry, pinned, row_hook)}
        </For>
    }
    .into_any();
    table_shell(
        &["Version", "Committed", "Change type", "Committer", ""],
        rows,
    )
}

/// One revision-history row plus its "Open" action.
///
/// The hook attribute's NAME is the family's (`row_hook`), so the button's whole
/// attribute set is applied with the custom-attribute API: attributes serialize
/// in the order they are applied, and a view-declared one would sort AFTER them.
fn history_row(entry: &VersionEntry, pinned: RwSignal<String>, row_hook: &'static str) -> AnyView {
    let version_id = entry.version_id.clone();
    let hook = version_id.clone();
    let target = version_id.clone();
    let shown = version_id.clone();
    // The pinned row is tinted. A computed class string (not `class:`) because
    // the accent token carries a hyphen, which the `class:name=` shorthand
    // cannot spell.
    let row_class = move || {
        if pinned.with(|current| current.as_str() == version_id.as_str()) {
            format!("{ROW} bg-accent-subtle")
        } else {
            ROW.to_owned()
        }
    };
    let committed = entry.committed.clone();
    let change_type = entry.change_type.clone();
    let committed_by = entry.committer.clone();
    let open = view! {
        <button on:click=move |_| pinned.set(target.clone())>
            <leptos_icons::Icon icon=icondata_lu::LuEye width="14" height="14" />
            "Open"
        </button>
    }
    .attr("type", "button")
    .attr("class", BTN_SECONDARY)
    .attr(row_hook, hook);
    view! {
        <tr class=row_class>
            <td class=CELL_MONO>{shown}</td>
            <td class=CELL>{committed}</td>
            <td class=CELL>{change_type}</td>
            <td class=CELL>{committed_by}</td>
            <td class=CELL>{open}</td>
        </tr>
    }
    .into_any()
}

/// The at-time lookup: a `datetime-local` input plus a button that resolves the
/// VERSION extant at that instant and pins it.
///
/// The `404` answer ("no version at that time") is a neutral note beside the
/// control, not an error bar — it is the answer to the question asked. Any other
/// failure renders through the normal inline-error path (a pure read, so no
/// toast — the viewer's feedback rule). `subject` is the id the lookup is
/// dispatched for (the EHR, or the demographic object).
#[must_use]
pub fn at_time_lookup_section(
    subject: Signal<String>,
    at_time: Action<(String, String), Result<String, ViewerError>>,
    at_time_input: RwSignal<String>,
    labels: VersionHistoryLabels,
) -> AnyView {
    let on_go = move |_| {
        let requested = at_time_input.get();
        if !requested.trim().is_empty() {
            at_time.dispatch((subject.get(), requested));
        }
    };
    let note = move || match at_time.value().get() {
        Some(Err(ref e)) if e.status_code() == Some(http::StatusCode::NOT_FOUND) => {
            view! { <p class="mt-2 text-sm text-ink-muted">{labels.at_time_absent}</p> }.into_any()
        }
        Some(Err(error)) => inline_error(&error),
        _ => ().into_any(),
    };
    view! {
        <section class=CARD_PAD>
            <h2 class=CARD_TITLE>{labels.at_time_title}</h2>
            <div class="flex flex-wrap items-end gap-3">
                <div class="flex flex-col gap-1">
                    <label class=LABEL r#for=labels.at_time_field_id>
                        "Date and time (interpreted as UTC)"
                    </label>
                    <input
                        id=labels.at_time_field_id
                        type="datetime-local"
                        class=INPUT
                        prop:value=move || at_time_input.get()
                        on:input:target=move |ev| at_time_input.set(ev.target().value())
                    />
                </div>
                <button
                    id=labels.at_time_button_id
                    type="button"
                    class=BTN_SECONDARY
                    disabled=Signal::derive(move || at_time.pending().get())
                    on:click=on_go
                >
                    <leptos_icons::Icon icon=icondata_lu::LuClock width="14" height="14" />
                    "Open that version"
                </button>
            </div>
            {note}
        </section>
    }
    .into_any()
}

/// The pinned version's document, read by its `OBJECT_VERSION_ID`.
///
/// A `<Transition>` so switching version keeps the previous document
/// visible. Nothing pinned is a first-class empty state, not an error.
#[must_use]
pub fn pinned_document_section(
    document: DocumentResource,
    pinned: RwSignal<String>,
    labels: VersionHistoryLabels,
) -> AnyView {
    let heading = move || {
        let version = pinned.get();
        if version.is_empty() {
            "Version document".to_owned()
        } else {
            format!("Version {version}")
        }
    };
    let body = view! {
        <Transition fallback=table_skeleton>
            {move || Suspend::new(async move {
                match document.await {
                    Ok(Some(body)) => {
                        let doc = RwSignal::new(body);
                        view! {
                            <div id=labels.document_id>
                                <DocumentPane body=doc />
                            </div>
                        }
                            .into_any()
                    }
                    Ok(None) => {
                        view! {
                            <EmptyState
                                icon=icondata_lu::LuFileClock
                                message="No version opened"
                                hint="Open a version from the revision history above, or resolve one by date and time."
                            />
                        }
                            .into_any()
                    }
                    Err(e) => inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any();
    view! {
        <section class=CARD_PAD>
            <h2 class=CARD_TITLE>{heading}</h2>
            {body}
        </section>
    }
    .into_any()
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::history_row;
    use crate::components::wire::VersionEntry;
    use leptos::prelude::*;

    #[test]
    fn the_open_button_carries_the_familys_hook_after_its_type_and_class() {
        // The E2E journeys click `[data-status-version$='::2']`, and the
        // attribute's NAME is a parameter here — so the server pass is what
        // proves it is emitted, in the order the row has always carried it.
        let entry = VersionEntry {
            version_id: "8849182c::example.org::2".to_owned(),
            committed: "2026-08-29T09:00:00Z".to_owned(),
            change_type: "creation".to_owned(),
            committer: "operator".to_owned(),
        };
        let owner = Owner::new();
        let html = owner.with(|| {
            let pinned = RwSignal::new(String::new());
            history_row(&entry, pinned, "data-status-version").to_html()
        });
        assert!(
            html.contains(concat!(
                "<button type=\"button\" class=\"",
                "inline-flex items-center gap-1.5 rounded-control border border-edge-strong ",
                "bg-raised px-3 py-1.5 text-sm font-medium text-ink hover:bg-sunken ",
                "focus:outline-none focus:ring-2 focus:ring-accent ",
                "disabled:opacity-50 disabled:pointer-events-none\" ",
                "data-status-version=\"8849182c::example.org::2\">"
            )),
            "the Open button's attributes are missing or reordered: {html}"
        );
        assert!(html.contains("2026-08-29T09:00:00Z"), "{html}");
        assert!(html.contains("creation"), "{html}");
        assert!(html.contains("operator"), "{html}");
    }
}
