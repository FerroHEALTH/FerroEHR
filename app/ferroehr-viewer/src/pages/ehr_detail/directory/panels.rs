// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The directory tab's supporting panels.
//!
//! The toolbar (history / time / path toggles + delete), the version-history
//! panel with time-travel and restore, the `version_at_time` panel, and the
//! `?path=` subtree panel. All are rendered outside the main directory
//! `<Suspense>` with their own `<Transition>` boundaries; their read resources
//! are created once in the section orchestrator and only fetch when their
//! panel is open.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use leptos::prelude::*;
use serde_json::Value;

use crate::components::empty_state::EmptyState;
use crate::components::field::{BTN_DANGER, BTN_SECONDARY, INPUT, LABEL};
use crate::components::notice::inline_error;
use crate::components::surface::{CARD_PAD, CARD_TITLE, WELL};
use crate::error::ViewerError;
use crate::pages::ehr_detail::directory::tree::read_only_tree;
use crate::pages::ehr_detail::directory::{
    DirectoryAtTime, DirectoryHistory, DirectoryState, DirectorySubtree, DirectoryVersion,
};

/// A toolbar toggle button, styled active when its panel is open.
fn toggle_class(open: bool) -> &'static str {
    if open {
        "inline-flex items-center gap-1.5 rounded-control bg-accent-subtle px-3 py-1.5 text-sm font-medium text-accent-ink focus:outline-none focus:ring-2 focus:ring-accent"
    } else {
        BTN_SECONDARY
    }
}

/// A version-history row button, highlighted when selected.
fn row_class(selected: bool) -> &'static str {
    if selected {
        "flex w-full flex-wrap items-center gap-2 rounded-control border border-accent bg-accent-subtle px-3 py-1.5 text-left text-sm"
    } else {
        "flex w-full flex-wrap items-center gap-2 rounded-control border border-edge px-3 py-1.5 text-left text-sm hover:bg-sunken"
    }
}

/// The directory toolbar: history / time / path toggles and a two-step
/// delete. Shown only when a directory exists. The existence flag comes from
/// the shared directory resource, which MUST be resolved under a
/// `<Transition>` boundary — a bare render-time read of a resource is a
/// hydration mismatch in hydrate mode (; caught live by the composed e2e
/// battery: the mismatch killed page interactivity).
pub(in crate::pages::ehr_detail::directory) fn directory_toolbar(
    ehr_id: Signal<String>,
    directory: Resource<Result<Option<DirectoryState>, ViewerError>>,
    delete: Action<(String, String), Result<(), ViewerError>>,
    history_open: RwSignal<bool>,
    time_open: RwSignal<bool>,
    path_open: RwSignal<bool>,
) -> AnyView {
    // Created OUTSIDE the Suspend so the confirm state survives re-runs.
    let confirm_delete = RwSignal::new(false);
    view! {
        <Transition fallback=|| ()>
            {move || Suspend::new(async move {
                let has_directory = matches!(directory.await, Ok(Some(_)));
                toolbar_body(
                    ehr_id,
                    directory,
                    delete,
                    history_open,
                    time_open,
                    path_open,
                    confirm_delete,
                    has_directory,
                )
            })}
        </Transition>
    }
    .into_any()
}

