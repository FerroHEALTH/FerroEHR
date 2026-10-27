// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer's pages in lazy groups, one for each part of the navigation a
//! reader reads together, and the sign-in on its own.
//!
//! Under `cargo leptos build --split` each group loads its code from chunks
//! of its own (the Leptos book, "Code Splitting"), so the first page costs
//! the entry and its group. [`ROUTES`] is the one route table: the one
//! route ([`Pages`]) loads the group that holds the page its address
//! names ([`Page::of`]), the group draws that page again as the address
//! changes, after a post the router drew, and when the display cookies the
//! server answers with change, and a page reads the segments its route names
//! through [`params`]. FerroUI loads a group ahead of a visit from the same
//! table (`ferrohealth_ui::widget::prefetch`). One route in place of one per
//! page keeps the router's route tree, which the entry WebAssembly carries
//! whole, to its two routes. No openEHR spec governs the grouping: our own
//! design.

use ferrohealth_ui::widget::prefetch::{Route, matches};
use leptos::prelude::*;
use leptos_router::{LazyRoute, lazy_route};

/// A viewer page, as its route names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// `/`.
    Dashboard,
    /// `/find`.
    Find,
    /// `/ehrs`.
    Ehrs,
    /// `/ehrs/new`.
    NewEhr,
    /// `/ehrs/:ehr_id`, the EHR's compositions.
    Compositions,
    /// `/ehrs/:ehr_id/directory`.
    Directory,
    /// `/ehrs/:ehr_id/directory/edit`.
    EditDirectory,
    /// `/ehrs/:ehr_id/status`.
    Status,
    /// `/ehrs/:ehr_id/status/edit`.
    EditStatus,
    /// `/ehrs/:ehr_id/contributions`.
    Contributions,
    /// `/ehrs/:ehr_id/contributions/:uid`.
    Contribution,
    /// `/ehrs/:ehr_id/contribution/new`.
    NewContribution,
    /// `/ehrs/:ehr_id/tags`.
    Tags,
    /// `/ehrs/:ehr_id/compositions/new`.
    NewComposition,
    /// `/ehrs/:ehr_id/compositions/:uid`.
    Composition,
    /// `/ehrs/:ehr_id/compositions/:uid/edit`.
    EditComposition,
    /// `/parties`.
    Parties,
    /// `/parties/new`.
    NewParty,
    /// `/parties/relationships/new`.
    NewRelationship,
    /// `/parties/relationships/:uid`.
    Relationship,
    /// `/parties/relationships/:uid/edit`.
    EditRelationship,
    /// `/parties/contributions/:uid`.
    PartyContribution,
    /// `/parties/:kind/:uid`.
    Party,
    /// `/parties/:kind/:uid/edit`.
    EditParty,
    /// `/templates`.
    Templates,
    /// `/templates/upload`.
    UploadTemplate,
    /// `/templates/adl2/:template_id` and its release.
    Adl2Template,
    /// `/templates/:template_id`.
    Template,
    /// `/terminology`.
    Terminologies,
    /// `/terminology/lookup`.
    Lookup,
    /// `/terminology/valuesets`.
    ValueSets,
    /// `/terminology/subsumption`.
    Subsumption,
    /// `/terminology/:id`.
    Terminology,
    /// `/queries`.
    StoredQueries,
    /// `/queries/:name/versions`.
    StoredQueryVersions,
    /// `/queries/:name/:version`.
    StoredQuery,
    /// `/queries/:name/:version/run`.
    StoredQueryRun,
    /// `/query`.
    Build,
    /// `/query/aql`.
    Aql,
    /// `/server`.
    ServerOverview,
    /// `/server/health`.
    Health,
    /// `/server/metrics`.
    Metrics,
    /// `/server/logging`.
    Logging,
    /// `/server/api`.
    Api,
    /// `/server/smart`.
    Smart,
    /// `/audit`.
    Audit,
    /// `/audit/:id`.
    AuditRecord,
    /// `/subscriptions`.
    Subscriptions,
    /// `/subscriptions/new`.
    NewSubscription,
    /// `/subscriptions/:name/edit`.
    EditSubscription,
    /// `/configuration` and its sections.
    Configuration,
    /// `/preferences` and its sections.
    Preferences,
    /// `/access`.
    Access,
    /// `/about`.
    About,
    /// `/login`.
    Login,
}

