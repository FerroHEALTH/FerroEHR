// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a page reads from the request it answers: the path, the query, the
//! display cookies, who is signed in, the viewer's own settings and the
//! instant the page is drawn at.
//!
//! The server reads them once per page into a [`Visit`], which the app
//! provides through a `SharedValue`, so the browser takes the same values
//! serialized with the page and both halves draw the same page (the Leptos
//! book, "Life Cycle of a Page Load"). The browser draws every page after the
//! first itself: the route group sets the visit's address as the router
//! moves and asks the server for the rest again ([`follow_address`]) after
//! each navigation and each post drawn in the page. The visit carries no session cookie and no credential, so
//! neither reaches a script. A read outside the app falls back to the
//! request itself, and a page's model read as a server function reads the
//! visit of the page that asked ([`visiting`]). No openEHR spec governs the
//! viewer's pages: our own design.
#![cfg_attr(
    target_arch = "wasm32",
    allow(
        clippy::unused_async_trait_impl,
        reason = "the server function macro writes the browser half's call as an async trait method; only that target has it"
    )
)]

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
use crate::error::ViewerError;

/// The prefix every cookie a page may read in the browser carries: the
/// preferences, the sidebar and the listings' positions.
const KEPT_PREFIX: &str = "ferroehr-viewer-";

/// How the signed-in reader's session reaches the CDR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Method {
    /// Basic authentication, checked against the CDR at sign-in.
    Basic,
    /// A bearer token from the viewer's OpenID Connect sign-in.
    Oidc,
}

/// Who is signed in, without the credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signed {
    /// The display identity.
    pub identity: String,
    /// How the session reaches the CDR.
    pub method: Method,
    /// The scopes granted to the session's token, empty for Basic.
    pub scopes: Vec<String>,
}

/// The viewer's own settings a page shows.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Settings {
    /// The CDR's base address.
    pub cdr_address: String,
    /// The minutes a session stays open while the reader does nothing.
    pub idle_minutes: u64,
    /// The OpenID Provider's issuer, empty when none is configured.
    pub oidc_issuer: String,
}

/// What the server read of the request for the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Visit {
    /// The path and query of the request.
    pub path_and_query: String,
    /// The request's display cookies alone, as one `Cookie` header value.
    pub kept: String,
    /// Who is signed in, when the request carries a session.
    pub signed: Option<Signed>,
    /// The viewer's own settings.
    pub settings: Settings,
    /// The instant the server read its clock for the page, in milliseconds
    /// since the Unix epoch: every relative time and every today the page
    /// draws counts from it, on both halves ([`now`]).
    pub now: i64,
}

impl Default for Visit {
    fn default() -> Self {
        Self {
            path_and_query: "/".to_owned(),
            kept: String::new(),
            signed: None,
            settings: Settings::default(),
            now: 0,
        }
    }
}

impl Visit {
    /// What the server reads of the request being served.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn requested() -> Self {
        let state = state();
        Self {
            path_and_query: requested_path_and_query(),
            kept: kept_of(&cookies()),
            signed: session().map(|session| Signed {
                identity: session.identity,
                method: match session.credential {
                    crate::session::Credential::Basic { .. } => Method::Basic,
                    crate::session::Credential::Bearer { .. } => Method::Oidc,
                },
                scopes: session.scopes,
            }),
            settings: state
                .map(|state| Settings {
                    cdr_address: state.config.cdr.base_url.clone(),
                    idle_minutes: state.config.session.idle_minutes,
                    oidc_issuer: state.config.auth.oidc.issuer.clone(),
                })
                .unwrap_or_default(),
            now: jiff::Timestamp::now().as_millisecond(),
        }
    }

    /// What the server reads of the request being served, for the page at
    /// `href`, which a page's model read as a server function names: the
    /// viewer's own path, else `/`.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn of_page(href: &str) -> Self {
        Self {
            path_and_query: return_path(href).to_owned(),
            ..Self::requested()
        }
    }

    /// The visit the page answers: the shared one, else the request's own.
    fn current() -> Self {
        use_context::<Visits>().map_or_else(
            || {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    Self::requested()
                }
                #[cfg(target_arch = "wasm32")]
                {
                    Self::default()
                }
            },
            |visits| visits.0.get_untracked(),
        )
    }
}

