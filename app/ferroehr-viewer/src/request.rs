// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a page reads from the request it answers, and what it answers with.
//!
//! The path, the query, the display cookies, who is signed in, the viewer's
//! own settings and the instant the page is drawn at; its status, its
//! redirect and the cookies it sets.
//!
//! Every read goes to the request's [`crate::scope::Scope`], which the page
//! handler holds for the page's whole draw. No openEHR spec governs the
//! viewer's pages: our own design.

use serde::{Deserialize, Serialize};

/// How the signed-in reader's session reaches the CDR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Whether the session reaches the CDR with a bearer token from the
    /// viewer's OpenID Connect sign-in ([`Signed::method`]).
    pub oidc: bool,
    /// The scopes granted to the session's token, empty for Basic.
    pub scopes: Vec<String>,
}

impl Signed {
    /// How the session reaches the CDR.
    #[must_use]
    pub const fn method(&self) -> Method {
        if self.oidc {
            Method::Oidc
        } else {
            Method::Basic
        }
    }
}

/// The viewer's own settings a page shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    /// The CDR's base address.
    pub cdr_address: String,
    /// The minutes a session stays open while the reader does nothing.
    pub idle_minutes: u64,
    /// The OpenID Provider's issuer, empty when none is configured.
    pub oidc_issuer: String,
}

/// The prefix every display cookie carries: the preferences, the sidebar and
/// the listings' positions.
const KEPT_PREFIX: &str = "ferroehr-viewer-";

/// The cookies of the `Cookie` header value `header` that keep the reader's
/// display choices, in the order sent.
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

/// The path of the request, `/` when no request is in reach.
#[must_use]
pub fn path() -> String {
    crate::scope::with(|scope| scope.parts.uri.path().to_owned()).unwrap_or_else(|| "/".to_owned())
}

/// The path and query of the request, as a page names the place to return to.
#[must_use]
pub fn path_and_query() -> String {
    crate::scope::with(|scope| {
        scope
            .parts
            .uri
            .path_and_query()
            .map_or_else(|| "/".to_owned(), ToString::to_string)
    })
    .unwrap_or_else(|| "/".to_owned())
}

/// The query parameter `key` of `query`, trimmed; `None` when absent or empty.
#[must_use]
pub fn query_in(query: &str, key: &str) -> Option<String> {
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

/// The query parameter `key` of the request, trimmed; `None` when absent or
/// empty.
#[must_use]
pub fn query(key: &str) -> Option<String> {
    let whole = path_and_query();
    let (_, query) = whole.split_once('?')?;
    query_in(query, key)
}

/// Every query parameter of `query`, in the order sent.
#[must_use]
pub fn pairs_in(query: &str) -> Vec<(String, String)> {
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

/// Every query parameter of the request, in the order sent.
#[must_use]
pub fn query_pairs() -> Vec<(String, String)> {
    let whole = path_and_query();
    whole
        .split_once('?')
        .map(|(_, query)| pairs_in(query))
        .unwrap_or_default()
}

/// The request's `Cookie` header, empty for none.
#[must_use]
pub fn cookies() -> String {
    crate::scope::with(|scope| {
        scope
            .parts
            .headers
            .get(http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    })
    .flatten()
    .unwrap_or_default()
}

/// The request's display cookies: the preferences, the sidebar and the
/// listings' positions, as one `Cookie` header value.
#[must_use]
pub fn kept_cookies() -> String {
    kept_of(&cookies())
}

/// Who is signed in, when the request carries a session.
#[must_use]
pub fn signed() -> Option<Signed> {
    session().map(|session| Signed {
        identity: session.identity,
        oidc: matches!(
            session.credential,
            crate::session::Credential::Bearer { .. }
        ),
        scopes: session.scopes,
    })
}

/// The viewer's own settings a page shows.
#[must_use]
pub fn settings() -> Settings {
    state()
        .map(|state| Settings {
            cdr_address: state.config.cdr.base_url.clone(),
            idle_minutes: state.config.session.idle_minutes,
            oidc_issuer: state.config.auth.oidc.issuer.clone(),
        })
        .unwrap_or_default()
}

/// The instant the page is drawn at, read once for the whole page.
#[must_use]
pub fn now() -> jiff::Timestamp {
    crate::scope::with(|scope| scope.now).unwrap_or_else(jiff::Timestamp::now)
}

/// The viewer session the request's sealed cookie carries, when it carries
/// one that is still inside its idle window.
#[must_use]
pub fn session() -> Option<crate::session::ViewerSession> {
    crate::scope::with(|scope| {
        crate::session::unseal(
            &scope.state.session_keys,
            &scope.parts.headers,
            scope.state.config.session.idle_minutes,
        )
        .get::<crate::session::ViewerSession>(crate::session::SESSION_KEY)
    })
    .flatten()
}

/// The viewer's state, when a request is in reach.
#[must_use]
pub fn state() -> Option<crate::state::AppState> {
    crate::scope::with(|scope| scope.state.clone())
}

/// Sets the response status of the page being drawn.
///
/// A page that already ended in a redirect keeps it: a post that succeeded
/// keeps its redirect even when its page can no longer read its object.
pub fn set_status(status: http::StatusCode) {
    crate::scope::with(|scope| {
        scope.answer(|answer| {
            if answer.redirect.is_none() {
                answer.status = Some(status);
            }
        });
    });
}

/// Sends the browser to `to`, one of the viewer's own pages, instead of
/// drawing the page.
pub fn redirect(to: &str) {
    let to = return_path(to).to_owned();
    crate::scope::with(|scope| scope.answer(|answer| answer.redirect = Some(to)));
}

/// Appends a `Set-Cookie` to the response of the page being drawn.
pub fn set_cookie(cookie: &str) {
    if let Ok(value) = http::HeaderValue::from_str(cookie) {
        crate::scope::with(|scope| scope.answer(|answer| answer.cookies.push(value)));
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
    use super::{kept_of, pairs_in, query_in, return_path};

    #[test]
    fn a_page_reads_only_the_display_cookies() {
        let header = "ferroehr_viewer_session=sealed; ferroehr-viewer-theme=dark; \
                      other=1; ferroehr-viewer-flash=toast; ferroehr-viewer-paging-ehrs=2.25";
        assert_eq!(
            "ferroehr-viewer-theme=dark; ferroehr-viewer-paging-ehrs=2.25",
            kept_of(header)
        );
        assert_eq!("", kept_of(""));
    }

    #[test]
    fn the_query_answers_its_parameters_decoded() {
        let query = "q=a+b&empty=&q=c";
        assert_eq!(Some("a b".to_owned()), query_in(query, "q"));
        assert_eq!(None, query_in(query, "empty"));
        assert_eq!(3, pairs_in(query).len());
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