/// The toolbar's rendered body (built fresh per directory resolution — no
/// resource reads at render time; the click handlers read untracked).
#[expect(
    clippy::too_many_arguments,
    reason = "one view fn wiring the toolbar's full state set"
)]
fn toolbar_body(
    ehr_id: Signal<String>,
    directory: Resource<Result<Option<DirectoryState>, ViewerError>>,
    delete: Action<(String, String), Result<(), ViewerError>>,
    history_open: RwSignal<bool>,
    time_open: RwSignal<bool>,
    path_open: RwSignal<bool>,
    confirm_delete: RwSignal<bool>,
    has_directory: bool,
) -> AnyView {
    let on_delete = move |_| {
        if let Some(Ok(Some(state))) = directory.get_untracked() {
            delete.dispatch((ehr_id.get(), state.version_uid.clone()));
        }
        confirm_delete.set(false);
    };

    view! {
        <section class=CARD_PAD class:hidden=!has_directory>
            <div class="flex flex-wrap items-center justify-between gap-2">
                <div class="flex flex-wrap items-center gap-2">
                    <button
                        type="button"
                        class=move || toggle_class(history_open.get())
                        on:click=move |_| history_open.update(|o| *o = !*o)
                    >
                        <leptos_icons::Icon icon=icondata_lu::LuHistory width="14" height="14" />
                        "Version history"
                    </button>
                    <button
                        type="button"
                        class=move || toggle_class(time_open.get())
                        on:click=move |_| time_open.update(|o| *o = !*o)
                    >
                        <leptos_icons::Icon icon=icondata_lu::LuClock width="14" height="14" />
                        "At time"
                    </button>
                    <button
                        type="button"
                        class=move || toggle_class(path_open.get())
                        on:click=move |_| path_open.update(|o| *o = !*o)
                    >
                        <leptos_icons::Icon
                            icon=icondata_lu::LuFolderSearch
                            width="14"
                            height="14"
                        />
                        "Path query"
                    </button>
                </div>
                <button type="button" class=BTN_DANGER on:click=move |_| confirm_delete.set(true)>
                    <leptos_icons::Icon icon=icondata_lu::LuTrash width="14" height="14" />
                    "Delete directory"
                </button>
            </div>

            // The inline delete confirmation (danger zone).
            <div
                class="mt-3 flex flex-wrap items-center gap-3 rounded-control border border-danger/40 bg-danger-subtle px-3 py-2"
                class:hidden=move || !confirm_delete.get()
            >
                <span class="text-sm text-danger">
                    "Permanently delete this EHR's directory? This creates a deletion version."
                </span>
                <button
                    id="directory-delete-confirm"
                    type="button"
                    class=BTN_DANGER
                    disabled=Signal::derive(move || delete.pending().get())
                    on:click=on_delete
                >
                    "Delete directory"
                </button>
                <button
                    type="button"
                    class=BTN_SECONDARY
                    on:click=move |_| confirm_delete.set(false)
                >
                    "Cancel"
                </button>
            </div>
        </section>
    }
    .into_any()
}

/// The version-history panel: the newest window of version rows, a
/// selected-version read-only preview, a restore action (a `PUT` of the chosen
/// version's tree against the current latest `If-Match`), and — while older
/// versions remain — a "load older" affordance that widens the window.
pub(in crate::pages::ehr_detail::directory) fn history_panel(
    ehr_id: Signal<String>,
    directory: Resource<Result<Option<DirectoryState>, ViewerError>>,
    versions: Resource<Result<DirectoryHistory, ViewerError>>,
    restore: Action<(String, String, String), Result<String, ViewerError>>,
    history_open: RwSignal<bool>,
    history_window: RwSignal<u32>,
) -> AnyView {
    let selected = RwSignal::new(Option::<String>::None);
    view! {
        <section class=CARD_PAD class:hidden=move || !history_open.get()>
            <h2 class=CARD_TITLE>"Version history"</h2>
            <Transition fallback=|| {
                view! { <p class="text-sm text-ink-muted">"Loading versions…"</p> }
            }>
                {move || Suspend::new(async move {
                    match versions.await {
                        Ok(history) if history.versions.is_empty() => {
                            view! {
                                <EmptyState
                                    icon=icondata_lu::LuHistory
                                    message="No versions yet"
                                    hint="This EHR has no directory — create one and its versions are listed here."
                                />
                            }
                                .into_any()
                        }
                        Ok(history) => {
                            history_content(
                                ehr_id,
                                directory,
                                restore,
                                history,
                                selected,
                                history_window,
                            )
                        }
                        Err(e) => inline_error(&e),
                    }
                })}
            </Transition>
        </section>
    }
    .into_any()
}

/// The history rows, the "load older" affordance, and the selected-version
/// preview.
fn history_content(
    ehr_id: Signal<String>,
    directory: Resource<Result<Option<DirectoryState>, ViewerError>>,
    restore: Action<(String, String, String), Result<String, ViewerError>>,
    history: DirectoryHistory,
    selected: RwSignal<Option<String>>,
    history_window: RwSignal<u32>,
) -> AnyView {
    let rows_source = history.versions.clone();
    let has_older = history.has_older;
    let versions = StoredValue::new(history.versions);
    let rows = view! {
        <For each=move || rows_source.clone() key=|v| v.version_uid.clone() let:version>
            {history_row(&version, selected)}
        </For>
    };
    let preview = move || {
        let chosen = selected.get().and_then(|uid| {
            versions.with_value(|l| l.iter().find(|v| v.version_uid == uid).cloned())
        });
        match chosen {
            Some(version) => version_preview(ehr_id, directory, restore, &version),
            None => {
                view! { <p class="text-sm text-ink-muted">"Select a version to preview it."</p> }
                    .into_any()
            }
        }
    };
    view! {
        <div class="flex flex-col gap-3">
            <ul class="flex flex-col gap-1">{rows}</ul>
            {load_older(has_older, history_window)}
            <div>{preview}</div>
        </div>
    }
    .into_any()
}

