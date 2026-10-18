// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The page header every routed screen opens with.
//!
//! FerroUI's (`ferrohealth_ui::view::surface::page_heading_live`), with the
//! breadcrumb trail, the title, the subtitle and the screen's primary actions,
//! so the page rhythm (header → toolbar → content) is identical across the
//! viewer.

use ferrohealth_ui::view::surface::{Header, page_heading_live};
use leptos::prelude::*;

/// One breadcrumb ancestor (the current page renders as plain text after
/// the linked trail).
#[derive(Debug, Clone)]
pub struct Crumb {
    /// The link label.
    pub label: String,
    /// The route the crumb navigates to.
    pub href: String,
}

impl Crumb {
    /// Convenience constructor.
    pub fn new(label: impl Into<String>, href: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            href: href.into(),
        }
    }
}

/// The standard page header.
///
/// `crumbs` are the ANCESTORS only — the header renders `title` as the
/// terminal crumb itself. `children` (optional) land right-aligned on the
/// title row: the screen's primary actions, built once.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn PageHeader(
    /// The page title (also the terminal breadcrumb).
    #[prop(into)]
    title: Signal<String>,
    /// One-line description under the title.
    #[prop(optional, into)]
    subtitle: Option<String>,
    /// Ancestor breadcrumbs, root first.
    #[prop(optional)]
    crumbs: Vec<Crumb>,
    /// Right-aligned primary actions.
    #[prop(optional)]
    children: Option<Children>,
    /// Render the title in a monospace face (ids, template names).
    #[prop(optional)]
    mono: bool,
) -> impl IntoView {
    let crumbs: Vec<ferrohealth_ui::view::surface::Crumb> = crumbs
        .into_iter()
        .map(|crumb| ferrohealth_ui::view::surface::Crumb::new(crumb.label, crumb.href))
        .collect();
    page_heading_live(
        Header {
            title: String::new(),
            subtitle,
            crumbs,
            mono,
        },
        title,
        children.map(|children| children()),
    )
}
