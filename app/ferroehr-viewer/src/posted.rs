// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A form a page posts to itself, and the page drawn again around it.
//!
//! Every form the viewer draws is a plain `POST` to the page that draws it, so
//! it works with no WebAssembly (FerroUI `leptos-ui.md` §1 and §5). The server
//! answers the post by drawing the same page with the posted fields in
//! context: a page whose work succeeded sends the browser on with a redirect
//! and a flash toast, and a page whose work the CDR refused draws the refusal
//! beside the fields as they were posted, so a draft is never lost. A field
//! that can carry clinical content therefore never reaches a URL. The app
//! shares the form with the browser as the page's other request state is
//! shared ([`provide_posted`]), and the browser lets it go at the first
//! navigation within the page ([`FollowNavigation`]). No openEHR spec governs
//! this: our own design.

use std::sync::Arc;

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

/// The fields of the form the page answers, in the order they were posted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Vec<(String, String)>", into = "Vec<(String, String)>")]
pub struct Posted(Arc<[(String, String)]>);

impl From<Vec<(String, String)>> for Posted {
    fn from(fields: Vec<(String, String)>) -> Self {
        Self::new(fields)
    }
}

impl From<Posted> for Vec<(String, String)> {
    fn from(posted: Posted) -> Self {
        posted.0.to_vec()
    }
}

impl Posted {
    /// The fields `fields`, as the form posted them.
    #[must_use]
    pub fn new(fields: Vec<(String, String)>) -> Self {
        Self(Arc::from(fields))
    }

    /// The last value posted as `name`, with the form's `CR LF` line breaks
    /// read back as the `LF` the page holds (HTML Living Standard §4.10.21.8).
    #[must_use]
    pub fn get(&self, name: &str) -> Option<String> {
        self.0
            .iter()
            .rev()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.replace("\r\n", "\n"))
    }

    /// The value posted as `name`, trimmed, empty when absent.
    #[must_use]
    pub fn text(&self, name: &str) -> String {
        self.get(name)
            .map(|value| value.trim().to_owned())
            .unwrap_or_default()
    }

    /// Every value posted as `name`, in order.
    #[must_use]
    pub fn all(&self, name: &str) -> Vec<String> {
        self.0
            .iter()
            .filter(|(field, _)| field == name)
            .map(|(_, value)| value.replace("\r\n", "\n"))
            .collect()
    }

    /// Whether a check box named `name` was posted checked.
    #[must_use]
    pub fn checked(&self, name: &str) -> bool {
        self.0
            .iter()
            .any(|(field, value)| field == name && value != "off")
    }

    /// What the post asks the page to do: the `intent` its submitter posted,
    /// else the form's `default-intent`.
    #[must_use]
    pub fn intent(&self) -> String {
        // NOTE: a form's default rides as `default-intent`, so the button that submitted it, which
        // may sit outside it in tree order (`form=`), always wins whatever the field order.
        self.get("intent")
            .or_else(|| self.get("default-intent"))
            .map(|value| value.trim().to_owned())
            .unwrap_or_default()
    }

    /// Every field, in the order posted.
    #[must_use]
    pub fn fields(&self) -> &[(String, String)] {
        &self.0
    }
}

/// The form the page answers, shared by both halves until the first
/// navigation within the page.
#[derive(Debug, Clone)]
pub struct PostedNow(ArcRwSignal<Option<Posted>>);

impl PostedNow {
    /// Lets the form go: a page drawn after a navigation answers no post.
    pub fn navigated(&self) {
        if self.0.with_untracked(Option::is_some) {
            self.0.set(None);
        }
    }
}

/// Provides the form the page answers to both halves: the request's on the
/// server, the serialized page's in the browser.
pub fn provide_posted() {
    let posted = SharedValue::new(use_context::<Posted>).into_inner();
    provide_context(PostedNow(ArcRwSignal::new(posted)));
}

/// Lets the shared form go at the first navigation within the page.
///
/// It sits inside the router. Effects run in the browser alone, so the
/// server draws nothing for it.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn FollowNavigation() -> impl IntoView {
    let location = leptos_router::hooks::use_location();
    let now = use_context::<PostedNow>();
    Effect::watch(
        move || (location.pathname.get(), location.search.get()),
        move |_, _, _| {
            if let Some(now) = &now {
                now.navigated();
            }
        },
        false,
    );
}

/// The form the page answers, when the request posted one.
#[must_use]
pub fn posted() -> Option<Posted> {
    match use_context::<PostedNow>() {
        Some(PostedNow(now)) => now.get_untracked(),
        None => use_context::<Posted>(),
    }
}

/// The form the page answers, when the request posted one asking `intent`.
#[must_use]
pub fn posted_for(intent: &str) -> Option<Posted> {
    posted().filter(|posted| posted.intent() == intent)
}

#[cfg(test)]
mod tests {
    use super::{Posted, posted, provide_posted};

    #[test]
    fn the_shared_form_answers_until_the_first_navigation() {
        let owner = leptos::prelude::Owner::new();
        owner.with(|| {
            let form = Posted::new(vec![("intent".to_owned(), "save".to_owned())]);
            leptos::prelude::provide_context(form.clone());
            provide_posted();
            assert_eq!(Some(form), posted());
            let now = leptos::prelude::use_context::<super::PostedNow>();
            now.iter().for_each(super::PostedNow::navigated);
            assert_eq!(None, posted());
        });
    }

    #[test]
    fn the_form_crosses_to_the_browser_whole() {
        let form = Posted::new(vec![
            ("a".to_owned(), "one".to_owned()),
            ("a".to_owned(), "x\r\ny".to_owned()),
        ]);
        let json = serde_json::to_string(&form).unwrap();
        assert_eq!(form, serde_json::from_str::<Posted>(&json).unwrap());
    }

    #[test]
    fn the_last_value_of_a_field_wins_and_line_breaks_read_back_as_lf() {
        let posted = Posted::new(vec![
            ("a".to_owned(), "one".to_owned()),
            ("doc".to_owned(), "x\r\ny".to_owned()),
            ("a".to_owned(), " two ".to_owned()),
        ]);
        assert_eq!(Some(" two ".to_owned()), posted.get("a"));
        assert_eq!("two", posted.text("a"));
        assert_eq!(vec!["one".to_owned(), " two ".to_owned()], posted.all("a"));
        assert_eq!(Some("x\ny".to_owned()), posted.get("doc"));
        assert!(posted.get("missing").is_none());
    }

    #[test]
    fn the_submitters_intent_wins_over_the_forms_default_in_any_order() {
        let pair = |name: &str, value: &str| (name.to_owned(), value.to_owned());
        let before = Posted::new(vec![pair("intent", "save"), pair("default-intent", "run")]);
        assert_eq!(before.intent(), "save");
        let after = Posted::new(vec![pair("default-intent", "run"), pair("intent", "save")]);
        assert_eq!(after.intent(), "save");
        let bare = Posted::new(vec![pair("default-intent", "run")]);
        assert_eq!(bare.intent(), "run");
        assert_eq!(Posted::new(Vec::new()).intent(), "");
    }

    #[test]
    fn a_check_box_reads_as_checked_unless_it_posted_off() {
        let posted = Posted::new(vec![
            ("q".to_owned(), "on".to_owned()),
            ("m".to_owned(), "off".to_owned()),
        ]);
        assert!(posted.checked("q"));
        assert!(!posted.checked("m"));
        assert!(!posted.checked("absent"));
    }
}
