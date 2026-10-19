// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The destructive-confirmation modal: FerroUI's modal dialog
//! (`ferrohealth_ui::widget::dialog`) in its danger form.
//!
//! ONE dialog every delete affordance in the viewer opens, so "are you sure?"
//! looks and behaves identically everywhere.
//!
//! State is a single source of truth: the caller owns the signal saying WHICH
//! object is awaiting confirmation, derives `open` from it, and clears it in
//! `on_cancel` — so the trigger, Cancel, Esc and a press outside all write the
//! same one signal, and no second copy of "is the dialog open" can drift.

use ferrohealth_ui::icon::{Glyph, icon};
use ferrohealth_ui::widget::dialog::{Modal, Width, modal_dialog};
use leptos::component;
use leptos::prelude::*;

use crate::components::field::BTN_DESTRUCTIVE;
use crate::error::ViewerError;

/// The row-driven delete confirmation a listing screen mounts once, outside its
/// table.
///
/// `pending` is both "which row" and "is the dialog open", so a list refetch
/// never re-creates the dialog and no second copy of that state can drift.
/// Confirming dispatches `delete` with the pending row and clears it; the
/// screen's toasts report the answer ([`toast_outcome`](crate::components::toast::toast_outcome)).
#[must_use]
pub fn delete_confirmation<R>(
    pending: RwSignal<Option<R>>,
    delete: Action<R, (String, Result<(), ViewerError>)>,
    title: &'static str,
    confirm_label: &'static str,
    ids: (&'static str, &'static str),
    message: Signal<String>,
) -> AnyView
where
    R: Clone + Send + Sync + 'static,
{
    let (dialog_id, confirm_id) = ids;
    view! {
        <ConfirmDialog
            open=Signal::derive(move || pending.get().is_some())
            title=title
            message=message
            confirm_label=confirm_label
            dialog_id=dialog_id
            confirm_id=confirm_id
            on_cancel=Callback::new(move |()| pending.set(None))
            on_confirm=Callback::new(move |()| {
                if let Some(row) = pending.get_untracked() {
                    drop(delete.dispatch(row));
                }
                pending.set(None);
            })
        />
    }
    .into_any()
}

/// A modal confirmation for one destructive action: a title, the consequence
/// copy naming the exact object, Cancel, and the solid destructive confirm.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn ConfirmDialog(
    /// Whether the dialog is open — derive it from the caller's "which object
    /// is awaiting confirmation" signal; never a second copy of that state.
    #[prop(into)]
    open: Signal<bool>,
    /// The dialog heading: the action, e.g. "Delete template".
    title: &'static str,
    /// The consequence copy, naming the exact object and what is lost.
    #[prop(into)]
    message: Signal<String>,
    /// The confirming button's label.
    confirm_label: &'static str,
    /// The confirming button's icon. Defaults to the trash glyph (the
    /// destructive-delete case this dialog was built for); a non-delete
    /// consequence — swapping a live log filter — passes its own.
    #[prop(optional, into)]
    confirm_icon: Option<Glyph>,
    /// The dialog's DOM id; its heading is this with `-title`.
    dialog_id: &'static str,
    /// The confirming button's DOM id — the stable E2E hook.
    confirm_id: &'static str,
    /// Dismissal (Cancel, Esc, a press outside): clear the caller's target
    /// signal here.
    on_cancel: Callback<()>,
    /// Confirmation: dispatch the destructive action here.
    on_confirm: Callback<()>,
) -> impl IntoView {
    let glyph = confirm_icon.unwrap_or(Glyph::TRASH);
    let body = view! { <p class="text-sm text-ink">{move || message.get()}</p> }.into_any();
    let actions = view! {
        <button
            id=confirm_id
            type="button"
            class=BTN_DESTRUCTIVE
            on:click=move |_| on_confirm.run(())
        >
            {icon(glyph, "16")}
            {confirm_label}
        </button>
    }
    .into_any();
    modal_dialog(
        Modal {
            id: dialog_id,
            title: Signal::derive(move || title.to_owned()),
            danger: true,
            width: Width::Narrow,
            body,
            actions,
        },
        open,
        on_cancel,
    )
}
