// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a page reads from the request it answers: the path, the query, the
//! display cookies, who is signed in and the viewer's own settings.
//!
//! The server reads them once per page into a [`Visit`], which the app
//! provides through a `SharedValue`, so the browser takes the same values
//! serialized with the page and both halves draw the same page (the Leptos
//! book, "Life Cycle of a Page Load"). The visit carries no session cookie
//! and no credential, so neither reaches a script. A read outside the app,
//! such as a server function's, falls back to the request itself. No openEHR
//! spec governs the viewer's pages: our own design.

use leptos::prelude::{SharedValue, provide_context, use_context};
use serde::{Deserialize, Serialize};

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
}

impl Default for Visit {
    fn default() -> Self {
        Self {
            path_and_query: "/".to_owned(),
            kept: String::new(),
            signed: None,
            settings: Settings::default(),
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
        }
    }

    /// The visit the page answers: the shared one, else the request's own.
    fn current() -> Self {
        use_context::<Self>().unwrap_or_else(|| {
            #[cfg(not(target_arch = "wasm32"))]
            {
                Self::requested()
            }
            #[cfg(target_arch = "wasm32")]
            {
                Self::default()
            }
        })
    }
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
    provide_context(visit);
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
    match use_context::<Visit>() {
        Some(visit) => visit.path_and_query,
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
    match use_context::<Visit>() {
        Some(visit) => visit.kept,
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
    use super::{Visit, kept_of, path, path_and_query, query, query_pairs, return_path};

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
            leptos::prelude::provide_context(Visit {
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
