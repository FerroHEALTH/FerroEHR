// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The URL-driven tab strip's one pill link.
//!
//! A detail screen's tabs are `?tab=` links, not client state, so a tab is
//! shareable and refresh-safe and the strip works before the WASM bundle loads
//! — the router intercepts the anchors once it does. Each screen still spells
//! its own strip, because the tabs it offers and the route they hang off are
//! its own; what is shared is one tab's look and its selected/idle split.

use leptos::prelude::*;

/// The selected tab pill.
const TAB_ON: &str =
    "rounded-control px-3 py-1.5 text-sm font-medium bg-accent-subtle text-accent-ink";

/// An unselected tab pill.
const TAB_OFF: &str =
    "rounded-control px-3 py-1.5 text-sm font-medium text-ink-muted hover:bg-sunken";

/// One tab of a URL-driven tab strip: a plain anchor to `href`, tinted while
/// `active`.
///
/// `href` is a closure because the route it points at usually reads the
/// screen's own id signal; `active` is the screen's `?tab=` comparison.
#[must_use]
pub fn tab_link(
    href: impl Fn() -> String + Send + Sync + 'static,
    label: &'static str,
    active: Signal<bool>,
) -> AnyView {
    let class = move || if active.get() { TAB_ON } else { TAB_OFF };
    view! {
        <a href=href class=class>
            {label}
        </a>
    }
    .into_any()
}
