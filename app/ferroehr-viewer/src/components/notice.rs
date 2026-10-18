// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The inline notice kit: the small alert, status and diagnostic blocks a
//! screen renders BESIDE the thing they are about, each FerroUI's notice
//! (`ferrohealth_ui::view::notice`) in its tone.
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

use ferrohealth_ui::tone::Tone;
use ferrohealth_ui::view::notice::{Announce, notice};
use leptos::prelude::*;

use crate::components::surface::WELL;
use crate::error::ViewerError;

/// One domain error rendered as the standard inline error bar.
///
/// Used by data sections that resolve their `Result` inside `<Suspense>`
/// (SSR'd `ErrorBoundary` fallbacks mismatch at hydration in leptos 0.8, so
/// sections render content-or-this directly; errors never render as nothing).
/// FerroUI's notice keeps line breaks, because a CDR diagnostic carrying
/// `validationErrors` puts one violation per line.
#[must_use]
pub fn inline_error(error: &ViewerError) -> AnyView {
    notice(Tone::Bad, Announce::Alert, None, error.to_string()).into_any()
}

/// A danger-toned note beside the control it is about: a client-side
/// validation complaint, or a refusal rendered as actionable copy.
///
/// `id` is the block's stable E2E hook.
#[must_use]
pub fn alert_note(id: &'static str, message: String) -> AnyView {
    notice(Tone::Bad, Announce::Alert, Some(id), message).into_any()
}

/// The whole-section notice that an object's current version is logically
/// deleted — a first-class state, so it is `role="status"`, not an alert.
#[must_use]
pub fn deleted_notice(id: &'static str, message: &'static str) -> AnyView {
    notice(Tone::Warn, Announce::Status, Some(id), message).into_any()
}

/// The whole-section notice that the CDR holds no object with this id.
#[must_use]
pub fn missing_notice(id: &'static str, message: &'static str) -> AnyView {
    notice(Tone::Bad, Announce::Alert, Some(id), message).into_any()
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

/// The CDR's own diagnostic for a failed write as a notice, inline beside the
/// failure toast.
///
/// Nothing renders while `error` is `None`, so the notice occupies no space
/// until a write is actually refused.
#[must_use]
pub fn failure_bar(error: Signal<Option<ViewerError>>) -> AnyView {
    view! {
        {move || {
            error
                .get()
                .map(|error| {
                    view! {
                        <div class="mt-2">
                            {notice(Tone::Bad, Announce::Alert, None, error.to_string())}
                        </div>
                    }
                })
        }}
    }
    .into_any()
}
