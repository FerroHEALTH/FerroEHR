// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `C_DATE_TIME` spec functions.
//!
//! Spec sources (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.c_date_time.adoc` §Functions,
//! `AM/docs/AOM2/master04.2-constraint_model-semantics.adoc` §Date/Time
//! Constraints, and `AM/docs/ADL2/master04.5-cadl_primitive_types.adoc`
//! §Patterns.

use crate::v2_4::aom2::constraint_model::primitive::c_date_time::CDateTime;
use crate::v2_4::aom2::constraint_model::primitive::c_temporal_definitions::CTemporalDefinitions;
use crate::v2_4::aom2::constraint_model::primitive::c_temporal_definitions_impl::{
    date_slots, slot_validity, time_slots, timezone_validity,
};
use openehr_base::v1_3::base_types::definitions::validity_kind::ValidityKind;

impl CDateTime {
    /// Returns the validity of the month field, when a pattern constrains it.
    ///
    /// `month_validity` (`org.openehr.am.aom2.c_date_time.adoc` §Functions) —
    /// the second `-`-separated slot of the date half of `pattern_constraint`,
    /// under the `master04.2-constraint_model-semantics.adoc` §Date/Time
    /// Constraints mapping.
    ///
    /// NOTE: the vendored AM text defines this result only over a pattern, and
    /// `pattern_constraint` is `0..1`, so a range-only constraint yields `None`.
    #[must_use]
    pub fn month_validity(&self) -> Option<ValidityKind> {
        self.date_slot(1)
    }

    /// Returns the validity of the day field, when a pattern constrains it.
    ///
    /// `day_validity` (`org.openehr.am.aom2.c_date_time.adoc` §Functions) — the
    /// third slot of the date half.
    #[must_use]
    pub fn day_validity(&self) -> Option<ValidityKind> {
        self.date_slot(2)
    }

    /// Returns the validity of the minute field, when a pattern constrains it.
    ///
    /// `minute_validity` (`org.openehr.am.aom2.c_date_time.adoc` §Functions) —
    /// the second `:`-separated slot of the time half.
    #[must_use]
    pub fn minute_validity(&self) -> Option<ValidityKind> {
        self.time_slot(1)
    }

    /// Returns the validity of the second field, when a pattern constrains it.
    ///
    /// `second_validity` (`org.openehr.am.aom2.c_date_time.adoc` §Functions) —
    /// the third slot of the time half.
    #[must_use]
    pub fn second_validity(&self) -> Option<ValidityKind> {
        self.time_slot(2)
    }

    /// Returns the validity of the timezone modifier.
    ///
    /// `timezone_validity` (`org.openehr.am.aom2.c_date_time.adoc` §Functions).
    /// `ADL2 master04.5` §Patterns: an appended `±hh` / `±hh:mm` / `±hhmm` / `Z`
    /// modifier makes a timezone required, absence makes it optional, and
    /// prohibition is unstateable — so this never returns `prohibited`.
    #[must_use]
    pub fn timezone_validity(&self) -> Option<ValidityKind> {
        self.pattern_constraint.as_deref().map(timezone_validity)
    }

    /// Returns true if `a_pattern` is a valid date/time constraint pattern.
    ///
    /// `valid_pattern_constraint` (`org.openehr.am.aom2.c_date_time.adoc`
    /// §Functions): "Return `valid_iso8601_date_time_constraint_pattern
    /// (a_pattern)`".
    #[must_use]
    #[expect(
        clippy::unused_self,
        reason = "the AM class page declares this as an instance function of the constrainer type, so the receiver is part of the spec signature even though the answer depends only on the pattern"
    )]
    pub fn valid_pattern_constraint(&self, a_pattern: &str) -> bool {
        CTemporalDefinitions::default().valid_iso8601_date_time_constraint_pattern(a_pattern)
    }

    /// Returns true if `a_pattern` may replace `an_other_pattern` in a
    /// specialised constraint.
    ///
    /// `valid_pattern_constraint_replacement`
    /// (`org.openehr.am.aom2.c_date_time.adoc` §Functions): "Return
    /// `valid_date_time_constraint_replacements.has (an_other_pattern.as_upper)
    /// and then valid_date_time_constraint_replacements.item
    /// (an_other_pattern.as_upper).has (a_pattern.as_upper)`".
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
            .valid_date_time_constraint_replacements
            .get(&an_other_pattern.to_uppercase())
            .is_some_and(|allowed| allowed.iter().any(|p| *p == a_pattern.to_uppercase()))
    }

    /// The validity of the `index`-th slot of the pattern's date half.
    fn date_slot(&self, index: usize) -> Option<ValidityKind> {
        let pattern = self.pattern_constraint.as_ref()?;
        let (date, _) = pattern.split_once(['T', 't'])?;
        date_slots(date).get(index).map(|slot| slot_validity(slot))
    }

    /// The validity of the `index`-th slot of the pattern's time half.
    fn time_slot(&self, index: usize) -> Option<ValidityKind> {
        let pattern = self.pattern_constraint.as_ref()?;
        let (_, time) = pattern.split_once(['T', 't'])?;
        time_slots(time).get(index).map(|slot| slot_validity(slot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date_time(pattern: Option<&str>) -> CDateTime {
        CDateTime {
            parent: None,
            soc_parent: None,
            rm_type_name: "DV_DATE_TIME".to_owned(),
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
    fn both_halves_of_the_pattern_are_read() {
        let d = date_time(Some("YYYY-MM-DDTHH:??:XX"));
        assert_eq!(d.month_validity(), Some(ValidityKind::Mandatory));
        assert_eq!(d.day_validity(), Some(ValidityKind::Mandatory));
        assert_eq!(d.minute_validity(), Some(ValidityKind::Optional));
        assert_eq!(d.second_validity(), Some(ValidityKind::Prohibited));
    }

    #[test]
    fn the_minimum_pattern_makes_every_field_optional() {
        let d = date_time(Some("YYYY-??-??T??:??:??"));
        assert_eq!(d.month_validity(), Some(ValidityKind::Optional));
        assert_eq!(d.day_validity(), Some(ValidityKind::Optional));
        assert_eq!(d.minute_validity(), Some(ValidityKind::Optional));
        assert_eq!(d.second_validity(), Some(ValidityKind::Optional));
    }

    #[test]
    fn a_range_only_constraint_states_no_field_validity() {
        assert_eq!(date_time(None).month_validity(), None);
        assert_eq!(date_time(None).minute_validity(), None);
        assert_eq!(date_time(None).timezone_validity(), None);
    }

    #[test]
    fn a_timezone_is_optional_until_a_modifier_demands_it() {
        assert_eq!(
            date_time(Some("YYYY-MM-DDTHH:MM:SS")).timezone_validity(),
            Some(ValidityKind::Optional)
        );
        assert_eq!(
            date_time(Some("YYYY-MM-DDTHH:MM:SS+HH")).timezone_validity(),
            Some(ValidityKind::Mandatory)
        );
    }

    #[test]
    fn only_declared_patterns_and_listed_replacements_are_valid() {
        let d = date_time(None);
        assert!(d.valid_pattern_constraint("YYYY-MM-DDTHH:MM:??"));
        assert!(!d.valid_pattern_constraint("YYYY-MM-DDTHH:XX:SS"));
        assert!(
            d.valid_pattern_constraint_replacement("YYYY-MM-DDTHH:MM:XX", "YYYY-MM-DDTHH:??:XX")
        );
        assert!(
            !d.valid_pattern_constraint_replacement("YYYY-MM-DDTHH:??:XX", "YYYY-MM-DDTHH:MM:XX")
        );
    }
}
