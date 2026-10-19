// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The empty state: FerroUI's, with the explanation and the action that fills
//! the void — data regions must never render as bare muted text.

use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::empty::empty_state;
use leptos::prelude::*;

/// A friendly, actionable empty state for a data region.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn EmptyState(
    /// The Lucide glyph summarizing the region.
    icon: Glyph,
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
    empty_state(
        icon,
        message,
        hint.unwrap_or_default(),
        children.map(|children| children()),
    )
}
