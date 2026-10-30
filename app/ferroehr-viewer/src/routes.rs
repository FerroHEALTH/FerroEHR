// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer's pages by the route that names each, and the draw of the page
//! a request names.
//!
//! [`ROUTES`] is the one route table, each fixed segment listed before a
//! segment a route names, so [`Page::of`] ranks them as a router does; a page
//! reads the segments its route names through [`params`]. [`draw`] draws the
//! page a request names, once, whole, on the server (FerroUI
//! `docs/consoles.md`, "How a console builds"). No openEHR spec governs the
//! routing: our own design.

use ferrohealth_ui::html::Html;

/// A page as its handler drew it: its bare title, its content and whether
/// the shell frames it.
#[derive(Debug, Clone, Default)]
pub struct Drawn {
    /// The page's bare title, which the document adds the product name to.
    pub title: String,
    /// The page's content.
    pub body: Html,
    /// Whether the content stands alone, centred, with no shell: the
    /// sign-in page.
    pub bare: bool,
}

impl Drawn {
    /// A page titled `title` drawn in the shell.
    #[must_use]
    pub const fn page(title: String, body: Html) -> Self {
        Self {
            title,
            body,
            bare: false,
        }
    }

    /// A page titled `title` drawn centred, with no shell.
    #[must_use]
    pub const fn bare(title: String, body: Html) -> Self {
        Self {
            title,
            body,
            bare: true,
        }
    }
}

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
    /// segment a route names, as a router ranks them.
    #[must_use]
    pub fn of(path: &str) -> Option<Self> {
        ROUTES
            .iter()
            .find(|route| matches(route, path))
            .and_then(|route| Self::of_pattern(route))
    }
}

/// Whether the address path `path` is one the route `pattern` names: the
/// same number of segments, each fixed one equal and each `:name` one
/// filled.
#[must_use]
pub fn matches(pattern: &str, path: &str) -> bool {
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let wanted: Vec<&str> = pattern.split('/').filter(|part| !part.is_empty()).collect();
    let given: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    wanted.len() == given.len()
        && wanted
            .iter()
            .zip(&given)
            .all(|(part, segment)| part.starts_with(':') || part == segment)
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
    let Some(route) = ROUTES.iter().find(|route| matches(route, path)) else {
        return Params::default();
    };
    let given = path.split('/').filter(|part| !part.is_empty());
    let wanted = route.split('/').filter(|part| !part.is_empty());
    Params(
        wanted
            .zip(given)
            .filter_map(|(part, segment)| {
                let name = part.strip_prefix(':')?;
                // NOTE: no openEHR spec governs this: our own design; a segment that is no
                // valid percent-encoding reads as sent.
                let value = urlencoding::decode(segment)
                    .map_or_else(|_| segment.to_owned(), std::borrow::Cow::into_owned);
                Some((name.to_owned(), value))
            })
            .collect(),
    )
}

/// The segments the route of the page being drawn names.
#[must_use]
pub fn params() -> Params {
    params_of(&crate::request::path())
}

/// Every route, each fixed segment before a segment a route names, so
/// [`Page::of`] ranks them as a router does.
pub static ROUTES: &[&str] = &[
    "/",
    "/find",
    "/ehrs",
    "/ehrs/new",
    "/ehrs/:ehr_id",
    "/ehrs/:ehr_id/directory",
    "/ehrs/:ehr_id/directory/edit",
    "/ehrs/:ehr_id/status",
    "/ehrs/:ehr_id/status/edit",
    "/ehrs/:ehr_id/contributions",
    "/ehrs/:ehr_id/contributions/:uid",
    "/ehrs/:ehr_id/contribution/new",
    "/ehrs/:ehr_id/tags",
    "/ehrs/:ehr_id/compositions/new",
    "/ehrs/:ehr_id/compositions/:uid",
    "/ehrs/:ehr_id/compositions/:uid/edit",
    "/parties",
    "/parties/new",
    "/parties/relationships/new",
    "/parties/relationships/:uid",
    "/parties/relationships/:uid/edit",
    "/parties/contributions/:uid",
    "/parties/:kind/:uid",
    "/parties/:kind/:uid/edit",
    "/templates",
    "/templates/upload",
    "/templates/adl2/:template_id",
    "/templates/adl2/:template_id/:release",
    "/templates/:template_id",
    "/terminology",
    "/terminology/lookup",
    "/terminology/valuesets",
    "/terminology/subsumption",
    "/terminology/:id",
    "/queries",
    "/queries/:name/versions",
    "/queries/:name/:version",
    "/queries/:name/:version/run",
    "/query",
    "/query/aql",
    "/server",
    "/server/health",
    "/server/metrics",
    "/server/logging",
    "/server/api",
    "/server/smart",
    "/audit",
    "/audit/:id",
    "/subscriptions",
    "/subscriptions/new",
    "/subscriptions/:name/edit",
    "/configuration",
    "/configuration/:section",
    "/preferences",
    "/preferences/:section",
    "/access",
    "/about",
    "/login",
];

