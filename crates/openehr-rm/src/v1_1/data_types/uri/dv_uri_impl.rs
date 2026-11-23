// @generated-from-template templates/openehr-rm/data_types/uri/dv_uri_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! `DV_URI` (RM 1.2.0 `org.openehr.rm.data_types.dv_uri.adoc`): `Value_valid`
//! and the RFC 3986 accessors, each empty when the component is absent.

use crate::v1_1::data_types::uri::dv_uri::DvUriData;
use openehr_base::validate::{InvariantViolation, Validate};

/// `true` when `s` is a syntactically valid RFC-3986 scheme:
/// `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )`.
fn is_scheme(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// The `Value_valid` core shared by the typed impl and `validate::fast`.
pub(crate) fn push_dv_uri_invariants(value: &str, out: &mut Vec<InvariantViolation>) {
    // NOTE: `dv_uri.adoc` §Invariants, `master10-uri_package.adoc` §Design: the
    // only invariant is non-emptiness; plain-text URIs are allowed.
    crate::v1_1::validate::generated::dv_uri_core(value, out);
}

impl DvUriData {
    /// RM `DV_URI.scheme()`: the URI scheme (e.g. `ftp`, `mailto`), or the
    /// empty string for a scheme-less (relative) value.
    #[must_use]
    pub fn scheme(&self) -> &str {
        match self.value.split_once(':') {
            Some((s, _)) if is_scheme(s) => s,
            _ => "",
        }
    }

    /// RM `DV_URI.path()`: after `scheme:` and any `//authority`, up to `?` or
    /// `#`.
    #[must_use]
    pub fn path(&self) -> &str {
        let mut rest = self.value.as_str();
        if let Some((s, r)) = rest.split_once(':')
            && is_scheme(s)
        {
            rest = r;
        }
        if let Some(after) = rest.strip_prefix("//") {
            // Authority ends at the next '/', '?' or '#'.
            rest = after
                .find(['/', '?', '#'])
                .and_then(|i| after.get(i..))
                .unwrap_or_default();
        }
        rest.split(['?', '#']).next().unwrap_or("")
    }

    /// RM `DV_URI.query()`: the query string (between `?` and any `#`), or
    /// the empty string.
    #[must_use]
    pub fn query(&self) -> &str {
        self.value
            .split_once('?')
            .map_or("", |(_, q)| q.split('#').next().unwrap_or(""))
    }

    /// RM `DV_URI.fragment_id()`: the fragment (after `#`), or the empty
    /// string.
    #[must_use]
    pub fn fragment_id(&self) -> &str {
        self.value.split_once('#').map_or("", |(_, f)| f)
    }
}

impl Validate for DvUriData {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        push_dv_uri_invariants(&self.value, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(value: &str) -> DvUriData {
        DvUriData {
            value: value.to_owned(),
        }
    }

    fn messages(value: &str) -> Vec<String> {
        uri(value)
            .invariants()
            .into_iter()
            .map(|m| m.message)
            .collect()
    }

    #[test]
    fn value_valid() {
        assert_eq!(
            uri("http://example.org/x").invariants(),
            Vec::<InvariantViolation>::new()
        );
        // An empty value is both empty and scheme-less; both are `Value_valid`,
        // which is the only invariant `dv_uri.adoc` §Invariants declares.
        assert!(messages("").contains(&"Invariant Value_valid failed on type DV_URI".to_owned()));
    }

    /// An empty value fails `Value_valid`.
    #[test]
    fn scheme_less_and_plain_text_values_are_accepted() {
        // Absolute references are accepted.
        assert_eq!(
            uri("ftp://ftp.is.co.za/rfc/rfc1808.txt").invariants(),
            Vec::<InvariantViolation>::new()
        );
        assert_eq!(
            uri("mailto:someone@example.org").invariants(),
            Vec::<InvariantViolation>::new()
        );
        // Plain-text and scheme-less URIs are admitted (§Design).
        assert_eq!(
            uri("http://example.org/a b").invariants(),
            Vec::<InvariantViolation>::new()
        );
        assert_eq!(
            uri("see the attached guideline").invariants(),
            Vec::<InvariantViolation>::new()
        );
        assert_eq!(uri("xyz").invariants(), Vec::<InvariantViolation>::new());
        assert_eq!(
            uri("www.iana.org").invariants(),
            Vec::<InvariantViolation>::new()
        );
        assert_eq!(
            uri("content/items").invariants(),
            Vec::<InvariantViolation>::new()
        );
    }

    #[test]
    fn accessors_decompose_uri() {
        let u = uri("ftp://ftp.example.org/pub/images/image_01?fmt=png#frag");
        assert_eq!(u.scheme(), "ftp");
        assert_eq!(u.path(), "/pub/images/image_01");
        assert_eq!(u.query(), "fmt=png");
        assert_eq!(u.fragment_id(), "frag");
    }

    #[test]
    fn accessors_handle_absent_components() {
        let u = uri("mailto:someone@example.org");
        assert_eq!(u.scheme(), "mailto");
        assert_eq!(u.path(), "someone@example.org");
        assert_eq!(u.query(), "");
        assert_eq!(u.fragment_id(), "");

        // A relative (scheme-less) reference.
        let r = uri("content/items");
        assert_eq!(r.scheme(), "");
        assert_eq!(r.path(), "content/items");

        // An authority with no path.
        let a = uri("https://example.org");
        assert_eq!(a.scheme(), "https");
        assert_eq!(a.path(), "");
    }
}
