// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The root Leptos application: the HTML document shell, the theme provider,
//! and the route tree.
//!
//! `/login` is public; every other screen is nested under the session-guarded
//! [`crate::pages::shell::AppShell`] layout, which renders the matched child
//! through its `<Outlet/>`.
//!
//! Every routed view sets a `<Title/>` carrying its BARE section name; the
//! product suffix is appended in exactly one place, `viewer_title`, through
//! the root `<Title formatter=…/>`.

use leptos::prelude::*;
use leptos_meta::{HashedStylesheet, MetaTags, Title, provide_meta_context};
use leptos_router::components::{ParentRoute, Route, Router, Routes};
use leptos_router::path;

/// The full HTML document shell rendered by the server.
///
/// The stylesheet link is emitted HERE, not from a component body, because it
/// is the one `<head>` element whose href depends on the build's content
/// hashes: `HashedStylesheet` reads the cargo-leptos hash manifest through
/// [`LeptosOptions`], which exists only on the server
/// (<https://docs.rs/leptos_meta/0.8/leptos_meta/fn.HashedStylesheet.html>).
#[must_use]
pub fn shell(options: LeptosOptions) -> impl IntoView {
    // The theme the request's cookie names, written on <html> so the first
    // paint is already in it (no script, no flash).
    let theme = crate::theme::current_theme();
    view! {
        <!DOCTYPE html>
        // The viewer ships one locale, and its copy is written in English, so
        // `en` is the honest declaration rather than a placeholder — an
        // assistive technology must be told which language it is reading.
        // TODO(#300): drive lang from the active locale when a second locale lands.
        <html lang="en" data-theme=theme.key()>
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <AutoReload options=options.clone() />
                <HydrationScripts options=options.clone() />
                // The viewer's own utilities first, then FerroUI's stylesheet,
                // whose base layer then wins over Tailwind's preflight, then
                // the FerroEHR brand theme over FerroUI's roles.
                <HashedStylesheet options id="leptos" />
                <MetaTags />
                <link rel="stylesheet" href=STYLESHEET_PATH />
                <link rel="stylesheet" href=THEME_PATH />
            </head>
            <body>
                <App />
            </body>
        </html>
    }
}

/// The product name as a human reads it — the viewer's own display name.
///
/// Lowercase `ferroehr-viewer` stays reserved for technical identifiers (the
/// crate, the asset paths).
const PRODUCT: &str = "FerroEHR Viewer";

/// The path the server serves FerroUI's stylesheet at.
pub const STYLESHEET_PATH: &str = "/ferrohealth-ui.css";

/// The path the server serves the FerroEHR brand theme at.
pub const THEME_PATH: &str = "/theme.css";

/// Formats one screen's section name into the document title.
///
/// Every routed screen sets a BARE section name (`"Templates"`), and this is
/// the single place the product name is appended — so a new screen cannot
/// forget the suffix and no screen can spell it differently. The product name
/// itself passes through unchanged rather than doubling: it is the root
/// `<Title/>`'s own text, shown until a screen pushes its section.
///
/// The owned `String` argument is `leptos_meta::Formatter`'s own signature
/// (`Fn(String) -> String`), so the suffix is appended in place.
fn viewer_title(mut section: String) -> String {
    if section.is_empty() || section == PRODUCT {
        return PRODUCT.to_owned();
    }
    section.push_str(" · ");
    section.push_str(PRODUCT);
    section
}

/// Marks the page hydrated: `data-hydrated` on `<body>` for the E2E harness,
/// and the page-wide [`Hydrated`](crate::components::field::Hydrated) the
/// code editors read.
fn mark_hydration() {
    // Hydration-completion marker: effects run only in the browser (Leptos
    // book, reactivity/14), so `data-hydrated` lands on `<body>` exactly when
    // the client runtime is live and hydrated listeners exist. The E2E
    // harness waits on it before driving hydration-dependent controls — a
    // file set on the upload input BEFORE its listener exists fires no later
    // event. Set post-hydration, it cannot mismatch the SSR document.
    let hydrated = RwSignal::new(false);
    provide_context(crate::components::field::Hydrated {
        signal: hydrated,
        no_marks: Signal::stored(Vec::new()),
        owner: Owner::current(),
    });
    Effect::new(move |_| {
        if let Some(body) = document().body() {
            drop(body.set_attribute("data-hydrated", "true"));
        }
        hydrated.set(true);
    });
}