/// Draws the page the request being answered names, or the page that is not
/// there.
pub async fn draw() -> Drawn {
    use crate::pages;
    let Some(page) = Page::of(&crate::request::path()) else {
        return pages::errors::not_found_page();
    };
    match page {
        Page::Dashboard => pages::dashboard::page().await,
        Page::Find => pages::find::page().await,
        Page::Ehrs => pages::ehrs::list_page().await,
        Page::NewEhr => pages::ehrs::new_page().await,
        Page::Compositions => pages::ehr::compositions::page().await,
        Page::Directory => pages::ehr::directory::page().await,
        Page::EditDirectory => pages::ehr::directory::edit_page().await,
        Page::Status => pages::ehr::status::page().await,
        Page::EditStatus => pages::ehr::status::edit_page().await,
        Page::Contributions => pages::ehr::contribution::tab_page().await,
        Page::Contribution => pages::ehr::contribution::page().await,
        Page::NewContribution => pages::ehr::contribution::new_page().await,
        Page::Tags => pages::ehr::tags::page().await,
        Page::NewComposition => pages::composition::new_page().await,
        Page::Composition => pages::composition::page().await,
        Page::EditComposition => pages::composition::edit_composition_page().await,
        Page::Parties => pages::parties::list_page().await,
        Page::NewParty => pages::parties::new_party_page().await,
        Page::NewRelationship => pages::parties::new_relationship_page().await,
        Page::Relationship => pages::parties::relationship_page().await,
        Page::EditRelationship => pages::parties::edit_relationship_page().await,
        Page::PartyContribution => pages::parties::party_contribution_page().await,
        Page::Party => pages::parties::party_page().await,
        Page::EditParty => pages::parties::edit_party_page().await,
        Page::Templates => pages::templates::list_page().await,
        Page::UploadTemplate => pages::templates::upload_page().await,
        Page::Adl2Template => pages::templates::adl2_page().await,
        Page::Template => pages::templates::page().await,
        Page::Terminologies => pages::terminology::terminologies_page().await,
        Page::Lookup => pages::terminology::lookup_page().await,
        Page::ValueSets => pages::terminology::value_sets_page().await,
        Page::Subsumption => pages::terminology::subsumption_page().await,
        Page::Terminology => pages::terminology::terminology_page().await,
        Page::StoredQueries => pages::queries::list_page().await,
        Page::StoredQueryVersions => pages::queries::versions_page().await,
        Page::StoredQuery => pages::queries::page().await,
        Page::StoredQueryRun => pages::queries::run_page().await,
        Page::Build => pages::query::build_page().await,
        Page::Aql => pages::query::aql_page().await,
        Page::ServerOverview => pages::server::overview_page().await,
        Page::Health => pages::server::health_page().await,
        Page::Metrics => pages::server::metrics_page().await,
        Page::Logging => pages::server::logging_page().await,
        Page::Api => pages::server::api_page().await,
        Page::Smart => pages::server::smart_page().await,
        Page::Audit => pages::audit::page().await,
        Page::AuditRecord => pages::audit::record_page().await,
        Page::Subscriptions => pages::subscriptions::page().await,
        Page::NewSubscription => pages::subscriptions::new_page().await,
        Page::EditSubscription => pages::subscriptions::edit_page().await,
        Page::Configuration => pages::configuration::page().await,
        Page::Preferences => pages::preferences::page(),
        Page::Access => pages::access::page(),
        Page::About => pages::about::page().await,
        Page::Login => pages::login::login_page().await,
    }
}

#[cfg(test)]
mod tests {
    use super::{Page, Params, ROUTES, matches, params_of};

    #[test]
    fn every_route_names_a_page() {
        for route in ROUTES {
            assert!(Page::of_pattern(route).is_some(), "{route}");
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
    fn a_route_matches_only_its_own_shape() {
        assert!(matches("/ehrs/:ehr_id", "/ehrs/abc"));
        assert!(matches("/ehrs/:ehr_id", "/ehrs/abc?tab=x"));
        assert!(!matches("/ehrs/:ehr_id", "/ehrs"));
        assert!(!matches("/ehrs/:ehr_id", "/ehrs/abc/status"));
        assert!(matches("/", "/"));
        assert!(!matches("/", "/ehrs"));
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
