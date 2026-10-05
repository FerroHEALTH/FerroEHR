// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `C_BOOLEAN` spec functions.
//!
//! Spec sources (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.c_boolean.adoc` §Functions and
//! `AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §Conformance semantics: C_BOOLEAN.

use crate::v2_4::aom2::constraint_model::primitive::c_boolean::CBoolean;

impl CBoolean {
    /// Returns true if any Boolean value would be allowed.
    ///
    /// `any_allowed` (`org.openehr.am.aom2.c_boolean.adoc` §Functions),
    /// post-condition `Result = constraint.is_empty`. The attribute is `0..1`,
    /// so absent and present-but-empty both read as "no constraint stated".
    #[must_use]
    pub fn any_allowed(&self) -> bool {
        self.constraint.as_deref().is_none_or(<[bool]>::is_empty)
    }

    /// Returns true if this node's value constraint is a strict subset of
    /// `other`'s.
    ///
    /// `c_value_conforms_to` (`master04.5` §Conformance semantics: C_BOOLEAN):
    /// `other.any_allowed or constraint.count < other.constraint.count and for
    /// all c in constraint | other.constraint.has (c)`. An equal constraint is
    /// deliberately not "conformant" here — that case is
    /// [`CBoolean::c_value_congruent_to`].
    #[must_use]
    pub fn c_value_conforms_to(&self, other: &CBoolean) -> bool {
        other.any_allowed()
            || (self.values().len() < other.values().len() && self.values_are_subset_of(other))
    }

    /// Returns true if this node's value constraint is the same as `other`'s.
    ///
    /// `c_value_congruent_to` (`master04.5` §Conformance semantics: C_BOOLEAN):
    /// `constraint.count = other.constraint.count and for all c in constraint |
    /// other.constraint.has (c)`.
    #[must_use]
    pub fn c_value_congruent_to(&self, other: &CBoolean) -> bool {
        self.values().len() == other.values().len() && self.values_are_subset_of(other)
    }

    /// The stated constraint values, empty when none is stated.
    fn values(&self) -> &[bool] {
        self.constraint.as_deref().unwrap_or_default()
    }

    /// Whether every value this node allows is also allowed by `other`.
    fn values_are_subset_of(&self, other: &CBoolean) -> bool {
        self.values()
            .iter()
            .all(|value| other.values().iter().any(|permitted| permitted == value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boolean(constraint: Option<Vec<bool>>) -> CBoolean {
        CBoolean {
            parent: None,
            soc_parent: None,
            rm_type_name: "Boolean".to_owned(),
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
        assert!(boolean(None).any_allowed());
        assert!(boolean(Some(Vec::new())).any_allowed());
    }

    #[test]
    fn one_permitted_value_is_already_a_constraint() {
        assert!(!boolean(Some(vec![true])).any_allowed());
        assert!(!boolean(Some(vec![true, false])).any_allowed());
    }

    /// The `master04.5` body is a STRICT subset test, so an equal constraint is
    /// congruent rather than conformant.
    #[test]
    fn conformance_is_a_strict_narrowing_and_equality_is_congruence() {
        let both = boolean(Some(vec![true, false]));
        let only_true = boolean(Some(vec![true]));
        assert!(only_true.c_value_conforms_to(&both));
        assert!(!both.c_value_conforms_to(&only_true));
        assert!(!only_true.c_value_conforms_to(&only_true));
        assert!(only_true.c_value_congruent_to(&only_true));
        assert!(!only_true.c_value_congruent_to(&both));
        // An unconstrained parent admits anything.
        assert!(both.c_value_conforms_to(&boolean(None)));
    }
}
