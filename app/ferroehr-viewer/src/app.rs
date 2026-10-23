// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The HTML document the server draws, and the route tree.
//!
//! The viewer renders in Leptos islands mode (FerroUI
//! `crates/ferrohealth-ui/CLAUDE.md`): every page is drawn whole on the
//! server, and the browser hydrates only the islands. Navigation is plain
//! links and forms, with no client router. The document carries the reader's
//! preferences as `data-theme`, `data-density`, `data-motion` and `data-hints`
//! and the page's language as `lang`, so the first paint is right with no
//! script. `/login` is public; every other page is drawn in the shell
//! ([`crate::shell::AppShell`]). Each page sets a bare `<Title>`, and
//! `viewer_title` adds the product name once. No openEHR spec governs the
//! viewer: our own design.

use ferrohealth_ui::island::ToastRegion;
use ferrohealth_ui::widget::toast::RegionWords;
use leptos::prelude::*;
use leptos_meta::{MetaTags, Title, provide_meta_context};
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
                <HydrationScripts options islands=true />
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

/// The route tree, the one title format and the toast region.
///
/// It runs on the server alone: the browser hydrates the islands of a page
/// and nothing else.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the post it answers, the reads it waits on and the view it draws"
)]
#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();
    ferrohealth_ui::i18n::provide_locale(crate::locale::current());
    let flash = crate::flash::take();
    let routes = view! {
        <Routes fallback=crate::pages::errors::not_found_page>
            <Route path=path!("/login") view=crate::pages::login::LoginPage ssr=SsrMode::Async />
            <ParentRoute path=path!("") view=crate::shell::AppShell ssr=SsrMode::Async>
                <Route path=path!("") view=crate::pages::dashboard::DashboardPage />
                <Route path=path!("find") view=crate::pages::find::FindPage />
                <Route path=path!("ehrs") view=crate::pages::ehrs::EhrsPage />
                <Route path=path!("ehrs/new") view=crate::pages::ehrs::NewEhrPage />
                <Route path=path!("ehrs/:ehr_id") view=crate::pages::ehr::compositions::CompositionsTab />
                <Route path=path!("ehrs/:ehr_id/directory") view=crate::pages::ehr::directory::DirectoryTab />
                <Route
                    path=path!("ehrs/:ehr_id/directory/edit")
                    view=crate::pages::ehr::directory::DirectoryEditPage
                />
                <Route path=path!("ehrs/:ehr_id/status") view=crate::pages::ehr::status::StatusTab />
                <Route
                    path=path!("ehrs/:ehr_id/status/edit")
                    view=crate::pages::ehr::status::StatusEditPage
                />
                <Route
                    path=path!("ehrs/:ehr_id/contributions")
                    view=crate::pages::ehr::contribution::ContributionsTab
                />
                <Route
                    path=path!("ehrs/:ehr_id/contributions/:uid")
                    view=crate::pages::ehr::contribution::ContributionPage
                />
                <Route
                    path=path!("ehrs/:ehr_id/contribution/new")
                    view=crate::pages::ehr::contribution::NewContributionPage
                />
                <Route path=path!("ehrs/:ehr_id/tags") view=crate::pages::ehr::tags::TagsTab />
                <Route
                    path=path!("ehrs/:ehr_id/compositions/new")
                    view=crate::pages::composition::NewCompositionPage
                />
                <Route
                    path=path!("ehrs/:ehr_id/compositions/:uid")
                    view=crate::pages::composition::CompositionPage
                />
                <Route
                    path=path!("ehrs/:ehr_id/compositions/:uid/edit")
                    view=crate::pages::composition::EditCompositionPage
                />
                <Route path=path!("parties") view=crate::pages::parties::PartiesPage />
                <Route path=path!("parties/new") view=crate::pages::parties::NewPartyPage />
                <Route
                    path=path!("parties/relationships/new")
                    view=crate::pages::parties::NewRelationshipPage
                />
                <Route
                    path=path!("parties/relationships/:uid")
                    view=crate::pages::parties::RelationshipPage
                />
                <Route
                    path=path!("parties/relationships/:uid/edit")
                    view=crate::pages::parties::EditRelationshipPage
                />
                <Route
                    path=path!("parties/contributions/:uid")
                    view=crate::pages::parties::PartyContributionPage
                />
                <Route path=path!("parties/:kind/:uid") view=crate::pages::parties::PartyPage />
                <Route
                    path=path!("parties/:kind/:uid/edit")
                    view=crate::pages::parties::EditPartyPage
                />
                <Route path=path!("templates") view=crate::pages::templates::TemplatesPage />
                <Route
                    path=path!("templates/upload")
                    view=crate::pages::templates::UploadTemplatePage
                />
                <Route
                    path=path!("templates/adl2/:template_id")
                    view=crate::pages::templates::Adl2TemplatePage
                />
                <Route
                    path=path!("templates/adl2/:template_id/:release")
                    view=crate::pages::templates::Adl2TemplatePage
                />
                <Route path=path!("templates/:template_id") view=crate::pages::templates::TemplatePage />
                <Route path=path!("terminology") view=crate::pages::terminology::TerminologiesTab />
                <Route path=path!("terminology/lookup") view=crate::pages::terminology::LookupTab />
                <Route
                    path=path!("terminology/valuesets")
                    view=crate::pages::terminology::ValueSetsTab
                />
                <Route
                    path=path!("terminology/subsumption")
                    view=crate::pages::terminology::SubsumptionTab
                />
                <Route path=path!("terminology/:id") view=crate::pages::terminology::TerminologyPage />
                <Route path=path!("queries") view=crate::pages::queries::StoredQueriesPage />
                <Route
                    path=path!("queries/:name/versions")
                    view=crate::pages::queries::StoredQueryVersionsTab
                />
                <Route path=path!("queries/:name/:version") view=crate::pages::queries::StoredQueryPage />
                <Route
                    path=path!("queries/:name/:version/run")
                    view=crate::pages::queries::StoredQueryRunTab
                />
                <Route path=path!("query") view=crate::pages::query::BuildTab />
                <Route path=path!("query/aql") view=crate::pages::query::AqlTab />
                <Route path=path!("server") view=crate::pages::server::OverviewTab />
                <Route path=path!("server/health") view=crate::pages::server::HealthTab />
                <Route path=path!("server/metrics") view=crate::pages::server::MetricsTab />
                <Route path=path!("server/logging") view=crate::pages::server::LoggingTab />
                <Route path=path!("server/api") view=crate::pages::server::ApiTab />
                <Route path=path!("server/smart") view=crate::pages::server::SmartTab />
                <Route path=path!("audit") view=crate::pages::audit::AuditPage />
                <Route path=path!("audit/:id") view=crate::pages::audit::AuditRecordPage />
                <Route path=path!("subscriptions") view=crate::pages::subscriptions::SubscriptionsPage />
                <Route
                    path=path!("subscriptions/new")
                    view=crate::pages::subscriptions::NewSubscriptionPage
                />
                <Route
                    path=path!("subscriptions/:name/edit")
                    view=crate::pages::subscriptions::EditSubscriptionPage
                />
                <Route
                    path=path!("configuration")
                    view=crate::pages::configuration::ConfigurationPage
                />
                <Route
                    path=path!("configuration/:section")
                    view=crate::pages::configuration::ConfigurationPage
                />
                <Route path=path!("preferences") view=crate::pages::preferences::PreferencesPage />
                <Route
                    path=path!("preferences/:section")
                    view=crate::pages::preferences::PreferencesPage
                />
                <Route path=path!("access") view=crate::pages::access::AccessPage />
                <Route path=path!("about") view=crate::pages::about::AboutPage />
            </ParentRoute>
        </Routes>
    }
    .into_any();
    view! {
        <Title formatter=viewer_title text=product() />
        <Router>{routes}</Router>
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
