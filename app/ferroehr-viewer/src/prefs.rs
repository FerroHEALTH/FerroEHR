// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The reader's preferences: each one its own cookie, read when a page is
//! drawn and changed by a plain form post (family rule 9, FerroUI
//! `design/COMPONENTS.md`).
//!
//! The family preferences are FerroUI's (`ferrohealth_ui::theme`): theme,
//! density, reduced motion, language, first day of the week and shortcut
//! hints, each under its cookie name after [`COOKIE_PREFIX`]. The viewer adds
//! one of its own, how long a revealed identifier stays shown
//! ([`REVEAL`]), inside the Reading tab. Every cookie is `SameSite=Lax`,
//! lives a year, holds no patient data, and is `Secure` whenever the session
//! cookie is. The sidebar's rail is kept the same way. No openEHR spec
//! governs the viewer's preferences: our own design.

use axum::extract::Form;
use axum::response::{IntoResponse, Redirect, Response};
use ferrohealth_ui::theme::{
    Density, Hints, Language, Motion, Preference, Preferences, Theme, WeekStart, cookie,
};
use http::StatusCode;
use http::header::SET_COOKIE;

/// What the viewer puts before each preference's cookie name, so the theme
/// lives in `ferroehr-viewer-theme`.
pub const COOKIE_PREFIX: &str = "ferroehr-viewer-";

/// The route every preference posts to, its value under its cookie's name.
pub const PREFERENCE_ROUTE: &str = "/preference";

/// The route Reset to defaults posts to.
pub const RESET_ROUTE: &str = "/preferences/reset";

/// The route the sidebar's Collapse and Expand post to.
pub const SIDEBAR_ROUTE: &str = "/sidebar";

/// The cookie the sidebar's state is kept in.
pub const SIDEBAR_COOKIE: &str = "ferroehr-viewer-sidebar";

/// The viewer's own preference: the seconds a revealed identifier stays shown.
pub const REVEAL: &str = "reveal";

/// The seconds a revealed identifier may stay shown, the default first.
pub const REVEAL_CHOICES: [u32; 4] = [30, 15, 60, 120];

/// The most seconds the viewer shows a revealed identifier, whatever the
/// cookie says.
pub const REVEAL_MAX: u32 = 120;

/// The cookies Reset to defaults clears: the family's and the viewer's own.
#[must_use]
pub fn reset_cookies() -> Vec<String> {
    Preferences::COOKIES
        .into_iter()
        .chain([REVEAL])
        .map(|name| format!("{COOKIE_PREFIX}{name}"))
        .collect()
}

/// The preferences the request keeps.
#[must_use]
pub fn current() -> Preferences {
    Preferences::read(&crate::request::cookies(), COOKIE_PREFIX)
}

/// The seconds a revealed identifier stays shown for this reader, capped at
/// [`REVEAL_MAX`] whatever the cookie says.
#[must_use]
pub fn reveal_seconds() -> u32 {
    reveal_of(&crate::request::cookies())
}

/// The reveal duration the `Cookie` header value `cookies` keeps, the default
/// for none or for a value the viewer does not offer.
#[must_use]
pub fn reveal_of(cookies: &str) -> u32 {
    cookie(cookies, &format!("{COOKIE_PREFIX}{REVEAL}"))
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|seconds| REVEAL_CHOICES.contains(seconds))
        .map_or(REVEAL_CHOICES[0], |seconds| seconds.min(REVEAL_MAX))
}

/// Whether the request asks for the sidebar's rail.
#[must_use]
pub fn rail() -> bool {
    cookie(&crate::request::cookies(), SIDEBAR_COOKIE) == Some("rail")
}

/// The value the cookie of the preference `name` keeps, when `value` is one
/// the viewer offers for it, and `None` for anything else.
#[must_use]
pub fn preference_value(name: &str, value: &str) -> Option<String> {
    let kept = |choice: Option<&str>| choice.map(str::to_owned);
    if name == Theme::COOKIE {
        kept(Theme::from_cookie(value).map(Theme::key))
    } else if name == Density::COOKIE {
        kept(Density::from_cookie(value).map(Density::key))
    } else if name == Motion::COOKIE {
        kept(Motion::from_cookie(value).map(Motion::key))
    } else if name == WeekStart::COOKIE {
        kept(WeekStart::from_cookie(value).map(WeekStart::key))
    } else if name == Hints::COOKIE {
        kept(Hints::from_cookie(value).map(Hints::key))
    } else if name == Language::COOKIE {
        let offered = |tag: &str| {
            crate::locale::shipped()
                .iter()
                .any(|locale| locale.tag() == tag)
        };
        Language::from_cookie(value)
            .filter(|language| language.tag().is_none_or(offered))
            .map(|language| language.cookie_value().to_owned())
    } else if name == REVEAL {
        value
            .parse::<u32>()
            .ok()
            .filter(|seconds| REVEAL_CHOICES.contains(seconds))
            .map(|seconds| seconds.to_string())
    } else {
        None
    }
}

