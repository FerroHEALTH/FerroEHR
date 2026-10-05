// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `C_DATE` spec functions.
//!
//! Spec sources (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.c_date.adoc` §Functions and
//! `AM/docs/AOM2/master04.2-constraint_model-semantics.adoc` §Date/Time
//! Constraints.

use crate::v2_4::aom2::constraint_model::primitive::c_date::CDate;
use crate::v2_4::aom2::constraint_model::primitive::c_temporal_definitions::CTemporalDefinitions;
use crate::v2_4::aom2::constraint_model::primitive::c_temporal_definitions_impl::{
    date_slots, slot_validity,
};
use openehr_base::v1_3::base_types::definitions::validity_kind::ValidityKind;

impl CDate {
    /// Returns the validity of the month field, when a pattern constrains it.
    ///
    /// `month_validity` (`org.openehr.am.aom2.c_date.adoc` §Functions) reads the
    /// second slot of `pattern_constraint` under the
    /// `master04.2-constraint_model-semantics.adoc` §Date/Time Constraints
    /// mapping (`??` → optional, `XX` → prohibited, field letters → mandatory).
    ///
    /// NOTE: the vendored AM text defines this result only over a pattern, and
    /// `pattern_constraint` is `0..1`, so a range-only constraint yields `None`.
    #[must_use]
    pub fn month_validity(&self) -> Option<ValidityKind> {
        self.pattern_slot(1)
    }

    /// Returns the validity of the day field, when a pattern constrains it.
    ///
    /// `day_validity` (`org.openehr.am.aom2.c_date.adoc` §Functions) — the third
    /// slot, under the same mapping as [`CDate::month_validity`].
    #[must_use]
    pub fn day_validity(&self) -> Option<ValidityKind> {
        self.pattern_slot(2)
    }

    /// Returns true if `a_pattern` is a valid date constraint pattern.
    ///
    /// `valid_pattern_constraint` (`org.openehr.am.aom2.c_date.adoc`
    /// §Functions): "Return `valid_iso8601_date_constraint_pattern
    /// (a_pattern)`".
    #[must_use]
    #[expect(
        clippy::unused_self,
        reason = "the AM class page declares this as an instance function of the constrainer type, so the receiver is part of the spec signature even though the answer depends only on the pattern"
    )]
    pub fn valid_pattern_constraint(&self, a_pattern: &str) -> bool {
        CTemporalDefinitions::default().valid_iso8601_date_constraint_pattern(a_pattern)
    }

    /// Returns true if `a_pattern` may replace `an_other_pattern` in a
    /// specialised constraint.
    ///
    /// `valid_pattern_constraint_replacement` (`org.openehr.am.aom2.c_date.adoc`
    /// §Functions): "Return `valid_date_constraint_replacements.has
    /// (an_other_pattern.as_upper) and then
    /// valid_date_constraint_replacements.item (an_other_pattern.as_upper).has
    /// (a_pattern.as_upper)`".
    #[must_use]
    #[expect(
        clippy::unused_self,
        reason = "the AM class page declares this as an instance function of the constrainer type, so the receiver is part of the spec signature even though the answer depends only on the two patterns"
    )]
    pub fn valid_pattern_constraint_replacement(
        &self,
        a_pattern: &str,
        an_other_pattern: &str,
    ) -> bool {
        CTemporalDefinitions::default()
            .valid_date_constraint_replacements
            .get(&an_other_pattern.to_uppercase())
            .is_some_and(|allowed| allowed.iter().any(|p| *p == a_pattern.to_uppercase()))
    }

    /// The validity of the `index`-th `-`-separated slot of the pattern.
    fn pattern_slot(&self, index: usize) -> Option<ValidityKind> {
        let pattern = self.pattern_constraint.as_ref()?;
        date_slots(pattern)
            .get(index)
            .map(|slot| slot_validity(slot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(pattern: Option<&str>) -> CDate {
        CDate {
            parent: None,
            soc_parent: None,
            rm_type_name: "DV_DATE".to_owned(),
            occurrences: None,
            node_id: "at9999".to_owned(),
            alternative_ids: None,
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value: None,
            is_enumerated_type_constraint: None,
            constraint: None,
            pattern_constraint: pattern.map(str::to_owned),
        }
    }

    #[test]
    fn the_pattern_slots_map_to_field_validities() {
        let d = date(Some("YYYY-??-XX"));
        assert_eq!(d.month_validity(), Some(ValidityKind::Optional));
        assert_eq!(d.day_validity(), Some(ValidityKind::Prohibited));
        let full = date(Some("YYYY-MM-DD"));
        assert_eq!(full.month_validity(), Some(ValidityKind::Mandatory));
        assert_eq!(full.day_validity(), Some(ValidityKind::Mandatory));
    }

    #[test]
    fn a_range_only_constraint_states_no_field_validity() {
        assert_eq!(date(None).month_validity(), None);
        assert_eq!(date(None).day_validity(), None);
    }

    #[test]
    fn only_declared_patterns_are_valid() {
        let d = date(None);
        assert!(d.valid_pattern_constraint("YYYY-MM-??"));
        assert!(!d.valid_pattern_constraint("YYYY-XX-DD"));
    }

    #[test]
    fn a_replacement_must_be_listed_under_the_pattern_it_narrows() {
        let d = date(None);
        assert!(d.valid_pattern_constraint_replacement("YYYY-MM-DD", "YYYY-MM-??"));
        assert!(!d.valid_pattern_constraint_replacement("YYYY-MM-??", "YYYY-MM-DD"));
        assert!(!d.valid_pattern_constraint_replacement("YYYY-MM-DD", "YYYY-XX-XX"));
    }
}