impl Page {
    /// The page whose route is `pattern`, as [`ROUTES`] names it.
    #[must_use]
    pub fn of_pattern(pattern: &str) -> Option<Self> {
        Some(match pattern {
            "/" => Self::Dashboard,
            "/find" => Self::Find,
            "/ehrs" => Self::Ehrs,
            "/ehrs/new" => Self::NewEhr,
            "/ehrs/:ehr_id" => Self::Compositions,
            "/ehrs/:ehr_id/directory" => Self::Directory,
            "/ehrs/:ehr_id/directory/edit" => Self::EditDirectory,
            "/ehrs/:ehr_id/status" => Self::Status,
            "/ehrs/:ehr_id/status/edit" => Self::EditStatus,
            "/ehrs/:ehr_id/contributions" => Self::Contributions,
            "/ehrs/:ehr_id/contributions/:uid" => Self::Contribution,
            "/ehrs/:ehr_id/contribution/new" => Self::NewContribution,
            "/ehrs/:ehr_id/tags" => Self::Tags,
            "/ehrs/:ehr_id/compositions/new" => Self::NewComposition,
            "/ehrs/:ehr_id/compositions/:uid" => Self::Composition,
            "/ehrs/:ehr_id/compositions/:uid/edit" => Self::EditComposition,
            "/parties" => Self::Parties,
            "/parties/new" => Self::NewParty,
            "/parties/relationships/new" => Self::NewRelationship,
            "/parties/relationships/:uid" => Self::Relationship,
            "/parties/relationships/:uid/edit" => Self::EditRelationship,
            "/parties/contributions/:uid" => Self::PartyContribution,
            "/parties/:kind/:uid" => Self::Party,
            "/parties/:kind/:uid/edit" => Self::EditParty,
            "/templates" => Self::Templates,
            "/templates/upload" => Self::UploadTemplate,
            "/templates/adl2/:template_id" | "/templates/adl2/:template_id/:release" => {
                Self::Adl2Template
            }
            "/templates/:template_id" => Self::Template,
            "/terminology" => Self::Terminologies,
            "/terminology/lookup" => Self::Lookup,
            "/terminology/valuesets" => Self::ValueSets,
            "/terminology/subsumption" => Self::Subsumption,
            "/terminology/:id" => Self::Terminology,
            "/queries" => Self::StoredQueries,
            "/queries/:name/versions" => Self::StoredQueryVersions,
            "/queries/:name/:version" => Self::StoredQuery,
            "/queries/:name/:version/run" => Self::StoredQueryRun,
            "/query" => Self::Build,
            "/query/aql" => Self::Aql,
            "/server" => Self::ServerOverview,
            "/server/health" => Self::Health,
            "/server/metrics" => Self::Metrics,
            "/server/logging" => Self::Logging,
            "/server/api" => Self::Api,
            "/server/smart" => Self::Smart,
            "/audit" => Self::Audit,
            "/audit/:id" => Self::AuditRecord,
            "/subscriptions" => Self::Subscriptions,
            "/subscriptions/new" => Self::NewSubscription,
            "/subscriptions/:name/edit" => Self::EditSubscription,
            "/configuration" | "/configuration/:section" => Self::Configuration,
            "/preferences" | "/preferences/:section" => Self::Preferences,
            "/access" => Self::Access,
            "/about" => Self::About,
            "/login" => Self::Login,
            _ => return None,
        })
    }

