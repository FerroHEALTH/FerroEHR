// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The outcome of a form post, carried across its redirect as a flash toast.
//!
//! A page whose work succeeded answers its post with a redirect (post,
//! redirect, get) and keeps what it did in a short-lived cookie; the next page
//! hands it to FerroUI's toast region as a flash toast, which shows with no
//! script, and clears the cookie in the same response (FerroUI
//! `docs/consoles.md`, "How a console builds", step 5). The cookie holds the
//! toast's words, never patient data. No openEHR spec governs this: our own
//! design.

use base64::Engine as _;
use ferrohealth_ui::tone::Tone;
use ferrohealth_ui::view::toast::{Flash, ToastLink};
use serde::{Deserialize, Serialize};

/// The cookie a flash toast crosses its redirect in.
pub const FLASH_COOKIE: &str = "ferroehr-viewer-flash";

/// A toast a post's outcome raises on the next page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Done {
    /// Its tone.
    pub tone: Tone,
    /// Its title, in the page's locale.
    pub title: String,
    /// Its line, in the page's locale.
    pub line: Option<String>,
    /// Its link: the words and the viewer page it opens.
    pub link: Option<(String, String)>,
}

impl Done {
    /// A toast in the ok tone saying `title`.
    #[must_use]
    pub fn ok(title: impl Into<String>) -> Self {
        Self {
            tone: Tone::Ok,
            title: title.into(),
            line: None,
            link: None,
        }
    }

    /// The same toast with the line `line`.
    #[must_use]
    pub fn line(mut self, line: impl Into<String>) -> Self {
        self.line = Some(line.into());
        self
    }

    /// The same toast with a link saying `words` to the viewer page `href`.
    #[must_use]
    pub fn link(mut self, words: impl Into<String>, href: impl Into<String>) -> Self {
        self.link = Some((words.into(), href.into()));
        self
    }

    /// The toast as FerroUI's region draws it.
    #[must_use]
    pub fn flash(self) -> Flash {
        Flash {
            tone: self.tone,
            title: self.title,
            line: self.line,
            action: self.link.map(|(label, href)| ToastLink { label, href }),
        }
    }

    /// The cookie value that carries the toast.
    ///
    /// # Panics
    ///
    /// Never: a toast of strings and a tone always serializes.
        #[must_use]
    #[expect(
        clippy::expect_used,
        reason = "serde_json serializes a struct of strings, a tone and an optional pair of strings without error: no map with non-string keys and no failing Serialize impl is reachable"
    )]
    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("a toast should serialize to JSON");
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)
    }

    /// The toast a cookie value carries; `None` for a value no toast wrote.
        #[must_use]
    pub fn decode(value: &str) -> Option<Self> {
        let json = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(value.trim())
            .ok()?;
        serde_json::from_slice(&json).ok()
    }
}

/// Ends the page being drawn with a redirect to the viewer page `to`, the
/// toast `done` waiting for it.
pub fn redirect_with(to: &str, done: &Done) {
    crate::request::set_cookie(&format!(
        "{FLASH_COOKIE}={}; Path=/; SameSite=Lax; HttpOnly; Max-Age=60",
        done.encode()
    ));
    crate::request::redirect(to);
}

/// The toast waiting for the page being drawn, cleared in the same response.
#[must_use]
pub fn take() -> Vec<Flash> {
    let cookies = crate::request::cookies();
    let Some(value) = ferrohealth_ui::theme::cookie(&cookies, FLASH_COOKIE) else {
        return Vec::new();
    };
    crate::request::set_cookie(&format!(
        "{FLASH_COOKIE}=; Path=/; SameSite=Lax; HttpOnly; Max-Age=0"
    ));
    Done::decode(value).map(Done::flash).into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::Done;

    #[test]
    fn a_toast_crosses_its_redirect_whole() {
        let done = Done::ok("Committed")
            .line("Version 3")
            .link("Open", "/ehrs/x");
        assert_eq!(Some(done.clone()), Done::decode(&done.encode()));
        assert_eq!(None, Done::decode("not-a-toast"));
    }

    #[test]
    fn the_cookie_value_is_a_cookie_octet_string() {
        let value = Done::ok("Ünïcode; and = signs").encode();
        assert!(
            value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "{value}"
        );
    }
}