/// Where a preference post returns: the page `back` names, and on Your
/// preferences the same page saying which preference it kept.
#[must_use]
pub fn preference_return(back: &str, name: &str) -> String {
    let page = crate::request::return_path(back);
    let path = page.split('?').next().unwrap_or(page);
    if path.starts_with("/preferences") {
        format!("{path}?saved={name}")
    } else {
        page.to_owned()
    }
}

/// The `Set-Cookie` value keeping `value` under `name` for a year.
fn kept_cookie(name: &str, value: &str, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{name}={value}; Path=/; SameSite=Lax; HttpOnly; Max-Age=31536000{secure}")
}

/// Whether the deployment marks its cookies `Secure`.
fn secure(state: &crate::state::AppState) -> bool {
    state.config.session.cookie_secure
}

/// The redirect to `location` that sets each of `cookies`.
fn redirect_with(location: &str, cookies: &[String]) -> Response {
    let mut response = Redirect::to(location).into_response();
    for value in cookies {
        match http::HeaderValue::from_str(value) {
            Ok(value) => {
                response.headers_mut().append(SET_COOKIE, value);
            }
            Err(_invalid) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }
    response
}

/// Keeps the one preference a plain form post chose in its cookie and sends
/// the browser back to the page it came from.
///
/// The post carries `back` and one preference under its cookie's name; a
/// switch posts `off` before its `on`, and the last value wins.
pub async fn choose_preference(
    axum::Extension(state): axum::Extension<crate::state::AppState>,
    Form(fields): Form<Vec<(String, String)>>,
) -> Response {
    let back = fields
        .iter()
        .rev()
        .find(|(name, _)| name == "back")
        .map_or("/", |(_, back)| back.as_str());
    let chosen = fields
        .iter()
        .filter(|(name, _)| name != "back")
        .collect::<Vec<_>>();
    let Some((name, value)) = chosen.last() else {
        return (StatusCode::BAD_REQUEST, "the post names no preference").into_response();
    };
    if chosen.iter().any(|(other, _)| other != name) {
        return (StatusCode::BAD_REQUEST, "a post keeps one preference").into_response();
    }
    let Some(value) = preference_value(name, value) else {
        return (
            StatusCode::BAD_REQUEST,
            "the preference or its value is not one the viewer offers",
        )
            .into_response();
    };
    let cookie = kept_cookie(&format!("{COOKIE_PREFIX}{name}"), &value, secure(&state));
    redirect_with(&preference_return(back, name), &[cookie])
}

/// What Reset to defaults posts.
#[derive(Debug, serde::Deserialize)]
pub struct Reset {
    /// The page to return to.
    back: String,
}

/// Clears every preference cookie and sends the browser back.
pub async fn reset_preferences(Form(reset): Form<Reset>) -> Response {
    let cleared = reset_cookies()
        .into_iter()
        .map(|name| format!("{name}=; Path=/; SameSite=Lax; HttpOnly; Max-Age=0"))
        .collect::<Vec<_>>();
    redirect_with(crate::request::return_path(&reset.back), &cleared)
}

/// What the sidebar's Collapse or Expand posts.
#[derive(Debug, serde::Deserialize)]
pub struct SidebarChoice {
    /// `rail` to fold the navigation, `full` to unfold it.
    sidebar: String,
    /// The page to return to.
    back: String,
}

/// Keeps the sidebar's state in its cookie and sends the browser back.
pub async fn choose_sidebar(
    axum::Extension(state): axum::Extension<crate::state::AppState>,
    Form(choice): Form<SidebarChoice>,
) -> Response {
    if !matches!(choice.sidebar.as_str(), "rail" | "full") {
        return (StatusCode::BAD_REQUEST, "the sidebar is not rail or full").into_response();
    }
    let cookie = kept_cookie(SIDEBAR_COOKIE, &choice.sidebar, secure(&state));
    redirect_with(crate::request::return_path(&choice.back), &[cookie])
}

#[cfg(test)]
mod tests {
    use super::{preference_return, preference_value, reveal_of};

    #[test]
    fn only_a_value_the_viewer_offers_is_kept() {
        assert_eq!(Some("dark".to_owned()), preference_value("theme", "dark"));
        assert_eq!(None, preference_value("theme", "sepia"));
        assert_eq!(Some("60".to_owned()), preference_value("reveal", "60"));
        assert_eq!(None, preference_value("reveal", "3600"));
        assert_eq!(None, preference_value("unknown", "x"));
    }

    #[test]
    fn a_revealed_identifier_stays_at_most_two_minutes() {
        assert_eq!(30, reveal_of(""));
        assert_eq!(120, reveal_of("ferroehr-viewer-reveal=120"));
        assert_eq!(30, reveal_of("ferroehr-viewer-reveal=999"));
    }

    #[test]
    fn a_post_from_your_preferences_returns_saying_what_it_kept() {
        assert_eq!(
            "/preferences/reading?saved=reveal",
            preference_return("/preferences/reading?saved=theme", "reveal")
        );
        assert_eq!("/ehrs", preference_return("/ehrs", "theme"));
        assert_eq!("/", preference_return("https://evil.example", "theme"));
    }
}