    /// The page the address path `path` names: the first route of
    /// [`ROUTES`] that matches it, each fixed segment listed before a
    /// segment a route names, as the router ranks them.
    #[must_use]
    pub fn of(path: &str) -> Option<Self> {
        ROUTES
            .iter()
            .find(|route| matches(route.path, path))
            .and_then(|route| Self::of_pattern(route.path))
    }
}

/// The segments the route of a page names, decoded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Params(Vec<(String, String)>);

impl Params {
    /// The segment the route names `key`, decoded.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<String> {
        self.0
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    }

    /// What `read` makes of the segments.
    pub fn with<R>(&self, read: impl FnOnce(&Self) -> R) -> R {
        read(self)
    }
}

/// The segments the route of the address path `path` names, each decoded,
/// as the first route of [`ROUTES`] that matches `path` names them.
#[must_use]
pub fn params_of(path: &str) -> Params {
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let Some(route) = ROUTES.iter().find(|route| matches(route.path, path)) else {
        return Params::default();
    };
    let given = path.split('/').filter(|part| !part.is_empty());
    let wanted = route.path.split('/').filter(|part| !part.is_empty());
    Params(
        wanted
            .zip(given)
            .filter_map(|(part, segment)| {
                let name = part.strip_prefix(':')?;
                // NOTE: no openEHR spec governs this: our own design; a segment that is no
                // valid percent-encoding reads as sent, as the router would read it.
                let value = urlencoding::decode(segment)
                    .map_or_else(|_| segment.to_owned(), std::borrow::Cow::into_owned);
                Some((name.to_owned(), value))
            })
            .collect(),
    )
}

/// The segments the route of the page in view names.
#[must_use]
pub fn params() -> Params {
    params_of(&crate::request::path())
}

/// Draws the page `page` names with no tracking, as `view!` draws a
/// component.
fn drawn<V: IntoView + 'static>(page: fn() -> V) -> AnyView {
    untrack(|| page().into_any())
}

/// A group's view as it loads, erased, so every group shares one `Suspend`.
type Loading = std::pin::Pin<Box<dyn Future<Output = AnyView> + Send>>;

/// The view of the group `$group` as it loads: the server draws it in place,
/// and the browser loads its code from its chunks first, as `leptos_router`'s
/// own nested router awaits a lazy route.
macro_rules! loading {
    ($group:ident) => {{
        #[cfg(not(target_arch = "wasm32"))]
        let loading: Loading = Box::pin(async { $group::draw() });
        #[cfg(target_arch = "wasm32")]
        let loading: Loading =
            Box::pin(send_wrapper::SendWrapper::new($group::view($group::data())));
        loading
    }};
}

/// The group `loading` loads, drawn once it has.
fn group_view(loading: Loading) -> AnyView {
    view! { <Suspense fallback=|| ()>{Suspend::new(loading)}</Suspense> }.into_any()
}

/// The place in the dispatch of the group that holds `page`; an address that
/// names no page goes to the overview, which draws the page that is not there.
fn group_of(page: Option<Page>) -> u8 {
    let Some(page) = page else {
        return 0;
    };
    [
        LOverview::PAGES,
        LRecords::PAGES,
        LDefinitions::PAGES,
        LQueries::PAGES,
        LConsole::PAGES,
        LSignIn::PAGES,
    ]
    .iter()
    .zip(0..)
    .find_map(|(group, place)| group.contains(&page).then_some(place))
    .unwrap_or(0)
}

/// The one route: the group that holds the page the address names.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn Pages() -> impl IntoView {
    let location = leptos_router::hooks::use_location();
    let group = Memo::new(move |_| group_of(Page::of(&location.pathname.get())));
    move || {
        group_view(match group.get() {
            1 => loading!(LRecords),
            2 => loading!(LDefinitions),
            3 => loading!(LQueries),
            4 => loading!(LConsole),
            5 => loading!(LSignIn),
            _ => loading!(LOverview),
        })
    }
}

