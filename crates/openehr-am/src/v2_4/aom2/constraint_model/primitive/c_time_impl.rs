// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `C_TIME` spec functions.
//!
//! Spec sources (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.c_time.adoc` §Functions,
//! `AM/docs/AOM2/master04.2-constraint_model-semantics.adoc` §Date/Time
//! Constraints, and `AM/docs/ADL2/master04.5-cadl_primitive_types.adoc`
//! §Patterns.

use crate::v2_4::aom2::constraint_model::primitive::c_temporal_definitions::CTemporalDefinitions;
use crate::v2_4::aom2::constraint_model::primitive::c_temporal_definitions_impl::{
    slot_validity, time_slots, timezone_validity,
};
use crate::v2_4::aom2::constraint_model::primitive::c_time::CTime;
use openehr_base::v1_3::base_types::definitions::validity_kind::ValidityKind;

impl CTime {
    /// Returns the validity of the minute field, when a pattern constrains it.
    ///
    /// `minute_validity` (`org.openehr.am.aom2.c_time.adoc` §Functions) reads
    /// the second slot of `pattern_constraint` under the
    /// `master04.2-constraint_model-semantics.adoc` §Date/Time Constraints
    /// mapping (`??` → optional, `XX` → prohibited, field letters → mandatory).
    ///
    /// NOTE: the vendored AM text defines this result only over a pattern, and
    /// `pattern_constraint` is `0..1`, so a range-only constraint yields `None`.
    #[must_use]
    pub fn minute_validity(&self) -> Option<ValidityKind> {
        self.pattern_slot(1)
    }

    /// Returns the validity of the second field, when a pattern constrains it.
    ///
    /// `second_validity` (`org.openehr.am.aom2.c_time.adoc` §Functions) — the
    /// third slot, under the same mapping as [`CTime::minute_validity`].
    #[must_use]
    pub fn second_validity(&self) -> Option<ValidityKind> {
        self.pattern_slot(2)
    }

    /// Returns the validity of the timezone modifier.
    ///
    /// `timezone_validity` (`org.openehr.am.aom2.c_time.adoc` §Functions).
    /// `ADL2 master04.5` §Patterns: appending a `±hh` / `±hh:mm` / `±hhmm` / `Z`
    /// modifier makes a timezone required, "the absence of a timezone constraint
    /// indicates that a timezone modifier is optional", and "there is no way to
    /// state that timezone information be prohibited" — so this never returns
    /// `prohibited`. A constraint with no pattern states nothing at all.
    #[must_use]
    pub fn timezone_validity(&self) -> Option<ValidityKind> {
        self.pattern_constraint.as_deref().map(timezone_validity)
    }

    /// Returns true if `a_pattern` is a valid time constraint pattern.
    ///
    /// `valid_pattern_constraint` (`org.openehr.am.aom2.c_time.adoc`
    /// §Functions): "Return `valid_iso8601_time_constraint_pattern
    /// (a_pattern)`".
    #[must_use]
    #[expect(
        clippy::unused_self,
        reason = "the AM class page declares this as an instance function of the constrainer type, so the receiver is part of the spec signature even though the answer depends only on the pattern"
    )]
    pub fn valid_pattern_constraint(&self, a_pattern: &str) -> bool {
        CTemporalDefinitions::default().valid_iso8601_time_constraint_pattern(a_pattern)
    }

    /// Returns true if `a_pattern` may replace `an_other_pattern` in a
    /// specialised constraint.
    ///
    /// `valid_pattern_constraint_replacement` (`org.openehr.am.aom2.c_time.adoc`
    /// §Functions): "Return `valid_time_constraint_replacements.has
    /// (an_other_pattern.as_upper) and then
    /// valid_time_constraint_replacements.item (an_other_pattern.as_upper).has
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
            .valid_time_constraint_replacements
            .get(&an_other_pattern.to_uppercase())
            .is_some_and(|allowed| allowed.iter().any(|p| *p == a_pattern.to_uppercase()))
    }

    /// The validity of the `index`-th `:`-separated slot of the pattern.
    fn pattern_slot(&self, index: usize) -> Option<ValidityKind> {
        let pattern = self.pattern_constraint.as_ref()?;
        time_slots(pattern)
            .get(index)
            .map(|slot| slot_validity(slot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn time(pattern: Option<&str>) -> CTime {
        CTime {
            parent: None,
            soc_parent: None,
            rm_type_name: "DV_TIME".to_owned(),
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
        let t = time(Some("HH:??:XX"));
        assert_eq!(t.minute_validity(), Some(ValidityKind::Optional));
        assert_eq!(t.second_validity(), Some(ValidityKind::Prohibited));
        let full = time(Some("HH:MM:SS"));
        assert_eq!(full.minute_validity(), Some(ValidityKind::Mandatory));
        assert_eq!(full.second_validity(), Some(ValidityKind::Mandatory));
    }

    #[test]
    fn a_range_only_constraint_states_no_field_validity() {
        assert_eq!(time(None).minute_validity(), None);
        assert_eq!(time(None).timezone_validity(), None);
    }

    #[test]
    fn a_timezone_is_optional_until_a_modifier_demands_it() {
        assert_eq!(
            time(Some("HH:MM:SS")).timezone_validity(),
            Some(ValidityKind::Optional)
        );
        assert_eq!(
            time(Some("HH:MM:SS+HH:MM")).timezone_validity(),
            Some(ValidityKind::Mandatory)
        );
        assert_eq!(
            time(Some("HH:MM:SSZ")).timezone_validity(),
            Some(ValidityKind::Mandatory)
        );
    }

    #[test]
    fn the_slot_split_ignores_a_timezone_modifier() {
        let t = time(Some("HH:??:XX+HH:MM"));
        assert_eq!(t.minute_validity(), Some(ValidityKind::Optional));
        assert_eq!(t.second_validity(), Some(ValidityKind::Prohibited));
    }

    #[test]
    fn only_declared_patterns_and_listed_replacements_are_valid() {
        let t = time(None);
        assert!(t.valid_pattern_constraint("HH:??:XX"));
        assert!(!t.valid_pattern_constraint("HH:XX:SS"));
        assert!(t.valid_pattern_constraint_replacement("HH:MM:SS", "HH:MM:??"));
        assert!(!t.valid_pattern_constraint_replacement("HH:MM:??", "HH:MM:SS"));
    }
}