/// The visit the page answers, which the browser updates as it draws the
/// pages after the first.
#[derive(Debug, Clone, Copy)]
pub struct Visits(pub RwSignal<Visit>);

/// The visit the page answers, tracked, so a part of the shell that draws it
/// draws again when the server's answer after a navigation changes it.
#[must_use]
pub fn tracked() -> Visit {
    use_context::<Visits>().map_or_else(Visit::current, |visits| visits.0.get())
}

/// The display cookies of the visit, as a memo a route group draws its page
/// again on when the server's answer after a post changes them.
#[must_use]
pub fn kept_memo() -> Memo<String> {
    let visits = use_context::<Visits>();
    Memo::new(move |_| {
        visits.map_or_else(String::new, |Visits(visits)| {
            visits.with(|visit| visit.kept.clone())
        })
    })
}

/// Tracks the visit the page answers, so the part of the shell that calls it
/// draws again when the server's answer after a navigation changes it.
pub fn track() {
    if let Some(Visits(visits)) = use_context::<Visits>() {
        visits.track();
    }
}

/// Provides `visit` as the visit the page answers.
pub fn provide(visit: Visit) {
    provide_context(Visits(RwSignal::new(visit)));
}

/// Provides the visit to the page: read from the request on the server and
/// taken from the serialized page in the browser.
pub fn provide_visit() {
    let visit = SharedValue::new(|| {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Visit::requested()
        }
        #[cfg(target_arch = "wasm32")]
        {
            Visit::default()
        }
    })
    .into_inner();
    provide(visit);
}

/// Sets the address of the visit to `href`, the page the router draws.
///
/// The route group calls it before the page reads the visit; `posts` counts
/// the posts the router has drawn in the page. The form the first page
/// answered lets go once the page is another one or a post drawn in the page
/// followed it.
///
/// The server's visit is the request's, and a hydrating page keeps the one
/// the server drew, so only a page the browser draws after it follows; for
/// that page the browser asks the server for the rest of the visit again
/// ([`asked::visit_now`]).
pub fn follow_address(href: &str, posts: u32) {
    if cfg!(not(target_arch = "wasm32")) || !ferrohealth_ui::widget::hydration::hydrated() {
        return;
    }
    if let Some(Visits(visits)) = use_context::<Visits>()
        && visits.with_untracked(|visit| visit.path_and_query != href)
    {
        visits.update_untracked(|visit| href.clone_into(&mut visit.path_and_query));
        crate::posted::let_go();
    }
    if posts > 0 {
        crate::posted::let_go();
    }
    refresh(href.to_owned());
}

/// Marks a page's model read as a server function: the response is the
/// server function's own, so a page's status stays the page's.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy)]
struct Fetched;

/// Opens the read of a page's model for the page at `href`.
///
/// Drawn by the server, the page's own visit stands. Asked by the browser
/// for a page it draws after a navigation, the read is a server function:
/// the visit is the request's, at the address of the page that asked, and
/// [`set_status`] leaves the server function's answer alone.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
#[cfg(not(target_arch = "wasm32"))]
pub async fn visiting(href: &str) -> Result<crate::session::ViewerSession, ViewerError> {
    let session = crate::session::require_session().await?;
    visiting_public(href);
    Ok(session)
}

/// Opens the read of the model of a page anyone may read, the sign-in, for
/// the page at `href`, as [`visiting`] does with no session.
#[cfg(not(target_arch = "wasm32"))]
pub fn visiting_public(href: &str) {
    if use_context::<Visits>().is_none() {
        provide(Visit::of_page(href));
        provide_context(Fetched);
    }
}

