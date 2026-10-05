// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The wordmark: a small inline-SVG mark (a stylized node tree — the
//! decomposed-document storage model) plus the product name. Pure SVG,
//! no asset fetch, renders identically on both targets.

use leptos::prelude::*;

/// The viewer wordmark (mark + name), sized for the topbar.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn Wordmark() -> impl IntoView {
    view! {
        <span class="flex items-center gap-2">
            <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true" class="text-accent">
                <g fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
                    <circle cx="12" cy="5" r="2.2" fill="currentColor" stroke="none" />
                    <circle cx="6" cy="18" r="2.2" fill="currentColor" stroke="none" />
                    <circle cx="18" cy="18" r="2.2" fill="currentColor" stroke="none" />
                    <path d="M12 7.5v4m0 0-4.5 4m4.5-4 4.5 4" />
                </g>
            </svg>
            <span class="text-base font-semibold tracking-tight text-ink">
                "FerroEHR" <span class="text-accent">" Viewer"</span>
            </span>
        </span>
    }
}
