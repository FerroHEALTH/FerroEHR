// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The inline notice kit: the small alert, status and diagnostic blocks a
//! screen renders BESIDE the thing they are about.
//!
//! Five shapes, each with exactly one definition so a failed read, a refusal,
//! an absence and a rejected write look the same wherever they land: the
//! general "this section's read failed" bar ([`inline_error`]), the alert note
//! beside a control ([`alert_note`]), the two whole-section notices that say an
//! object is deleted or unknown ([`deleted_notice`], [`missing_notice`]), the
//! CDR's verbatim diagnostic under the form it refused ([`diagnostic_pane`]),
//! and the message bar a failed write shows beside its failure toast
//! ([`failure_bar`]).
//!
//! The viewer's feedback rule decides which one applies: a pure READ renders
//! inline and never toasts; a MUTATION toasts on both outcomes and may keep
//! the detail inline beside the toast.

use leptos::prelude::*;

use crate::components::surface::WELL;
use crate::error::ViewerError;

/// The class set of a danger-toned inline block (a refusal, a validation
/// complaint).
const ALERT: &str =
    "rounded-control border border-danger/40 bg-danger-subtle px-3 py-2 text-sm text-danger";

/// The class set of a whole-section notice that something is absent.
const MISSING: &str =
    "rounded-card border border-danger/40 bg-danger-subtle px-3 py-2 text-sm text-danger";

/// The class set of a whole-section notice that something is deleted.
const DELETED: &str =
    "rounded-card border border-warn/40 bg-warn-subtle px-3 py-2 text-sm text-warn";

/// One domain error rendered as the standard inline error bar.
///
/// Used by data sections that resolve their `Result` inside `<Suspense>`
/// (SSR'd `ErrorBoundary` fallbacks mismatch at hydration in leptos 0.8, so
/// sections render content-or-this directly; errors never render as nothing).
/// Newlines survive (`whitespace-pre-line`), because a CDR diagnostic carrying
/// `validationErrors` puts one violation per line.
#[must_use]
pub fn inline_error(error: &ViewerError) -> AnyView {
    let message = error.to_string();
    view! {
        <div
            role="alert"
            class="rounded-control border border-danger/40 bg-danger-subtle px-3 py-2 text-sm text-danger whitespace-pre-line"
        >
            {message}
        </div>
    }
    .into_any()
}

/// A danger-toned note beside the control it is about: a client-side
/// validation complaint, or a refusal rendered as actionable copy.
///
/// `id` is the block's stable E2E hook.
#[must_use]
pub fn alert_note(id: &'static str, message: String) -> AnyView {
    view! {
        <p role="alert" id=id class=ALERT>
            {message}
        </p>
    }
    .into_any()
}

/// The whole-section notice that an object's current version is logically
/// deleted — a first-class state, so it is `role="status"`, not an alert.
#[must_use]
pub fn deleted_notice(id: &'static str, message: &'static str) -> AnyView {
    view! {
        <div role="status" id=id class=DELETED>
            {message}
        </div>
    }
    .into_any()
}

/// The whole-section notice that the CDR holds no object with this id.
#[must_use]
pub fn missing_notice(id: &'static str, message: &'static str) -> AnyView {
    view! {
        <div role="alert" id=id class=MISSING>
            {message}
        </div>
    }
    .into_any()
}

/// The CDR's own diagnostic for a refused write, verbatim, in a scrollable
/// well under the form it refused.
///
/// The toast is the notification; this is the detail worth reading line by
/// line, which is why the text is never truncated or reworded.
#[must_use]
pub fn diagnostic_pane(id: &'static str, error: Signal<Option<ViewerError>>) -> AnyView {
    view! {
        {move || match error.get() {
            Some(error) => {
                let detail = error.to_string();
                view! {
                    <div class=WELL id=id role="alert">
                        <pre class="overflow-auto max-h-[40vh] whitespace-pre-wrap font-mono text-xs text-danger">
                            {detail}
                        </pre>
                    </div>
                }
                    .into_any()
            }
            None => ().into_any(),
        }}
    }
    .into_any()
}

/// The CDR's own diagnostic for a failed write as a message bar, inline beside
/// the failure toast.
///
/// Nothing renders while `error` is `None`, so the bar occupies no space until
/// a write is actually refused.
#[must_use]
pub fn failure_bar(error: Signal<Option<ViewerError>>) -> AnyView {
    view! {
        {move || {
            error
                .get()
                .map(|error| {
                    view! {
                        <div class="mt-2">
                            <thaw::MessageBar intent=thaw::MessageBarIntent::Error>
                                <thaw::MessageBarBody>{error.to_string()}</thaw::MessageBarBody>
                            </thaw::MessageBar>
                        </div>
                    }
                })
        }}
    }
    .into_any()
}
