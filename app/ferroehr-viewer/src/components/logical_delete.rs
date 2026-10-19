// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The **logical delete** affordance a versioned object's detail screen carries
//! above its tabs.
//!
//! openEHR's delete is logical: it commits a `523|deleted|` version, so the
//! object stops resolving as current while every earlier version stays readable
//! by its own uid (RM common master06 §Logical Deletion). Every screen that
//! offers it does the same four things — a danger button, the shared
//! confirmation modal, both outcome toasts, and a return to the listing the
//! object came from — so they are spelled once here. The caller owns the
//! `Action` (each family deletes through its own server function) and the
//! consequence copy naming its object.

use leptos::prelude::*;

use crate::components::confirm_dialog::ConfirmDialog;
use crate::components::field::BTN_DANGER;
use crate::components::toast::{toast_error, toast_success};
use crate::error::ViewerError;
use crate::locale::{Msg, tr};

/// The copy and DOM hooks one family's delete affordance renders with.
#[derive(Debug, Clone, Copy)]
pub struct LogicalDeleteCopy {
    /// The trigger button's DOM id — the stable E2E hook.
    pub button_id: &'static str,
    /// The action's label, used for the button, the dialog title and the
    /// confirming button alike ("Delete party").
    pub label: Msg,
    /// The confirmation dialog's DOM id.
    pub dialog_id: &'static str,
    /// The confirming button's DOM id.
    pub confirm_id: &'static str,
    /// The success toast's title.
    pub success_title: Msg,
    /// The noun phrase the failure copy is built around ("this party").
    pub object: Msg,
}

/// The delete button plus its confirmation modal and outcome toasts.
///
/// `delete` is dispatched with `version_uid` — the version to SUPERSEDE, which
/// the screen's one read of the object publishes — and `message` is the
/// consequence copy the modal states. On success the viewer toasts and
/// navigates to `return_href`; on failure it toasts the CDR's own reason
/// ([`crate::feedback::logical_delete_failure_copy`]).
#[must_use]
pub fn logical_delete_section(
    delete: Action<String, Result<(), ViewerError>>,
    version_uid: RwSignal<String>,
    message: Signal<String>,
    return_href: String,
    copy: LogicalDeleteCopy,
) -> AnyView {
    let toaster = crate::components::toast::toaster();
    let confirming = RwSignal::new(false);
    let navigate = leptos_router::hooks::use_navigate();
    Effect::new(move |_| match delete.value().get() {
        Some(Ok(())) => {
            toast_success(
                toaster,
                &tr(copy.success_title),
                &tr(Msg::DeletedVersionWasCommittedEarlierVersions),
            );
            navigate(&return_href, leptos_router::NavigateOptions::default());
        }
        Some(Err(error)) => toast_error(
            toaster,
            &tr(Msg::DeleteFailed),
            &crate::feedback::logical_delete_failure_copy(&tr(copy.object), &error),
        ),
        None => {}
    });

    view! {
        <div class="contents">
            <button
                id=copy.button_id
                type="button"
                class=BTN_DANGER
                disabled=Signal::derive(move || delete.pending().get())
                on:click=move |_| confirming.set(true)
            >
                {ferrohealth_ui::icon::icon(ferrohealth_ui::icon::Glyph::TRASH, "14")}
                {tr(copy.label)}
            </button>
            <ConfirmDialog
                open=confirming
                title=tr(copy.label)
                message=message
                confirm_label=tr(copy.label)
                dialog_id=copy.dialog_id
                confirm_id=copy.confirm_id
                on_cancel=Callback::new(move |()| confirming.set(false))
                on_confirm=Callback::new(move |()| {
                    delete.dispatch(version_uid.get_untracked());
                    confirming.set(false);
                })
            />
        </div>
    }
    .into_any()
}