/// The server function the browser asks the visit of a page with.
pub mod asked {
    #![cfg_attr(
        not(target_arch = "wasm32"),
        expect(
            clippy::unused_async,
            reason = "the server function reads the request alone, with nothing to await"
        )
    )]

    use leptos::server;

    use super::Visit;
    use crate::error::ViewerError;

    /// The visit of the request that asks, for the page at `href` the
    /// browser draws after a navigation or a post.
    ///
    /// # Errors
    /// When the server function cannot be reached or answered.
    #[server(client = crate::session_client::SessionAwareClient)]
    pub async fn visit_now(
        /// The path and query of the page the browser draws.
        href: String,
    ) -> Result<Visit, ViewerError> {
        Ok(Visit::of_page(&href))
    }
}

/// Asks the server for the visit of the page at `href` again, so a
/// preference or a position a post just kept, or a session that ended,
/// reaches the page drawn in the browser.
fn refresh(href: String) {
    let Some(Visits(visits)) = use_context::<Visits>() else {
        return;
    };
    leptos::task::spawn_local(async move {
        match asked::visit_now(href).await {
            Ok(now) => {
                let before = visits.with_untracked(|visit| visit.kept.clone());
                if follow_root(&before, &now) {
                    return;
                }
                // NOTE: no openEHR spec governs this: our own design; the address is the
                // router's, so an answer to an earlier navigation never moves the page.
                let kept = visits.with_untracked(|visit| visit.path_and_query.clone());
                let now = Visit {
                    path_and_query: kept,
                    ..now
                };
                if visits
                    .with_untracked(|visit| visit.kept != now.kept || visit.signed != now.signed)
                {
                    visits.set(now);
                } else {
                    visits.update_untracked(|visit| *visit = now);
                }
            }
            Err(error) => leptos::logging::warn!("the visit did not come: {error}"),
        }
    });
}

/// The path and query of the router's location `path` and raw query
/// `search`.
#[must_use]
pub fn address(path: &str, search: &str) -> String {
    if search.is_empty() {
        path.to_owned()
    } else {
        format!("{path}?{search}")
    }
}

/// Sets the root element's theme, density, motion and hints from the visit
/// `now`, as the server writes them on a document it draws, so a preference
/// a post just kept takes effect in the page; `true` when the language the
/// display cookies `before` kept changed, and the browser loads the page
/// again in its new language.
fn follow_root(before: &str, now: &Visit) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        use ferrohealth_ui::theme::{Language, Preference as _, cookie};
        let language = format!("{}{}", crate::prefs::COOKIE_PREFIX, Language::COOKIE);
        // NOTE: no openEHR spec governs this: our own design; the words of a page are
        // drawn in its locale, so a new language loads the page as a document.
        if cookie(before, &language) != cookie(&now.kept, &language) {
            if window().location().reload().is_err() {
                leptos::logging::warn!("the browser refused to load the page in its language");
            }
            return true;
        }
        let Some(root) = document().document_element() else {
            return false;
        };
        let chosen =
            ferrohealth_ui::theme::Preferences::read(&now.kept, crate::prefs::COOKIE_PREFIX);
        let attributes = [
            ("data-theme", chosen.theme.key()),
            ("data-density", chosen.density.key()),
            ("data-motion", chosen.motion.key()),
            ("data-hints", chosen.hints.key()),
        ];
        for (name, value) in attributes {
            if root.get_attribute(name).as_deref() != Some(value)
                && root.set_attribute(name, value).is_err()
            {
                leptos::logging::warn!("the browser refused the root's {name}");
            }
        }
        false
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _: (&str, &Visit) = (before, now);
        false
    }
}

