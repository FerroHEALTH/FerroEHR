// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer's mark and lockup.
//!
//! A small inline-SVG mark (a stylized node tree, the decomposed-document
//! storage model) beside the product name, drawn by FerroUI's lockup. Pure SVG,
//! no asset fetch, the same in both halves.

use ferrohealth_ui::view::shell::lockup;
use leptos::prelude::*;

/// The viewer's mark, decorative, in the accent.
#[must_use]
pub fn mark() -> impl IntoView {
    view! {
        <span class="fui-mark" aria-hidden="true">
            <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true" focusable="false">
                <g fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
                    <circle cx="12" cy="5" r="2.2" fill="currentColor" stroke="none" />
                    <circle cx="6" cy="18" r="2.2" fill="currentColor" stroke="none" />
                    <circle cx="18" cy="18" r="2.2" fill="currentColor" stroke="none" />
                    <path d="M12 7.5v4m0 0-4.5 4m4.5-4 4.5 4" />
                </g>
            </svg>
        </span>
    }
}

/// The lockup that leads to the dashboard: the mark beside "FerroEHR Viewer".
#[must_use]
pub fn viewer_lockup() -> impl IntoView {
    lockup("/", mark(), "FerroEHR", "Viewer")
}
