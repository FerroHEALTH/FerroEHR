// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shared empty state: icon + explanation + the action that fills the
//! void — data regions must never render as bare muted text.

use leptos::prelude::*;
use leptos_icons::Icon;

/// A friendly, actionable empty state for a data region.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn EmptyState(
    /// The Lucide icon summarizing the region.
    icon: icondata_core::Icon,
    /// What is empty and why ("No templates yet").
    #[prop(into)]
    message: String,
    /// How to fill it ("Upload your first OPT to get started").
    #[prop(optional, into)]
    hint: Option<String>,
    /// Optional action (a button or link view).
    #[prop(optional)]
    children: Option<Children>,
) -> impl IntoView {
    view! {
        <div class="flex flex-col items-center justify-center gap-2 rounded-card border border-dashed border-edge-strong px-6 py-10 text-center">
            <span class="flex h-10 w-10 items-center justify-center rounded-full bg-sunken text-ink-faint">
                <Icon icon width="20" height="20" />
            </span>
            <p class="text-sm font-medium text-ink">{message}</p>
            {hint
                .map(|h| {
                    view! { <p class="text-sm text-ink-muted">{h}</p> }
                })}
            <div class="mt-1">{children.map(|c| c())}</div>
        </div>
    }
}