/// The cookies of the `Cookie` header value `header` a page may read in the
/// browser, in the order sent.
#[must_use]
pub fn kept_of(header: &str) -> String {
    header
        .split(';')
        .map(str::trim)
        .filter(|pair| {
            let name = pair.split_once('=').map_or(*pair, |(name, _)| name);
            name.starts_with(KEPT_PREFIX) && name != crate::flash::FLASH_COOKIE
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// The request the page answers, when one is in reach.
#[cfg(not(target_arch = "wasm32"))]
fn parts() -> Option<http::request::Parts> {
    use_context::<http::request::Parts>()
}

/// The path and query of the request in reach, `/` when none is.
#[cfg(not(target_arch = "wasm32"))]
fn requested_path_and_query() -> String {
    parts()
        .and_then(|parts| parts.uri.path_and_query().map(ToString::to_string))
        .unwrap_or_else(|| "/".to_owned())
}

/// The path of the request, `/` when no request is in reach.
#[must_use]
pub fn path() -> String {
    let mut whole = path_and_query();
    if let Some(query) = whole.find('?') {
        whole.truncate(query);
    }
    whole
}

/// The path and query of the request, as a page names the place to return to.
#[must_use]
pub fn path_and_query() -> String {
    match use_context::<Visits>() {
        Some(visits) => visits
            .0
            .with_untracked(|visit| visit.path_and_query.clone()),
        None => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                requested_path_and_query()
            }
            #[cfg(target_arch = "wasm32")]
            {
                "/".to_owned()
            }
        }
    }
}

/// The query of the request, when it has one.
fn query_text() -> Option<String> {
    path_and_query()
        .split_once('?')
        .map(|(_, query)| query.to_owned())
}

/// The query parameter `key` of the request, trimmed; `None` when absent or
/// empty.
#[must_use]
pub fn query(key: &str) -> Option<String> {
    let query = query_text()?;
    query.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        let name = urlencoding::decode(name).ok()?;
        if name != key {
            return None;
        }
        let value = urlencoding::decode(&value.replace('+', " "))
            .ok()?
            .trim()
            .to_owned();
        (!value.is_empty()).then_some(value)
    })
}

/// Every query parameter of the request, in the order sent.
#[must_use]
pub fn query_pairs() -> Vec<(String, String)> {
    let Some(query) = query_text() else {
        return Vec::new();
    };
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            let name = urlencoding::decode(name).ok()?.into_owned();
            let value = urlencoding::decode(&value.replace('+', " "))
                .ok()?
                .into_owned();
            Some((name, value))
        })
        .collect()
}

/// The request's `Cookie` header, empty for none.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn cookies() -> String {
    parts()
        .and_then(|parts| {
            parts
                .headers
                .get(http::header::COOKIE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

/// The request's display cookies: the preferences, the sidebar and the
/// listings' positions, as one `Cookie` header value.
#[must_use]
pub fn kept_cookies() -> String {
    match use_context::<Visits>() {
        Some(visits) => visits.0.with_untracked(|visit| visit.kept.clone()),
        None => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                kept_of(&cookies())
            }
            #[cfg(target_arch = "wasm32")]
            {
                String::new()
            }
        }
    }
}

/// Who is signed in, when the request carries a session.
#[must_use]
pub fn signed() -> Option<Signed> {
    Visit::current().signed
}

/// The viewer's own settings a page shows.
#[must_use]
pub fn settings() -> Settings {
    Visit::current().settings
}

/// The instant the page is drawn at, read once by the server for the whole
/// page, so the browser draws the same relative times and the same today.
#[must_use]
pub fn now() -> jiff::Timestamp {
    match use_context::<Visits>() {
        Some(visits) => instant(visits.0.with_untracked(|visit| visit.now)),
        None => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                jiff::Timestamp::now()
            }
            #[cfg(target_arch = "wasm32")]
            {
                instant(Visit::default().now)
            }
        }
    }
}

/// The instant `milliseconds` after the Unix epoch.
///
/// # Panics
/// When `milliseconds` lies outside jiff's range, which an instant the
/// server's clock read never does.
#[expect(
    clippy::panic,
    reason = "the server writes the milliseconds of an instant its own clock read, which lie inside jiff's range; a panic that formats jiff's error would link jiff's whole error text into the entry WebAssembly"
)]
fn instant(milliseconds: i64) -> jiff::Timestamp {
    let Ok(instant) = jiff::Timestamp::from_millisecond(milliseconds) else {
        panic!("the visit's instant should lie inside jiff's range");
    };
    instant
}

/// The viewer session the request's sealed cookie carries, when it carries
/// one that is still inside its idle window.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn session() -> Option<crate::session::ViewerSession> {
    let parts = parts()?;
    let state = use_context::<crate::state::AppState>()?;
    crate::session::unseal(
        &state.session_keys,
        &parts.headers,
        state.config.session.idle_minutes,
    )
    .get::<crate::session::ViewerSession>(crate::session::SESSION_KEY)
}

