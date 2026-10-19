// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The URL-driven tab strip: FerroUI's keyed link tabs
//! (`ferrohealth_ui::view::tabs::keyed_tabs`).
//!
//! A detail screen's tabs are `?tab=` links, not client state, so a tab is
//! shareable and refresh-safe and the strip works before the WASM bundle loads
//! — the router intercepts the anchors once it does. Each screen names its own
//! tabs and the route they hang off; the strip, its look and its current mark
//! are FerroUI's.

use ferrohealth_ui::view::tabs::{KeyedTab, keyed_tabs};
use leptos::prelude::*;

/// The tab strip named `label` over `tabs` (each its key and its words), each
/// leading to `href` of its key, the one whose key `current` reads marked.
///
/// `href` reads the screen's own id signal, so the strip follows a client-side
/// navigation to another object of the same screen.
#[must_use]
pub fn url_tabs(
    label: &'static str,
    tabs: &'static [(&'static str, &'static str)],
    href: impl Fn(&'static str) -> String + Send + Sync + 'static,
    current: Signal<String>,
) -> AnyView {
    // The strip keeps one spacing step from the panel it switches.
    view! {
        <div class="mb-4">
            {move || {
                let tabs = tabs
                    .iter()
                    .map(|(key, words)| KeyedTab {
                        key,
                        href: href(key),
                        label: words,
                    })
                    .collect();
                keyed_tabs(label, tabs, &current.get())
            }}
        </div>
    }
    .into_any()
}