/// The root component: the meta context and the router.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();
    mark_hydration();
    view! {
        <Title formatter=viewer_title text=PRODUCT />
            <Router>
                <Routes fallback=|| view! { <NotFound /> }>
                    // NOTE: /login uses SsrMode::Async so the sign-in form
                    // works with JavaScript disabled (Leptos book, ssr/23
                    // "Async Rendering": "Works if JavaScript is disabled").
                    <Route
                        path=path!("/login")
                        view=crate::pages::login::LoginPage
                        ssr=leptos_router::SsrMode::Async
                    />
                    // NOTE: every authenticated screen uses SsrMode::Async on
                    // the PARENT route — streamed fragments race WASM init and
                    // crash hydration (Leptos book, ssr/23 "Async Rendering").
                    <ParentRoute
                        path=path!("")
                        view=crate::pages::shell::AppShell
                        ssr=leptos_router::SsrMode::Async
                    >
                        <Route path=path!("") view=crate::pages::dashboard::DashboardPage />
                        <Route
                            path=path!("templates")
                            view=crate::pages::templates::TemplatesPage
                        />
                        <Route
                            path=path!("templates/adl2/:template_id")
                            view=crate::pages::template_adl2::Adl2TemplateDetailPage
                        />
                        <Route
                            path=path!("templates/:template_id")
                            view=crate::pages::template_detail::TemplateDetailPage
                        />
                        <Route path=path!("queries") view=crate::pages::queries::QueriesPage />
                        <Route
                            path=path!("queries/builder")
                            view=crate::pages::query_builder::QueryBuilderPage
                        />
                        <Route
                            path=path!("queries/aql")
                            view=crate::pages::query_aql::QueryAqlPage
                        />
                        <Route
                            path=path!("queries/stored")
                            view=crate::pages::query_stored::QueryStoredPage
                        />
                        <Route path=path!("ehrs") view=crate::pages::ehrs::EhrsPage />
                        <Route
                            path=path!("ehrs/:ehr_id")
                            view=crate::pages::ehr_detail::EhrDetailPage
                        />
                        <Route
                            path=path!("ehrs/:ehr_id/compositions/:uid")
                            view=crate::pages::composition::CompositionPage
                        />
                        // The demographic routes are declared BEFORE the
                        // `:kind` ones on purpose: leptos_router matches a
                        // route tuple in declaration order and takes the first
                        // hit, so `relationship`/`contribution` would otherwise
                        // be read as a party kind.
                        <Route
                            path=path!("demographics")
                            view=crate::pages::demographics::browse::DemographicsPage
                        />
                        <Route
                            path=path!("demographics/relationship")
                            view=crate::pages::demographics::relationship::RelationshipsPage
                        />
                        <Route
                            path=path!("demographics/relationship/:uid")
                            view=crate::pages::demographics::relationship::RelationshipDetailPage
                        />
                        <Route
                            path=path!("demographics/contribution/:uid")
                            view=crate::pages::demographics::contribution::DemographicContributionPage
                        />
                        <Route
                            path=path!("demographics/:kind")
                            view=crate::pages::demographics::browse::PartyBrowserPage
                        />
                        <Route
                            path=path!("demographics/:kind/:uid")
                            view=crate::pages::demographics::party::PartyDetailPage
                        />
                        <Route
                            path=path!("terminology")
                            view=crate::pages::terminology::TerminologyPage
                        />
                        <Route path=path!("audit") view=crate::pages::audit::AuditPage />
                        <Route path=path!("system") view=crate::pages::system::SystemPage />
                        <Route
                            path=path!("subscriptions")
                            view=crate::pages::subscriptions::SubscriptionsPage
                        />
                        <Route
                            path=path!("operations")
                            view=crate::pages::operations::OperationsPage
                        />
                    </ParentRoute>
                </Routes>
            </Router>
    }
}

/// The router's real 404 fallback: sets a distinct title and offers a link
/// back to the dashboard.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn NotFound() -> impl IntoView {
    let content = ferrohealth_ui::view::surface::panel(ferrohealth_ui::view::empty::empty_state(
        ferrohealth_ui::icon::Glyph::COMPASS,
        "Page not found",
        "The page you requested does not exist.",
        ferrohealth_ui::view::button::link_button(
            "/".to_owned(),
            ferrohealth_ui::view::button::Variant::Primary,
            None,
            "Return to the dashboard",
        ),
    ))
    .into_any();
    view! {
        <Title text="Not found" />
        {ferrohealth_ui::view::shell::centred(
            content,
            ferrohealth_ui::view::shell::Footer {
                product: PRODUCT,
                version: env!("CARGO_PKG_VERSION"),
                about: "/system#manufacturer",
            },
        )}
    }
}

#[cfg(test)]
mod tests {
    use super::{PRODUCT, viewer_title};

    #[test]
    fn a_section_name_gains_the_product_suffix() {
        assert_eq!(
            viewer_title("Templates".to_owned()),
            "Templates · FerroEHR Viewer"
        );
        assert_eq!(
            viewer_title("Not found".to_owned()),
            "Not found · FerroEHR Viewer"
        );
        // The per-object screens format their own prefix; the suffix still
        // lands exactly once, at the end.
        assert_eq!(
            viewer_title("Template · vitals.v1".to_owned()),
            "Template · vitals.v1 · FerroEHR Viewer"
        );
    }

    /// `leptos_meta` applies the formatter to whatever text is on top of the
    /// title stack — including the root `<Title/>`'s own product name, which
    /// is what the viewer shows before a screen pushes its section. Suffixing
    /// that would read `FerroEHR Viewer · FerroEHR Viewer`.
    #[test]
    fn the_product_name_is_never_suffixed_with_itself() {
        assert_eq!(viewer_title(PRODUCT.to_owned()), PRODUCT);
        assert_eq!(viewer_title(String::new()), PRODUCT);
    }
}
