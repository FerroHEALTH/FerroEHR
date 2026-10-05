// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shared page header: breadcrumbs, title, subtitle, and an action
//! slot — every routed screen opens with one so the page rhythm (header →
//! toolbar → content) is identical across the viewer.

use leptos::prelude::*;
use leptos_router::components::A;

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
/// `crumbs` are the ANCESTORS only — the component renders `title` as the
/// terminal crumb itself. `children` (optional) land right-aligned on the
/// title row: the screen's primary actions.
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
    let trail = (!crumbs.is_empty()).then(|| {
        let links = crumbs
            .into_iter()
            .map(|crumb| {
                view! {
                    <li class="flex items-center gap-1.5">
                        <A
                            href=crumb.href
                            attr:class="text-ink-muted hover:text-accent hover:underline"
                        >
                            {crumb.label}
                        </A>
                        <span aria-hidden="true" class="text-ink-faint">
                            "/"
                        </span>
                    </li>
                }
            })
            .collect_view();
        view! {
            <nav aria-label="Breadcrumb" class="mb-1">
                <ol class="flex flex-wrap items-center gap-1.5 text-sm">
                    {links} <li aria-current="page" class="text-ink-muted">
                        {move || title.get()}
                    </li>
                </ol>
            </nav>
        }
    });
    let title_class = if mono {
        "text-xl font-semibold font-mono text-ink"
    } else {
        "text-xl font-semibold text-ink"
    };
    view! {
        <header class="mb-6">
            {trail} <div class="flex flex-wrap items-center justify-between gap-3">
                <h1 class=title_class>{move || title.get()}</h1>
                <div class="flex items-center gap-2">{children.map(|c| c())}</div>
            </div>
            {subtitle
                .map(|s| {
                    view! { <p class="mt-1 text-sm text-ink-muted">{s}</p> }
                })}
        </header>
    }
}
