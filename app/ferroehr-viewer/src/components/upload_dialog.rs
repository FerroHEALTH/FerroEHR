// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The source-upload modal: FerroUI's wide modal dialog holding FerroUI's
//! upload field (`ferrohealth_ui::widget::upload`), where a file is dropped,
//! chosen or pasted, read in the browser, and sent.
//!
//! ONE dialog every upload affordance opens. The field reads a chosen file into
//! the ONE source signal on purpose: the file can be read, and corrected,
//! before it is sent, and there is exactly one thing the submit button can
//! dispatch.
//!
//! State is a single source of truth: the caller owns the "is the dialog open"
//! signal, clears it in `on_dismiss`, and closes it from the upload action's
//! own async continuation on success.

use ferrohealth_ui::icon::{Glyph, icon};
use ferrohealth_ui::widget::dialog::{Modal, Width, modal_dialog};
use ferrohealth_ui::widget::upload::{Upload, upload_field};
use leptos::component;
use leptos::prelude::*;

use crate::components::field::BTN_PRIMARY;

/// A modal that takes an artefact source from a file or the keyboard and hands
/// it to the caller's upload action.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn UploadDialog(
    /// Whether the dialog is open — the caller's own signal, never a copy.
    #[prop(into)]
    open: Signal<bool>,
    /// Dismissal (Cancel, Esc, a press outside): clear the caller's signal.
    on_dismiss: Callback<()>,
    /// The dialog heading, naming what is being uploaded.
    #[prop(into)]
    title: Signal<String>,
    /// One short paragraph: what the CDR ingests, and that refusals are shown
    /// verbatim.
    #[prop(into)]
    help: Signal<String>,
    /// What the field accepts and says: its ids, label, chooser words,
    /// extensions, size limit and placeholder.
    #[prop(into)]
    upload: Signal<Upload>,
    /// The submitting button's label.
    submit_label: &'static str,
    /// The source awaiting upload: the field fills it, and the submit button
    /// dispatches exactly what it holds.
    source: RwSignal<String>,
    /// Whether an upload is in flight.
    #[prop(into)]
    pending: Signal<bool>,
    /// The refusal diagnostic to render verbatim, if the last attempt failed.
    #[prop(into)]
    error: Signal<Option<String>>,
    /// The upload: dispatch the caller's action with the source given here.
    on_submit: Callback<String>,
    /// The dialog's DOM id.
    dialog_id: &'static str,
    /// The submitting button's DOM id — a stable E2E hook.
    submit_id: &'static str,
) -> impl IntoView {
    let empty = Signal::derive(move || source.read().trim().is_empty());
    let body = view! {
        <p class="text-sm text-ink-muted">{move || help.get()}</p>
        {move || upload_field(upload.get(), source, pending, error)}
    }
    .into_any();
    // Inert from first paint (a static `disabled` attribute for the server
    // HTML) with the live state on `prop:disabled` — properties carry live
    // state.
    let actions = view! {
        <button
            id=submit_id
            type="button"
            class=BTN_PRIMARY
            disabled=true
            prop:disabled=move || empty.get() || pending.get()
            on:click=move |_| on_submit.run(source.get_untracked())
        >
            {icon(Glyph::UPLOAD, "16")}
            {submit_label}
        </button>
    }
    .into_any();
    modal_dialog(
        Modal {
            id: dialog_id,
            title,
            danger: false,
            width: Width::Wide,
            body,
            actions,
        },
        open,
        on_dismiss,
    )
}
