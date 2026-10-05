// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The source-upload modal: choose a file or paste the source, then send.
//!
//! ONE dialog every upload affordance opens, so "give me the artefact" looks
//! and behaves identically wherever it appears — the same doctrine as
//! [`crate::components::confirm_dialog`].
//!
//! The picker and the paste area feed ONE source signal on purpose: a chosen
//! file is loaded into the editor so it can be read, and corrected, before it
//! is sent, and there is exactly one thing the submit button can dispatch.
//!
//! State is a single source of truth: the caller owns the "is the dialog open"
//! signal, clears it in `on_dismiss`, and closes it from the upload action's
//! own async continuation on success — so the trigger, Cancel, Esc, a backdrop
//! click and a successful upload all write that same one signal.
//!
//! Hydration: the dialog is closed on the first render of BOTH passes and
//! `thaw::Dialog` teleports nothing while closed, so the server HTML and the
//! hydrated view agree.

use leptos::component;
use leptos::prelude::*;
use leptos::reactive::wrappers::write::SignalSetter;

use crate::components::field::{BTN_PRIMARY, BTN_SECONDARY, TEXTAREA};

/// A modal that takes an artefact source from a file or the keyboard and hands
/// it to the caller's upload action.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[expect(
    clippy::too_many_arguments,
    reason = "presentation props of one cohesive dialog (copy, accept list, DOM ids, state); \
              grouping them into a struct would only move the same fields one indirection away"
)]
#[component]
pub fn UploadDialog(
    /// Whether the dialog is open — the caller's own signal, never a copy.
    #[prop(into)]
    open: Signal<bool>,
    /// Dismissal (Cancel, Esc, backdrop click): clear the caller's signal here.
    on_dismiss: Callback<()>,
    /// The dialog heading, naming what is being uploaded.
    #[prop(into)]
    title: Signal<String>,
    /// One short paragraph: what the CDR ingests, and that refusals are shown
    /// verbatim.
    #[prop(into)]
    help: Signal<String>,
    /// The file picker's `accept` list (e.g. `".opt,.xml"`).
    #[prop(into)]
    accept: Signal<String>,
    /// The paste area's placeholder — a recognizable first line of the format.
    #[prop(into)]
    placeholder: Signal<String>,
    /// The file picker's button label.
    #[prop(into)]
    choose_label: Signal<String>,
    /// The submitting button's label.
    #[prop(into)]
    submit_label: Signal<String>,
    /// The source awaiting upload: the picker fills it, the area edits it, and
    /// the submit button dispatches exactly what it holds.
    source: RwSignal<String>,
    /// Whether an upload is in flight.
    #[prop(into)]
    pending: Signal<bool>,
    /// The refusal diagnostic to render verbatim, if the last attempt failed.
    #[prop(into)]
    error: Signal<Option<String>>,
    /// The upload: dispatch the caller's action with the source given here.
    on_submit: Callback<String>,
    /// The file input's wrapper DOM id — a stable E2E hook.
    picker_id: &'static str,
    /// The paste area's DOM id — a stable E2E hook.
    source_id: &'static str,
    /// The submitting button's DOM id — a stable E2E hook.
    submit_id: &'static str,
) -> impl IntoView {
    // `thaw::Dialog` wants a WRITABLE open model; back it with the caller's
    // signal plus a setter that only ever dismisses — the dialog never opens
    // itself, so this stays one piece of state.
    let dismissal = SignalSetter::map(move |value: bool| {
        if !value {
            on_dismiss.run(());
        }
    });
    // `custom_request` runs only in the browser (a file-selection event), so
    // reading the file with the Web `File`/`Blob` API here is hydration-safe
    // (browser-only APIs never run on the server pass).
    let custom_request = move |files: thaw::FileList| {
        let Some(file) = files.get(0) else {
            return;
        };
        let promise = file.text();
        leptos::task::spawn_local(async move {
            if let Ok(value) = wasm_bindgen_futures::JsFuture::from(promise).await
                && let Some(text) = value.as_string()
            {
                source.set(text);
            }
        });
    };
    let empty = Signal::derive(move || source.read().trim().is_empty());

    let body = view! {
        <p class="mb-3 text-sm text-ink-muted">{move || help.get()}</p>
        <div id=picker_id class="mb-2">
            <thaw::Upload accept custom_request>
                <thaw::Button>
                    <leptos_icons::Icon icon=icondata_lu::LuUpload width="14" height="14" />
                    {move || choose_label.get()}
                </thaw::Button>
            </thaw::Upload>
        </div>
        <textarea
            id=source_id
            class=format!("{TEXTAREA} min-h-[12rem]")
            placeholder=move || placeholder.get()
            prop:value=move || source.get()
            on:input:target=move |ev| source.set(ev.target().value())
        ></textarea>
        <div class="mt-2 text-sm">
            <Show when=move || pending.get()>
                <span class="text-ink-muted">"Uploading…"</span>
            </Show>
            // The refusal renders HERE, beside the input that caused it, so a
            // rejected paste can be corrected without retyping it.
            {move || match error.get() {
                Some(message) => {
                    view! {
                        <thaw::MessageBar intent=thaw::MessageBarIntent::Error>
                            <thaw::MessageBarBody>{message}</thaw::MessageBarBody>
                        </thaw::MessageBar>
                    }
                        .into_any()
                }
                None => ().into_any(),
            }}
        </div>
    }
    .into_any();

    view! {
        <thaw::Dialog open=(open, dismissal)>
            <thaw::DialogSurface>
                <thaw::DialogBody>
                    <thaw::DialogTitle>{move || title.get()}</thaw::DialogTitle>
                    <thaw::DialogContent>{body}</thaw::DialogContent>
                    <thaw::DialogActions>
                        <button
                            type="button"
                            class=BTN_SECONDARY
                            on:click=move |_| on_dismiss.run(())
                        >
                            "Cancel"
                        </button>
                        // Inert from first paint (a static `disabled` attribute
                        // for the server HTML) with the live state on
                        // `prop:disabled` — properties carry live state.
                        <button
                            id=submit_id
                            type="button"
                            class=BTN_PRIMARY
                            disabled=true
                            prop:disabled=move || empty.get() || pending.get()
                            on:click=move |_| on_submit.run(source.get_untracked())
                        >
                            <leptos_icons::Icon icon=icondata_lu::LuUpload width="14" height="14" />
                            {move || submit_label.get()}
                        </button>
                    </thaw::DialogActions>
                </thaw::DialogBody>
            </thaw::DialogSurface>
        </thaw::Dialog>
    }
}
