// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written BASE class check for `OBJECT_REF.namespace`.
//!
//! The rule is released text. BASE base_types
//! `UML/classes/org.openehr.base.base_types.object_ref.adoc` §Attributes
//! states it in the `namespace` Meaning row: "Legal values for `_namespace_`
//! are: `"local"`, `"unknown"`, a string matching the standard regex
//! `[a-zA-Z][a-zA-Z0-9_.:/&?=+-]*`. Note that the first two are just special
//! values of the regex, and will be matched by it."
//!
//! NOTE: BASE states the rule but declares no invariant NAME for it — the
//! `OBJECT_REF` class table has no §Invariants section at all.
//! `Namespace_valid` is the label this workspace gives the check so its
//! violation reads in the same uniform `Invariant <name> failed on type
//! <RM_TYPE>` shape every other class check uses; the label is our own
//! convention, not a released invariant name.

use super::object_ref::ObjectRefData;
use crate::validate::{InvariantViolation, Validate};

/// `true` when `ns` matches the openEHR namespace regex
/// `[a-zA-Z][a-zA-Z0-9_.:/&?=+-]*` — BASE base_types
/// `org.openehr.base.base_types.object_ref.adoc` §Attributes, the `namespace`
/// Meaning row.
///
/// This is the single realization of that rule in the workspace: `PartyRef`
/// (which inherits the `OBJECT_REF` constraint) and every consumer outside
/// this crate judge a namespace through it, so there is one definition to
/// maintain against the spec text.
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