/// The "load older" affordance: one more page of history per click, rendered
/// only while the CDR still has versions beyond the loaded window. There is
/// deliberately no "load all" — an unbounded walk is the defect this replaces.
fn load_older(has_older: bool, history_window: RwSignal<u32>) -> AnyView {
    if !has_older {
        return ().into_any();
    }
    let widen = move |_| {
        history_window.update(|w| *w = w.saturating_add(crate::components::data_table::PAGE_SIZE));
    };
    view! {
        <div>
            <button id="directory-history-older" type="button" class=BTN_SECONDARY on:click=widen>
                <leptos_icons::Icon icon=icondata_lu::LuChevronDown width="14" height="14" />
                "Load older versions"
            </button>
        </div>
    }
    .into_any()
}

/// One version row (selectable).
fn history_row(version: &DirectoryVersion, selected: RwSignal<Option<String>>) -> AnyView {
    let uid = version.version_uid.clone();
    let uid_selected = uid.clone();
    let is_selected = move || selected.with(|s| s.as_deref() == Some(uid_selected.as_str()));
    let on_click = move |_| selected.set(Some(uid.clone()));
    let label = format!("v{}", version.number);
    let name = version.root_name.clone();
    let counts = format!(
        "{} folders · {} items",
        version.folder_count, version.item_count
    );
    let is_latest = version.is_latest;
    view! {
        <li>
            <button type="button" class=move || row_class(is_selected()) on:click=on_click>
                <span class="font-mono text-xs text-ink">{label}</span>
                <span class="text-ink">{name}</span>
                <span class="text-xs text-ink-muted">{counts}</span>
                {is_latest
                    .then(|| {
                        view! {
                            <span class="rounded-control bg-accent-subtle px-1.5 py-0.5 text-xs font-medium text-accent-ink">
                                "current"
                            </span>
                        }
                    })}
            </button>
        </li>
    }
    .into_any()
}

/// The selected version's read-only tree, with a restore action for non-latest
/// versions (restore = `PUT` the version's tree against the current latest).
fn version_preview(
    ehr_id: Signal<String>,
    directory: Resource<Result<Option<DirectoryState>, ViewerError>>,
    restore: Action<(String, String, String), Result<String, ViewerError>>,
    version: &DirectoryVersion,
) -> AnyView {
    let tree = match serde_json::from_str::<Value>(&version.body) {
        Ok(value) => read_only_tree(&value),
        Err(e) => inline_error(&ViewerError::Internal(format!("version JSON: {e}"))),
    };
    let is_latest = version.is_latest;
    let label = format!("v{}", version.number);
    let restore_body = version.body.clone();
    view! {
        <div class=WELL>
            <div class="mb-2 flex flex-wrap items-center justify-between gap-2">
                <span class="text-sm font-medium text-ink">{label}</span>
                // `Show` re-invokes its children (an `Fn`), so the click
                // handler is built fresh per invocation from a per-call clone.
                <Show when=move || {
                    !is_latest
                }>
                    {
                        let restore_body = restore_body.clone();
                        view! {
                            <button
                                type="button"
                                class=BTN_SECONDARY
                                disabled=Signal::derive(move || restore.pending().get())
                                on:click=move |_| {
                                    if let Some(Ok(Some(current))) = directory.get_untracked() {
                                        restore
                                            .dispatch((
                                                ehr_id.get(),
                                                current.version_uid.clone(),
                                                restore_body.clone(),
                                            ));
                                    }
                                }
                            >
                                <leptos_icons::Icon
                                    icon=icondata_lu::LuRotateCcw
                                    width="14"
                                    height="14"
                                />
                                "Restore this version"
                            </button>
                        }
                    }
                </Show>
            </div>
            {tree}
        </div>
    }
    .into_any()
}

