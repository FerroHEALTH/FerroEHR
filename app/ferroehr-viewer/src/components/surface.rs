// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The card surface classes: the design-system replacement for widget-kit
//! cards in static chrome.
//!
//! One look — token colors, hairline border, the single soft shadow level.
//! [`titled_card`] is the assembled panel card every operational screen builds
//! its sections from.

use leptos::prelude::*;

/// The card surface (no padding — content decides).
pub const CARD: &str = "rounded-card border border-edge bg-raised shadow-card";

/// The card surface with the standard padding.
pub const CARD_PAD: &str = "rounded-card border border-edge bg-raised shadow-card p-4";

/// A sunken well (code panes, read-only documents).
pub const WELL: &str = "rounded-card border border-edge bg-sunken p-3";

/// The standard section heading inside a card.
pub const CARD_TITLE: &str = "text-sm font-semibold text-ink mb-3";

/// A titled panel card wrapping an already-erased body: the uniform section of
/// the two-column operational panels (`/system`, `/operations`). `full_width`
/// spans both grid columns.
#[must_use]
pub fn titled_card(title: &'static str, full_width: bool, body: AnyView) -> AnyView {
    let class = if full_width {
        format!("{CARD_PAD} lg:col-span-2")
    } else {
        CARD_PAD.to_owned()
    };
    view! {
        <section class=class>
            <h2 class=CARD_TITLE>{title}</h2>
            {body}
        </section>
    }
    .into_any()
}
