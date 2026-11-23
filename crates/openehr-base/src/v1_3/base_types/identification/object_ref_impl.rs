// @generated-from-template templates/openehr-base/base_types/identification/object_ref_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! The `OBJECT_REF.namespace` check: "a string matching the standard regex
//! `[a-zA-Z][a-zA-Z0-9_.:/&?=+-]*`" (BASE
//! `org.openehr.base.base_types.object_ref.adoc` §Attributes).
//!
//! NOTE: BASE declares no invariant name for the rule; `Namespace_valid` is our
//! own label.

use super::object_ref::ObjectRefData;
use crate::validate::{InvariantViolation, Validate};

/// `true` when `ns` matches the openEHR namespace regex
/// `[a-zA-Z][a-zA-Z0-9_.:/&?=+-]*` (BASE `object_ref.adoc` §Attributes).
#[must_use]
pub fn namespace_valid(ns: &str) -> bool {
    let mut chars = ns.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    chars.all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(c, '_' | '.' | ':' | '/' | '&' | '?' | '=' | '+' | '-')
    })
}

impl Validate for ObjectRefData {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        if !namespace_valid(&self.namespace) {
            out.push(InvariantViolation::here(
                "Invariant Namespace_valid failed on type OBJECT_REF",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_regex() {
        assert!(namespace_valid("local"));
        assert!(namespace_valid("unknown"));
        assert!(namespace_valid("some.name-space_1"));
        assert!(!namespace_valid("1badhjklcd"));
        assert!(!namespace_valid("A*"));
        assert!(!namespace_valid(""));
    }
}
