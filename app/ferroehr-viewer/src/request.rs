// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a server-drawn page reads from the request it answers: the path, the
//! query, the cookies and the session.
//!
//! Every page renders on the server alone (islands mode), so these read the
//! `http::request::Parts` `leptos_axum` provides to the render, never a signal.
//! No openEHR spec governs the viewer's pages: our own design.

use leptos::prelude::use_context;

/// The request the page answers, when one is in reach.
fn parts() -> Option<http::request::Parts> {
    use_context::<http::request::Parts>()
}

/// The path of the request, `/` when no request is in reach.
#[must_use]
pub fn path() -> String {
    parts().map_or_else(|| "/".to_owned(), |parts| parts.uri.path().to_owned())
}

/// The path and query of the request, as a page names the place to return to.
#[must_use]
pub fn path_and_query() -> String {
    parts()
        .and_then(|parts| parts.uri.path_and_query().map(ToString::to_string))
        .unwrap_or_else(|| "/".to_owned())
}

/// The query parameter `key` of the request, trimmed; `None` when absent or
/// empty.
#[must_use]
pub fn query(key: &str) -> Option<String> {
    let parts = parts()?;
    let query = parts.uri.query()?;
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
    let Some(parts) = parts() else {
        return Vec::new();
    };
    let Some(query) = parts.uri.query() else {
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

/// The viewer session the request's sealed cookie carries, when it carries
/// one that is still inside its idle window.
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
#[must_use]
pub fn state() -> Option<crate::state::AppState> {
    use_context::<crate::state::AppState>()
}

/// Sets the response status of the page being drawn.
///
/// A page that already ended in a redirect keeps it: a post that succeeded
/// keeps its `302` even when its page can no longer read its object.
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
    use super::return_path;

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