/// The viewer's state, when the render was given it.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn state() -> Option<crate::state::AppState> {
    use_context::<crate::state::AppState>()
}

/// Sets the response status of the page being drawn.
///
/// A page that already ended in a redirect keeps it: a post that succeeded
/// keeps its `302` even when its page can no longer read its object.
#[cfg(not(target_arch = "wasm32"))]
pub fn set_status(status: http::StatusCode) {
    if use_context::<Fetched>().is_some() {
        return;
    }
    if let Some(response) = use_context::<leptos_axum::ResponseOptions>() {
        let Ok(mut parts) = response.0.write() else {
            return;
        };
        if parts.status.is_some_and(|set| set.is_redirection()) {
            return;
        }
        parts.status = Some(status);
    }
}

/// Appends a `Set-Cookie` to the response of the page being drawn.
#[cfg(not(target_arch = "wasm32"))]
pub fn set_cookie(cookie: &str) {
    if let (Some(response), Ok(value)) = (
        use_context::<leptos_axum::ResponseOptions>(),
        http::HeaderValue::from_str(cookie),
    ) {
        response.append_header(http::header::SET_COOKIE, value);
    }
}

/// The path `back` names when it is one of the viewer's own, a plain path of
/// visible ASCII, and `/` for anything else, so a post never redirects off
/// the viewer.
#[must_use]
pub fn return_path(back: &str) -> &str {
    let plain = back.starts_with('/')
        && !back.starts_with("//")
        && !back.starts_with("/\\")
        && back.chars().all(|c| c.is_ascii_graphic() && c != '\\')
        && !back.split(['/', '?']).any(|segment| segment == "..");
    if plain { back } else { "/" }
}

#[cfg(test)]
mod tests {
    use super::{Visit, kept_of, now, path, path_and_query, query, query_pairs, return_path};

    #[test]
    fn a_page_reads_only_the_display_cookies_in_the_browser() {
        let header = "ferroehr_viewer_session=sealed; ferroehr-viewer-theme=dark; \
                      other=1; ferroehr-viewer-flash=toast; ferroehr-viewer-paging-ehrs=2.25";
        assert_eq!(
            "ferroehr-viewer-theme=dark; ferroehr-viewer-paging-ehrs=2.25",
            kept_of(header)
        );
        assert_eq!("", kept_of(""));
    }

    #[test]
    fn the_shared_visit_answers_the_path_and_the_query() {
        let owner = leptos::prelude::Owner::new();
        owner.with(|| {
            super::provide(Visit {
                path_and_query: "/ehrs?q=a+b&empty=&q=c".to_owned(),
                ..Visit::default()
            });
            assert_eq!("/ehrs", path());
            assert_eq!("/ehrs?q=a+b&empty=&q=c", path_and_query());
            assert_eq!(Some("a b".to_owned()), query("q"));
            assert_eq!(None, query("empty"));
            assert_eq!(3, query_pairs().len());
        });
    }

    #[test]
    fn the_page_counts_from_the_instant_the_shared_visit_carries() {
        let owner = leptos::prelude::Owner::new();
        owner.with(|| {
            let read: jiff::Timestamp = "2026-10-21T09:42:11Z".parse().expect("an instant");
            super::provide(Visit {
                now: read.as_millisecond(),
                ..Visit::default()
            });
            assert_eq!(read, now());
            assert_eq!(
                read,
                now(),
                "every read of the page answers the same instant"
            );
        });
    }

    #[test]
    fn a_post_returns_only_to_a_page_of_the_viewer() {
        assert_eq!("/ehrs", return_path("/ehrs"));
        assert_eq!("/ehrs?tab=x", return_path("/ehrs?tab=x"));
        assert_eq!("/", return_path("//evil.example"));
        assert_eq!("/", return_path("/\\evil.example"));
        assert_eq!("/", return_path("https://evil.example"));
        assert_eq!("/", return_path("/a/../b"));
        assert_eq!("/", return_path("/a b"));
    }
}
