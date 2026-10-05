// @generated-from-template templates/openehr-rm/data_types/uri/dv_ehr_uri_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariants for `DV_EHR_URI`.
//!
//! - `Scheme_valid` (`scheme.is_equal (Ehr_scheme)`) —
//!   `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.dv_ehr_uri.adoc`
//!   §Invariants. `Ehr_scheme` is `"ehr"`
//!   (`docs/specs/openehr/RM/docs/data_types/master10-uri_package.adoc`
//!   §Definitions), compared case-insensitively per RFC 3986 §3.1
//!   (<https://www.rfc-editor.org/rfc/rfc3986#section-3.1>): "schemes are
//!   case-insensitive".
//! - `Value_valid` (`not value.is_empty`) — inherited from `DV_URI`
//!   (`…org.openehr.rm.data_types.dv_uri.adoc` §Invariants).
//!
//! An empty value fails both: it has no scheme either.

use crate::v1_2::data_types::uri::dv_ehr_uri::DvEhrUri;
use openehr_base::validate::{InvariantViolation, Validate};

/// The scheme is the substring before the first `:` (RFC-3986 `scheme:` prefix).
fn scheme(value: &str) -> Option<&str> {
    value.split_once(':').map(|(s, _)| s)
}

/// The `Scheme_valid` + `Value_valid` core over the projected input — one
/// source for the typed impl and the value-level fast path (`validate::fast`).
pub(crate) fn push_dv_ehr_uri_invariants(value: &str, out: &mut Vec<InvariantViolation>) {
    if !scheme(value).is_some_and(|s| s.eq_ignore_ascii_case("ehr")) {
        out.push(InvariantViolation::here(
            "Invariant Scheme_valid failed on type DV_EHR_URI",
        ));
    }
    if value.is_empty() {
        out.push(InvariantViolation::here(
            "Invariant Value_valid failed on type DV_EHR_URI",
        ));
    }
}

impl Validate for DvEhrUri {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        push_dv_ehr_uri_invariants(&self.value, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ehr_uri(value: &str) -> Vec<InvariantViolation> {
        DvEhrUri {
            value: value.to_owned(),
        }
        .invariants()
    }

    #[test]
    fn valid() {
        assert!(ehr_uri("ehr://something/something").is_empty());
    }

    #[test]
    fn wrong_scheme() {
        let v = ehr_uri("https://something/something");
        assert_eq!(v.len(), 1);
        assert_eq!(
            v[0].message,
            "Invariant Scheme_valid failed on type DV_EHR_URI"
        );
    }

    #[test]
    fn no_scheme() {
        let v = ehr_uri("target1");
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Scheme_valid failed on type DV_EHR_URI")
        );
    }

    #[test]
    fn empty_fails_both() {
        let msgs: Vec<_> = ehr_uri("").into_iter().map(|m| m.message).collect();
        assert!(msgs.contains(&"Invariant Scheme_valid failed on type DV_EHR_URI".to_owned()));
        assert!(msgs.contains(&"Invariant Value_valid failed on type DV_EHR_URI".to_owned()));
    }
}
