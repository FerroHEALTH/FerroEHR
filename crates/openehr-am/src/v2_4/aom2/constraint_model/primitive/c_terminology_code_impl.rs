// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `C_TERMINOLOGY_CODE` spec functions.
//!
//! Spec sources (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.c_terminology_code.adoc` §Functions
//! and `AM/docs/AOM2/master04.2-constraint_model-semantics.adoc` §Terminology
//! Code Resolution.

use crate::v2_4::aom2::constraint_model::primitive::c_terminology_code::CTerminologyCode;
use crate::v2_4::aom2::constraint_model::primitive::constraint_status::ConstraintStatus;
use crate::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData;

impl CTerminologyCode {
    /// Returns true if any coded value would be allowed.
    ///
    /// `any_allowed` (`org.openehr.am.aom2.c_terminology_code.adoc`
    /// §Functions), post-condition `Result := constraint.is_empty`. The
    /// attribute is a mandatory `String` whose "empty string" spelling the same
    /// page declares as the no-constraint form.
    #[must_use]
    pub fn any_allowed(&self) -> bool {
        self.constraint.is_empty()
    }

    /// Returns true if the constraint is a formally required one.
    ///
    /// `constraint_required` (`org.openehr.am.aom2.c_terminology_code.adoc`
    /// §Functions): "True if `constraint_status` is defined and equals
    /// `required` OR if Void. I.e. in archetypes where `C_TERMINOLOGY_CODE`
    /// instances have no `constraint_status`, the `required` status is assumed,
    /// which applies to all legacy archetypes."
    #[must_use]
    pub fn constraint_required(&self) -> bool {
        self.constraint_status
            .is_none_or(|status| status == ConstraintStatus::Required)
    }

    /// Returns the effective integer constraint status.
    ///
    /// `effective_constraint_status`
    /// (`org.openehr.am.aom2.c_terminology_code.adoc` §Functions): "Return the
    /// effective integer value of the `constraint_status` field if it exists.
    /// If it is null, return 0, i.e. `required`."
    #[must_use]
    pub fn effective_constraint_status(&self) -> i32 {
        self.constraint_status.map_or(0, ConstraintStatus::value)
    }

    /// Returns true if this node's value constraint conforms to `other`'s.
    ///
    /// `c_value_conforms_to` (`master04.5` §Conformance semantics:
    /// C_TERMINOLOGY_NODE): an `any_allowed` parent admits anything; a child
    /// `effective_constraint_status` above the parent's refuses (the ordering
    /// `required (0) → extensible (1) → preferred (2) → example (3)`, child
    /// numerically `<=` parent); a non-`required` parent "automatically
    /// conforms"; otherwise the codes must be `codes_conformant`.
    ///
    /// NOTE: the value-set half of the both-`required` branch compares
    /// `value_set_expanded` of the two nodes, which is resolved against the
    /// owning archetype's terminology and so is applied by the caller holding
    /// both flattened terminologies, not here.
    #[must_use]
    pub fn c_value_conforms_to(&self, other: &CTerminologyCode) -> bool {
        if other.any_allowed() {
            return true;
        }
        if self.effective_constraint_status() > other.effective_constraint_status() {
            return false;
        }
        if other.effective_constraint_status() > 0 {
            return true;
        }
        AdlCodeDefinitionsData::codes_conformant(&self.constraint, &other.constraint)
    }

    /// Returns true if this node's value constraint is the same as `other`'s.
    ///
    /// `c_value_congruent_to` (`master04.5` §Conformance semantics:
    /// C_TERMINOLOGY_NODE): equal `constraint` and equal
    /// `effective_constraint_status`, with the same value-set-expansion caveat
    /// as [`CTerminologyCode::c_value_conforms_to`].
    #[must_use]
    pub fn c_value_congruent_to(&self, other: &CTerminologyCode) -> bool {
        self.constraint == other.constraint
            && self.effective_constraint_status() == other.effective_constraint_status()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(constraint: &str, status: Option<ConstraintStatus>) -> CTerminologyCode {
        CTerminologyCode {
            parent: None,
            soc_parent: None,
            rm_type_name: "DV_CODED_TEXT".to_owned(),
            occurrences: None,
            node_id: "at9999".to_owned(),
            alternative_ids: None,
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value: None,
            is_enumerated_type_constraint: None,
            constraint: constraint.to_owned(),
            constraint_status: status,
        }
    }

    #[test]
    fn an_empty_constraint_string_allows_any_code() {
        assert!(code("", None).any_allowed());
        assert!(!code("ac1", None).any_allowed());
    }

    #[test]
    fn an_unstated_status_means_required() {
        assert_eq!(
            code("ac1", None).effective_constraint_status(),
            ConstraintStatus::Required.value()
        );
        assert!(code("ac1", None).constraint_required());
    }

    #[test]
    fn a_weaker_status_is_not_required() {
        let preferred = code("ac1", Some(ConstraintStatus::Preferred));
        assert_eq!(
            preferred.effective_constraint_status(),
            ConstraintStatus::Preferred.value()
        );
        assert!(!preferred.constraint_required());
        assert!(code("ac1", Some(ConstraintStatus::Required)).constraint_required());
    }
}
