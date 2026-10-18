// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The dashboard stat tile: icon, tabular-nums value, muted label. FerroEHR
//! keeps its own tile until FerroUI's stat card takes a label read at run time
//! and this look (FerroHEALTH/FerroUI#35).

use leptos::prelude::*;
use ferrohealth_ui::icon::{Glyph, icon as glyph_icon};

/// One dashboard metric tile.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn StatCard(
    /// The metric label ("EHRs", "Compositions", …).
    #[prop(into)]
    label: String,
    /// The metric value, already formatted.
    #[prop(into)]
    value: Signal<String>,
    /// The Lucide glyph for the metric.
    icon: Glyph,
    /// Optional link target — the whole tile navigates when set.
    #[prop(optional, into)]
    href: Option<String>,
) -> impl IntoView {
    let inner = view! {
        <div class="flex items-center gap-3">
            <span class="flex h-10 w-10 shrink-0 items-center justify-center rounded-control bg-accent-subtle text-accent-ink">
                {glyph_icon(icon, "20")}
            </span>
            <div class="min-w-0">
                <div class="text-2xl font-semibold tabular-nums text-ink">
                    {move || value.get()}
                </div>
                <div class="truncate text-sm text-ink-muted">{label}</div>
            </div>
        </div>
    }
    .into_any();
    match href {
        Some(href) => view! {
            <a
                href=href
                class="block rounded-card border border-edge bg-raised p-4 shadow-card transition-colors hover:border-accent"
            >
                {inner}
            </a>
        }
        .into_any(),
        None => view! { <div class="rounded-card border border-edge bg-raised p-4 shadow-card">{inner}</div> }
        .into_any(),
    }
}