/// The `version_at_time` panel: a datetime input driving the shared `at_time`
/// resource, distinguishing present / deleted-at-time / no-directory-then.
pub(in crate::pages::ehr_detail::directory) fn time_travel_panel(
    at_time: Resource<Result<Option<DirectoryAtTime>, ViewerError>>,
    time_input: RwSignal<String>,
    time_open: RwSignal<bool>,
) -> AnyView {
    view! {
        <section class=CARD_PAD class:hidden=move || !time_open.get()>
            <h2 class=CARD_TITLE>"Directory at a point in time"</h2>
            <div class="mb-3 flex flex-col gap-1">
                <label class=LABEL r#for="directory-at-time">
                    "Date and time (interpreted as UTC)"
                </label>
                <input
                    id="directory-at-time"
                    type="datetime-local"
                    class=INPUT
                    prop:value=move || time_input.get()
                    on:input:target=move |ev| time_input.set(ev.target().value())
                />
            </div>
            <Transition fallback=|| {
                view! { <p class="text-sm text-ink-muted">"Loading…"</p> }
            }>
                {move || Suspend::new(async move {
                    match at_time.await {
                        Ok(None) => {
                            view! {
                                <p class="text-sm text-ink-muted">
                                    "Pick a date and time to view the directory as it stood then."
                                </p>
                            }
                                .into_any()
                        }
                        Ok(Some(DirectoryAtTime::Present(state))) => {
                            match serde_json::from_str::<Value>(&state.body) {
                                Ok(value) => read_only_tree(&value),
                                Err(e) => {
                                    inline_error(
                                        &ViewerError::Internal(format!("directory JSON: {e}")),
                                    )
                                }
                            }
                        }
                        Ok(Some(DirectoryAtTime::DeletedAtTime)) => {
                            view! {
                                <EmptyState
                                    icon=icondata_lu::LuTriangleAlert
                                    message="Deleted at that time"
                                    hint="The directory had been deleted as of the selected time."
                                />
                            }
                                .into_any()
                        }
                        Ok(Some(DirectoryAtTime::NoneAtTime)) => {
                            view! {
                                <EmptyState
                                    icon=icondata_lu::LuClock
                                    message="No directory then"
                                    hint="No directory existed at the selected time."
                                />
                            }
                                .into_any()
                        }
                        Err(e) => inline_error(&e),
                    }
                })}
            </Transition>
        </section>
    }
    .into_any()
}

/// The `?path=` subtree panel: a path input driving the shared `at_path`
/// resource, rendering the matched sub-FOLDER or a miss.
pub(in crate::pages::ehr_detail::directory) fn path_panel(
    at_path: Resource<Result<Option<DirectorySubtree>, ViewerError>>,
    path_input: RwSignal<String>,
    path_open: RwSignal<bool>,
) -> AnyView {
    view! {
        <section class=CARD_PAD class:hidden=move || !path_open.get()>
            <h2 class=CARD_TITLE>"Sub-folder by path"</h2>
            <div class="mb-3 flex flex-col gap-1">
                <label class=LABEL r#for="directory-path">
                    "Path (e.g. folder-a/folder-b)"
                </label>
                <input
                    id="directory-path"
                    type="text"
                    class=INPUT
                    placeholder="folder-a/folder-b"
                    prop:value=move || path_input.get()
                    on:input:target=move |ev| path_input.set(ev.target().value())
                />
            </div>
            <Transition fallback=|| {
                view! { <p class="text-sm text-ink-muted">"Loading…"</p> }
            }>
                {move || Suspend::new(async move {
                    match at_path.await {
                        Ok(None) => {
                            view! {
                                <p class="text-sm text-ink-muted">
                                    "Enter a path to view only that sub-folder of the directory."
                                </p>
                            }
                                .into_any()
                        }
                        Ok(Some(DirectorySubtree::Found(body))) => {
                            match serde_json::from_str::<Value>(&body) {
                                Ok(value) => read_only_tree(&value),
                                Err(e) => {
                                    inline_error(
                                        &ViewerError::Internal(format!("subtree JSON: {e}")),
                                    )
                                }
                            }
                        }
                        Ok(Some(DirectorySubtree::Missing)) => {
                            view! {
                                <EmptyState
                                    icon=icondata_lu::LuFolderX
                                    message="No folder at that path"
                                    hint="The directory has no sub-folder matching the path."
                                />
                            }
                                .into_any()
                        }
                        Err(e) => inline_error(&e),
                    }
                })}
            </Transition>
        </section>
    }
    .into_any()
}