macro_rules! group {
    ($(#[$doc:meta])* $name:ident { $($page:ident => $draw:path),+ $(,)? }) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy)]
        pub struct $name;

        impl $name {
            /// The pages the group draws.
            pub const PAGES: &'static [Page] = &[$(Page::$page),+];

            /// The page the address names, drawn again as the address
            /// changes.
            fn draw() -> AnyView {
                let location = leptos_router::hooks::use_location();
                let kept = crate::request::kept_memo();
                // NOTE: no openEHR spec governs this: our own design; the group stays drawn
                // while the address stays in it, so it draws the page the address names.
                (move || {
                    let path = location.pathname.get();
                    let href = crate::request::address(&path, &location.search.get());
                    let posts = ferrohealth_ui::widget::submit::posts().get();
                    kept.track();
                    crate::request::follow_address(&href, posts);
                    crate::shell::undock();
                    match Page::of(&path) {
                        $(Some(Page::$page) => drawn($draw),)+
                        _ => crate::pages::errors::not_found_page(),
                    }
                })
                .into_any()
            }
        }

        #[lazy_route]
        impl LazyRoute for $name {
            fn data() -> Self {
                Self
            }

            fn view(_this: Self) -> AnyView {
                $name::draw()
            }
        }
    };
}

group!(
    /// The overview: the dashboard and the top bar's search.
    LOverview {
        Dashboard => crate::pages::dashboard::DashboardPage,
        Find => crate::pages::find::FindPage,
    }
);

group!(
    /// The records: EHRs and what each holds, compositions and parties.
    LRecords {
        Ehrs => crate::pages::ehrs::EhrsPage,
        NewEhr => crate::pages::ehrs::NewEhrPage,
        Compositions => crate::pages::ehr::compositions::CompositionsTab,
        Directory => crate::pages::ehr::directory::DirectoryTab,
        EditDirectory => crate::pages::ehr::directory::DirectoryEditPage,
        Status => crate::pages::ehr::status::StatusTab,
        EditStatus => crate::pages::ehr::status::StatusEditPage,
        Contributions => crate::pages::ehr::contribution::ContributionsTab,
        Contribution => crate::pages::ehr::contribution::ContributionPage,
        NewContribution => crate::pages::ehr::contribution::NewContributionPage,
        Tags => crate::pages::ehr::tags::TagsTab,
        NewComposition => crate::pages::composition::NewCompositionPage,
        Composition => crate::pages::composition::CompositionPage,
        EditComposition => crate::pages::composition::EditCompositionPage,
        Parties => crate::pages::parties::PartiesPage,
        NewParty => crate::pages::parties::NewPartyPage,
        NewRelationship => crate::pages::parties::NewRelationshipPage,
        Relationship => crate::pages::parties::RelationshipPage,
        EditRelationship => crate::pages::parties::EditRelationshipPage,
        PartyContribution => crate::pages::parties::PartyContributionPage,
        Party => crate::pages::parties::PartyPage,
        EditParty => crate::pages::parties::EditPartyPage,
    }
);

group!(
    /// The definitions: templates and terminology.
    LDefinitions {
        Templates => crate::pages::templates::TemplatesPage,
        UploadTemplate => crate::pages::templates::UploadTemplatePage,
        Adl2Template => crate::pages::templates::Adl2TemplatePage,
        Template => crate::pages::templates::TemplatePage,
        Terminologies => crate::pages::terminology::TerminologiesTab,
        Lookup => crate::pages::terminology::LookupTab,
        ValueSets => crate::pages::terminology::ValueSetsTab,
        Subsumption => crate::pages::terminology::SubsumptionTab,
        Terminology => crate::pages::terminology::TerminologyPage,
    }
);

group!(
    /// The queries: stored queries and Query.
    LQueries {
        StoredQueries => crate::pages::queries::StoredQueriesPage,
        StoredQueryVersions => crate::pages::queries::StoredQueryVersionsTab,
        StoredQuery => crate::pages::queries::StoredQueryPage,
        StoredQueryRun => crate::pages::queries::StoredQueryRunTab,
        Build => crate::pages::query::BuildTab,
        Aql => crate::pages::query::AqlTab,
    }
);

group!(
    /// The console: the server, the audit log, subscriptions, the
    /// configuration and the reader's own pages.
    LConsole {
        ServerOverview => crate::pages::server::OverviewTab,
        Health => crate::pages::server::HealthTab,
        Metrics => crate::pages::server::MetricsTab,
        Logging => crate::pages::server::LoggingTab,
        Api => crate::pages::server::ApiTab,
        Smart => crate::pages::server::SmartTab,
        Audit => crate::pages::audit::AuditPage,
        AuditRecord => crate::pages::audit::AuditRecordPage,
        Subscriptions => crate::pages::subscriptions::SubscriptionsPage,
        NewSubscription => crate::pages::subscriptions::NewSubscriptionPage,
        EditSubscription => crate::pages::subscriptions::EditSubscriptionPage,
        Configuration => crate::pages::configuration::ConfigurationPage,
        Preferences => crate::pages::preferences::PreferencesPage,
        Access => crate::pages::access::AccessPage,
        About => crate::pages::about::AboutPage,
    }
);

group!(
    /// The sign-in, outside the shell.
    LSignIn {
        Login => crate::pages::login::LoginPage,
    }
);

macro_rules! route {
    ($path:literal, $group:ident, $idle:literal) => {
        Route {
            path: $path,
            preload: || Box::pin($group::preload()),
            idle: $idle,
        }
    };
}

/// Every route, its group and whether it loads once the first page is idle
/// (the main navigation's), each fixed segment before a segment a route
/// names, so [`Page::of`] ranks them as the router does.
pub static ROUTES: &[Route] = &[
    route!("/", LOverview, true),
    route!("/find", LOverview, false),
    route!("/ehrs", LRecords, true),
    route!("/ehrs/new", LRecords, false),
    route!("/ehrs/:ehr_id", LRecords, false),
    route!("/ehrs/:ehr_id/directory", LRecords, false),
    route!("/ehrs/:ehr_id/directory/edit", LRecords, false),
    route!("/ehrs/:ehr_id/status", LRecords, false),
    route!("/ehrs/:ehr_id/status/edit", LRecords, false),
    route!("/ehrs/:ehr_id/contributions", LRecords, false),
    route!("/ehrs/:ehr_id/contributions/:uid", LRecords, false),
    route!("/ehrs/:ehr_id/contribution/new", LRecords, false),
    route!("/ehrs/:ehr_id/tags", LRecords, false),
    route!("/ehrs/:ehr_id/compositions/new", LRecords, false),
    route!("/ehrs/:ehr_id/compositions/:uid", LRecords, false),
    route!("/ehrs/:ehr_id/compositions/:uid/edit", LRecords, false),
    route!("/parties", LRecords, true),
    route!("/parties/new", LRecords, false),
    route!("/parties/relationships/new", LRecords, false),
    route!("/parties/relationships/:uid", LRecords, false),
    route!("/parties/relationships/:uid/edit", LRecords, false),
    route!("/parties/contributions/:uid", LRecords, false),
    route!("/parties/:kind/:uid", LRecords, false),
    route!("/parties/:kind/:uid/edit", LRecords, false),
    route!("/templates", LDefinitions, true),
    route!("/templates/upload", LDefinitions, false),
    route!("/templates/adl2/:template_id", LDefinitions, false),
    route!("/templates/adl2/:template_id/:release", LDefinitions, false),
    route!("/templates/:template_id", LDefinitions, false),
    route!("/terminology", LDefinitions, true),
    route!("/terminology/lookup", LDefinitions, false),
    route!("/terminology/valuesets", LDefinitions, false),
    route!("/terminology/subsumption", LDefinitions, false),
    route!("/terminology/:id", LDefinitions, false),
    route!("/queries", LQueries, true),
    route!("/queries/:name/versions", LQueries, false),
    route!("/queries/:name/:version", LQueries, false),
    route!("/queries/:name/:version/run", LQueries, false),
    route!("/query", LQueries, true),
    route!("/query/aql", LQueries, false),
    route!("/server", LConsole, true),
    route!("/server/health", LConsole, false),
    route!("/server/metrics", LConsole, false),
    route!("/server/logging", LConsole, false),
    route!("/server/api", LConsole, false),
    route!("/server/smart", LConsole, false),
    route!("/audit", LConsole, true),
    route!("/audit/:id", LConsole, false),
    route!("/subscriptions", LConsole, true),
    route!("/subscriptions/new", LConsole, false),
    route!("/subscriptions/:name/edit", LConsole, false),
    route!("/configuration", LConsole, true),
    route!("/configuration/:section", LConsole, false),
    route!("/preferences", LConsole, false),
    route!("/preferences/:section", LConsole, false),
    route!("/access", LConsole, false),
    route!("/about", LConsole, true),
    route!("/login", LSignIn, false),
];

#[cfg(test)]
mod tests {
    use super::{
        LConsole, LDefinitions, LOverview, LQueries, LRecords, LSignIn, Page, Params, ROUTES,
        params_of,
    };

    #[test]
    fn every_route_names_a_page() {
        for route in ROUTES {
            assert!(Page::of_pattern(route.path).is_some(), "{}", route.path);
        }
    }

    #[test]
    fn a_fixed_segment_wins_over_a_named_one() {
        assert_eq!(Some(Page::NewEhr), Page::of("/ehrs/new"));
        assert_eq!(Some(Page::Compositions), Page::of("/ehrs/abc"));
        assert_eq!(
            Some(Page::StoredQueryVersions),
            Page::of("/queries/org.example::q/versions")
        );
        assert_eq!(Some(Page::StoredQuery), Page::of("/queries/q/1.0.0"));
        assert_eq!(
            Some(Page::NewRelationship),
            Page::of("/parties/relationships/new")
        );
        assert_eq!(Some(Page::Party), Page::of("/parties/person/abc"));
        assert_eq!(Some(Page::UploadTemplate), Page::of("/templates/upload"));
        assert_eq!(Some(Page::Template), Page::of("/templates/vital_signs.v1"));
        assert_eq!(
            Some(Page::Adl2Template),
            Page::of("/templates/adl2/a/1.0.0")
        );
        assert_eq!(Some(Page::Lookup), Page::of("/terminology/lookup"));
        assert_eq!(Some(Page::Dashboard), Page::of("/"));
        assert_eq!(None, Page::of("/no/such/page"));
    }

    #[test]
    fn every_page_sits_in_one_group() {
        let groups = [
            LOverview::PAGES,
            LRecords::PAGES,
            LDefinitions::PAGES,
            LQueries::PAGES,
            LConsole::PAGES,
            LSignIn::PAGES,
        ];
        for route in ROUTES {
            let page = Page::of_pattern(route.path).expect("a page");
            let holding = groups.iter().filter(|group| group.contains(&page)).count();
            assert_eq!(1, holding, "{} sits in {holding} groups", route.path);
        }
    }

    #[test]
    fn a_page_reads_the_segments_its_route_names_decoded() {
        let params = params_of("/ehrs/abc/compositions/a%3A%3Ab::1?version=2");
        assert_eq!(Some("abc".to_owned()), params.get("ehr_id"));
        assert_eq!(Some("a::b::1".to_owned()), params.get("uid"));
        assert_eq!(None, params.get("kind"));
        let params = params_of("/templates/adl2/openEHR-EHR-COMPOSITION.x.v1/1.0.0");
        assert_eq!(Some("1.0.0".to_owned()), params.get("release"));
        assert_eq!(Params::default(), params_of("/no/such/page"));
    }
}
