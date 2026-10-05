// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The empty-directory create flow.
//!
//! For an EHR the CDR has no directory for (a `404`), commit an empty root
//! `FOLDER` with `POST /ehr/{ehr_id}/directory` (ITS-REST
//! `specifications/operations/directory_create.yaml`), then build the tree in
//! the structured editor.
//!
//! NOTE: create-empty is the ONLY create path. The viewer offers no library of
//! folder shapes of its own — no openEHR spec governs a viewer convenience
//! like that (our own design/extension), and a viewer-local library would be
//! state invisible to every other openEHR client. Structure is built in the
//! tree editor, which commits it as ordinary directory versions the CDR owns.

use leptos::prelude::*;

use crate::components::field::{BTN_PRIMARY, LABEL};
use crate::components::surface::{CARD_PAD, CARD_TITLE, WELL};
use crate::error::ViewerError;
use crate::pages::ehr_detail::directory::empty_root_folder;
use crate::pages::ehr_detail::directory::tree::read_only_tree;

/// The no-directory view: a preview of the empty root `FOLDER` that will be
/// committed, and the button that commits it through the shared `create`
/// action. Stateless — it holds no signals of its own, so re-running it when
/// the directory resource notifies (a `Suspend` re-run) disposes nothing.
pub(in crate::pages::ehr_detail::directory) fn create_section(
    ehr_id: Signal<String>,
    create: Action<(String, String), Result<String, ViewerError>>,
) -> AnyView {
    let on_create = move |_| {
        let body = serde_json::to_string(&empty_root_folder()).unwrap_or_default();
        create.dispatch((ehr_id.get(), body));
    };
    view! {
        <section class=CARD_PAD>
            <h2 class=CARD_TITLE>"Create directory"</h2>
            <p class="mb-3 text-sm text-ink-muted">
                "This EHR has no directory yet. Create the empty root folder, then add sub-folders and item references in the tree editor."
            </p>
            <div class="flex flex-col gap-3">
                <div class="flex flex-col gap-1">
                    <span class=LABEL>"Will be committed"</span>
                    <div class=WELL>{read_only_tree(&empty_root_folder())}</div>
                </div>
                <div class="flex items-center gap-3">
                    <button
                        id="directory-create"
                        type="button"
                        class=BTN_PRIMARY
                        disabled=Signal::derive(move || create.pending().get())
                        on:click=on_create
                    >
                        <leptos_icons::Icon icon=icondata_lu::LuPlus width="14" height="14" />
                        "Create directory"
                    </button>
                    <Show when=move || create.pending().get()>
                        <span class="text-sm text-ink-muted">"Creating…"</span>
                    </Show>
                </div>
                {create_feedback(create)}
            </div>
        </section>
    }
    .into_any()
}

/// The create action's failure pane: the CDR's diagnostics verbatim in a
/// scrollable WELL (`<pre>`). Success is a toast (see the section orchestrator).
fn create_feedback(create: Action<(String, String), Result<String, ViewerError>>) -> AnyView {
    view! {
        {move || match create.value().get() {
            Some(Err(error)) => {
                view! {
                    <div class=WELL>
                        <pre class="max-h-[40vh] overflow-auto whitespace-pre-wrap font-mono text-xs text-danger">
                            {error.to_string()}
                        </pre>
                    </div>
                }
                    .into_any()
            }
            _ => ().into_any(),
        }}
    }
    .into_any()
}
