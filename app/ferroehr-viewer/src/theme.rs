// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The operator's theme: light, dark or system.
//!
//! The choice is kept in a server cookie and written by the server as
//! `data-theme` on `<html>`, so the first paint is already in the chosen
//! theme and no browser storage holds it.
//!
//! The FerroUI stylesheet reads `data-theme` (`system` follows
//! `prefers-color-scheme`); the brand's accent is `style/theme.css`. The thaw
//! widgets that FerroUI does not draw yet take the matching thaw theme from
//! [`thaw_theme`]. No openEHR spec governs this: our own design.

use std::collections::HashMap;

use ferrohealth_ui::theme::Theme;

/// The cookie the operator's theme choice is kept in.
pub const THEME_COOKIE: &str = "ferroehr-theme";

/// The route a theme choice posts to.
pub const THEME_ROUTE: &str = "/theme";

/// The theme the `Cookie` header value `cookies` names, the system theme for
/// none.
#[must_use]
pub fn theme_of(cookies: &str) -> Theme {
    // NOTE: RFC 6265 §5.4: cookies arrive as `name=value` pairs joined by `; `, and
    // a value that names no theme is the choice not made.
    cookies
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == THEME_COOKIE)
        .and_then(|(_, value)| Theme::parse(value))
        .unwrap_or_default()
}

/// The theme the page is drawn in: on the server the request's cookie, in the
/// browser the `data-theme` the server wrote on `<html>`, so both halves agree.
#[must_use]
pub fn current_theme() -> Theme {
    #[cfg(not(target_arch = "wasm32"))]
    {
        leptos::prelude::use_context::<http::request::Parts>()
            .and_then(|parts| {
                parts
                    .headers
                    .get(http::header::COOKIE)
                    .and_then(|value| value.to_str().ok())
                    .map(theme_of)
            })
            .unwrap_or_default()
    }
    #[cfg(target_arch = "wasm32")]
    {
        leptos::prelude::document()
            .document_element()
            .and_then(|root| root.get_attribute("data-theme"))
            .and_then(|key| Theme::parse(&key))
            .unwrap_or_default()
    }
}

/// The page `back` names when it is one of the viewer's own, plain path
/// segments of visible ASCII, and `/` for anything else, so a theme post never
/// redirects off the viewer.
#[must_use]
pub fn return_path(back: &str) -> &str {
    let plain = back.starts_with('/')
        && !back.starts_with("//")
        && back
            .chars()
            .all(|c| c.is_ascii_graphic() && !matches!(c, '\\' | '?' | '#'))
        && !back.split('/').any(|segment| segment == "..");
    if plain { back } else { "/" }
}

/// The teal brand ramp, Fluent variant keys 10 (darkest) → 160 (lightest);
/// variant 80 is the primary accent.
const BRAND: [(i32, &str); 16] = [
    (10, "#031b19"),
    (20, "#042f2e"),
    (30, "#0a3f3c"),
    (40, "#134e4a"),
    (50, "#115e59"),
    (60, "#0f766e"),
    (70, "#0e857b"),
    (80, "#0d9488"),
    (90, "#14b8a6"),
    (100, "#2dd4bf"),
    (110, "#48ddc8"),
    (120, "#5eead4"),
    (130, "#7cf0dc"),
    (140, "#99f6e4"),
    (150, "#b3f9ea"),
    (160, "#ccfbf1"),
];

/// The thaw widget theme matching `theme`; the system theme draws them light,
/// since the server cannot know what the system asks for.
#[must_use]
pub fn thaw_theme(theme: Theme) -> thaw::Theme {
    let ramp: HashMap<i32, &'static str> = BRAND.into_iter().collect();
    match theme {
        Theme::Dark => thaw::Theme::custom_dark(&ramp),
        Theme::Light | Theme::System => thaw::Theme::custom_light(&ramp),
    }
}

#[cfg(test)]
mod tests {
    use ferrohealth_ui::theme::Theme;

    use super::{return_path, theme_of};

    #[test]
    fn the_cookie_names_the_theme_and_anything_else_is_the_system() {
        assert_eq!(Theme::Dark, theme_of("a=b; ferroehr-theme=dark"));
        assert_eq!(Theme::Light, theme_of("ferroehr-theme=light"));
        assert_eq!(Theme::System, theme_of("ferroehr-theme=sepia"));
        assert_eq!(Theme::System, theme_of(""));
    }

    #[test]
    fn a_return_path_never_leaves_the_viewer() {
        assert_eq!("/ehrs", return_path("/ehrs"));
        for hostile in [
            "//evil.example",
            "https://evil.example",
            "/a/../b",
            "/a?b",
            "",
        ] {
            assert_eq!("/", return_path(hostile), "{hostile}");
        }
    }
}
