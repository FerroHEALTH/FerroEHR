// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `C_STRING` spec functions.
//!
//! Spec sources (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.c_string.adoc` §Functions,
//! `AM/docs/ADL2/master04.5-cadl_primitive_types.adoc` §Regular Expressions,
//! and `AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §Conformance semantics: C_STRING.

use crate::v2_4::aom2::constraint_model::primitive::c_string::CString;

impl CString {
    /// Returns true if any String value would be allowed.
    ///
    /// `any_allowed` (`org.openehr.am.aom2.c_string.adoc` §Functions),
    /// post-condition `Result = constraint.is_empty or else constraint.count = 1
    /// and constraint.first.is_equal (Regex_any_string)`.
    ///
    /// NOTE: the vendored AM text names `Regex_any_string` but declares it
    /// nowhere, so the pattern is taken from
    /// `ADL2 master04.5-cadl_primitive_types.adoc` §Regular Expressions, whose
    /// table gives `.*` as the expression that "matches any string".
    #[must_use]
    pub fn any_allowed(&self) -> bool {
        match self.constraint.as_deref() {
            None | Some([]) => true,
            Some([only]) => only == REGEX_ANY_STRING,
            Some(_) => false,
        }
    }

    /// Returns true if this node's `constraint` is a strict subset of
    /// `other.constraint`.
    ///
    /// `c_value_conforms_to` (`master04.5` §Conformance semantics: C_STRING):
    /// `other.any_allowed or constraint.count < other.constraint.count and for
    /// all c in constraint | other.constraint.has (c)`. Constraint items are
    /// compared literally — a regular expression is narrower than another only
    /// when the parent lists it, since regex containment is undecidable in
    /// general.
    #[must_use]
    pub fn c_value_conforms_to(&self, other: &CString) -> bool {
        other.any_allowed()
            || (self.values().len() < other.values().len()
                && self
                    .values()
                    .iter()
                    .all(|value| other.values().contains(value)))
    }

    /// Returns true if this node's value constraint is the same as `other`'s.
    ///
    /// `c_value_congruent_to` (`master04.5` §Conformance semantics: C_STRING):
    /// `constraint.count = other.constraint.count and then across constraint as
    /// str_csr all other.constraint.i_th (str_csr.cursor_index).is_equal
    /// (str_csr.item)`, i.e. equal item-by-item in declaration order.
    #[must_use]
    pub fn c_value_congruent_to(&self, other: &CString) -> bool {
        self.values() == other.values()
    }

    /// The stated constraint values, empty when none is stated.
    fn values(&self) -> &[String] {
        self.constraint.as_deref().unwrap_or_default()
    }
}

/// The regular expression that matches any string
/// (`ADL2 master04.5-cadl_primitive_types.adoc` §Regular Expressions).
const REGEX_ANY_STRING: &str = ".*";

#[cfg(test)]
mod tests {
    use super::*;

    fn string(constraint: Option<Vec<String>>) -> CString {
        CString {
            parent: None,
            soc_parent: None,
            rm_type_name: "String".to_owned(),
            occurrences: None,
            node_id: "at9999".to_owned(),
            alternative_ids: None,
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value: None,
            is_enumerated_type_constraint: None,
            constraint,
        }
    }

    #[test]
    fn an_unstated_constraint_allows_any_value() {
        assert!(string(None).any_allowed());
        assert!(string(Some(Vec::new())).any_allowed());
    }

    #[test]
    fn the_lone_any_string_regex_also_allows_any_value() {
        assert!(string(Some(vec![".*".to_owned()])).any_allowed());
    }

    #[test]
    fn a_narrower_or_longer_constraint_list_does_not() {
        assert!(!string(Some(vec!["[a-z]+".to_owned()])).any_allowed());
        assert!(!string(Some(vec![".*".to_owned(), "[a-z]+".to_owned()])).any_allowed());
    }
}
