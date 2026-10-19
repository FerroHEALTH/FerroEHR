// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The dashboard stat tile: FerroUI's stat card, the glyph on its tint beside
//! the value and the label under it, the whole card a link when it leads
//! somewhere.

use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::surface::{Stat, stat_card};
use leptos::prelude::*;

/// One dashboard metric tile, its value read as it changes.
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
    move || {
        stat_card(Stat {
            glyph: icon,
            value: value.get(),
            label: label.clone().into(),
            href: href.clone(),
        })
    }
}
