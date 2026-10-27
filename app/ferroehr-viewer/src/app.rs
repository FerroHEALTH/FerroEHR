// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The HTML document the server draws, and the route tree.
//!
//! The server draws the first page whole and the browser hydrates it, then
//! draws every page after it itself, each group of pages a lazy route
//! ([`crate::routes`]), so a link or a `GET` form loads no new document
//! (FerroUI `docs/consoles.md`, "How a console builds"). A post submits as
//! the browser does and the server draws the page it was posted to, so every
//! page works with no WebAssembly. The document carries the reader's
//! preferences as `data-theme`, `data-density`, `data-motion` and
//! `data-hints` and the page's language as `lang`, so the first paint is
//! right with no script. `/login` is public; every other page is drawn in
//! the shell ([`crate::shell::AppShell`]), drawn once. Each page sets a bare
//! `<Title>`, and `viewer_title` adds the product name once. No openEHR spec
//! governs the viewer: our own design.

use ferrohealth_ui::island::{FormNavigator, PageAnnouncer, ToastRegion};
use ferrohealth_ui::widget::toast::RegionWords;
use leptos::prelude::*;
#[cfg(not(target_arch = "wasm32"))]
use leptos_meta::MetaTags;
use leptos_meta::{Title, provide_meta_context};
use leptos_router::components::{ParentRoute, Route, Router, Routes};
use leptos_router::{SsrMode, path};

use crate::locale::{Msg, tr};

/// The path the server serves FerroUI's stylesheet at.
pub const STYLESHEET_PATH: &str = "/ferrohealth-ui.css";

/// The path the server serves FerroUI's FerroEHR theme file at.
pub const THEME_PATH: &str = "/theme.css";

/// The path the FerroEHR mark is served at as the tab icon.
pub const ICON_PATH: &str = "/favicon.svg";

/// The whole document around [`App`], in the reader's theme, density,
/// motion and hint preferences and in the page's language.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn document(options: LeptosOptions) -> impl IntoView {
    let preferences = crate::prefs::current();
    let lang = crate::locale::current().tag();
    view! {
        <!DOCTYPE html>
        <html
            lang=lang
            data-theme=preferences.theme.key()
            data-density=preferences.density.key()
            data-motion=preferences.motion.key()
            data-hints=preferences.hints.key()
        >
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <link rel="icon" type="image/svg+xml" href=ICON_PATH />
                <link rel="stylesheet" href=STYLESHEET_PATH />
                <link rel="stylesheet" href=THEME_PATH />
                <HydrationScripts options />
                <MetaTags />
            </head>
            <body>
                <App />
            </body>
        </html>
    }
}

/// The product name as a reader sees it.
#[must_use]
pub fn product() -> String {
    tr(Msg::ProductName)
}

/// The document title of a page whose bare title is `section`: the section,
/// then the product name, once.
fn viewer_title(mut section: String) -> String {
    let product = product();
    if section.is_empty() || section == product {
        return product;
    }
    section.push_str(" · ");
    section.push_str(&product);
    section
}

/// The route tree, the one title format, the toast region and the page
/// announcer.
///
/// The server draws it and the browser hydrates it whole, both from the same
/// [`crate::request::Visit`], which the browser asks for again after each
/// navigation ([`crate::request::follow_address`]).
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();
    // NOTE: the Leptos book, `ssr/22_life_cycle`: what the server read of the request reaches a
    // hydrating page serialized, so both halves draw the same page.
    crate::request::provide_visit();
    crate::posted::provide_posted();
    ferrohealth_ui::i18n::provide_locale(crate::locale::current());
    ferrohealth_ui::widget::prefetch::routes(crate::routes::ROUTES);
    let flash = crate::flash::shared();
    view! {
        <Title formatter=viewer_title text=product() />
        <Router>
            <FormNavigator />
            <Routes fallback=|| view! { <crate::routes::Pages /> }>
                <Route path=path!("/login") view=crate::routes::Pages ssr=SsrMode::Async />
                <ParentRoute path=path!("") view=crate::shell::AppShell ssr=SsrMode::Async>
                    <Route path=path!("") view=crate::routes::Pages />
                    <Route path=path!("*any") view=crate::routes::Pages />
                </ParentRoute>
            </Routes>
        </Router>
        <PageAnnouncer />
        <ToastRegion words=RegionWords::of_page() flash=flash />
    }
}

#[cfg(test)]
mod tests {
    use super::viewer_title;

    #[test]
    fn a_section_name_gains_the_product_suffix_once() {
        let titled = viewer_title("Templates".to_owned());
        assert!(titled.starts_with("Templates · "), "{titled}");
        assert_eq!(1, titled.matches(" · ").count(), "{titled}");
        assert_eq!(
            viewer_title(String::new()),
            viewer_title(viewer_title(String::new()))
        );
    }
}
