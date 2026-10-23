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
//! that can carry clinical content therefore never reaches a URL. No openEHR
//! spec governs this: our own design.

use std::sync::Arc;

use leptos::prelude::use_context;

/// The fields of the form the page answers, in the order they were posted.
#[derive(Debug, Clone, Default)]
pub struct Posted(Arc<[(String, String)]>);

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

    /// What the post asks the page to do: its `intent` field.
    #[must_use]
    pub fn intent(&self) -> String {
        self.text("intent")
    }

    /// Every field, in the order posted.
    #[must_use]
    pub fn fields(&self) -> &[(String, String)] {
        &self.0
    }
}

/// The form the page answers, when the request posted one.
#[must_use]
pub fn posted() -> Option<Posted> {
    use_context::<Posted>()
}

/// The form the page answers, when the request posted one asking `intent`.
#[must_use]
pub fn posted_for(intent: &str) -> Option<Posted> {
    posted().filter(|posted| posted.intent() == intent)
}

#[cfg(test)]
mod tests {
    use super::Posted;

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
