// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `C_DURATION` spec functions.
//!
//! Spec sources (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.c_duration.adoc` §Description +
//! §Functions, `AM/docs/AOM2/master04.2-constraint_model-semantics.adoc`
//! §Duration Constraints, and `AM/docs/ADL2/master04.5-cadl_primitive_types.adoc`
//! §Duration Constraints.

use crate::v2_4::aom2::constraint_model::primitive::c_duration::CDuration;
use crate::v2_4::aom2::constraint_model::primitive::c_temporal_definitions::CTemporalDefinitions;

impl CDuration {
    /// Returns true if years may appear in the constrained duration.
    ///
    /// `years_allowed` (`org.openehr.am.aom2.c_duration.adoc` §Functions) over
    /// the `P[Y|y][M|m][W|w][D|d][T[H|h][M|m][S|s]]` pattern the same page
    /// states. With no `pattern_constraint` the pattern states nothing, so
    /// every ISO 8601 slot remains allowed.
    #[must_use]
    pub fn years_allowed(&self) -> bool {
        self.date_designator('Y')
    }

    /// Returns true if months may appear in the constrained duration.
    ///
    /// `months_allowed` (`org.openehr.am.aom2.c_duration.adoc` §Functions) —
    /// the `M` designator BEFORE the `T` separator, which the pattern grammar
    /// distinguishes from the minutes `M` after it.
    #[must_use]
    pub fn months_allowed(&self) -> bool {
        self.date_designator('M')
    }

    /// Returns true if weeks may appear in the constrained duration.
    ///
    /// `weeks_allowed` (`org.openehr.am.aom2.c_duration.adoc` §Functions). The
    /// same page notes that mixing `W` with the other designators is an openEHR
    /// deviation from ISO 8601.
    #[must_use]
    pub fn weeks_allowed(&self) -> bool {
        self.date_designator('W')
    }

    /// Returns true if days may appear in the constrained duration.
    ///
    /// `days_allowed` (`org.openehr.am.aom2.c_duration.adoc` §Functions).
    #[must_use]
    pub fn days_allowed(&self) -> bool {
        self.date_designator('D')
    }

    /// Returns true if hours may appear in the constrained duration.
    ///
    /// `hours_allowed` (`org.openehr.am.aom2.c_duration.adoc` §Functions) — the
    /// `H` designator after the `T` separator.
    #[must_use]
    pub fn hours_allowed(&self) -> bool {
        self.time_designator('H')
    }

    /// Returns true if minutes may appear in the constrained duration.
    ///
    /// `minutes_allowed` (`org.openehr.am.aom2.c_duration.adoc` §Functions) —
    /// the `M` designator AFTER the `T` separator.
    #[must_use]
    pub fn minutes_allowed(&self) -> bool {
        self.time_designator('M')
    }

    /// Returns true if seconds may appear in the constrained duration.
    ///
    /// `seconds_allowed` (`org.openehr.am.aom2.c_duration.adoc` §Functions).
    #[must_use]
    pub fn seconds_allowed(&self) -> bool {
        self.time_designator('S')
    }

    /// Returns true if `a_pattern` is a valid duration constraint pattern.
    ///
    /// `valid_pattern_constraint` (`org.openehr.am.aom2.c_duration.adoc`
    /// §Functions): "Return `valid_iso8601_duration_constraint_pattern
    /// (a_pattern)`".
    #[must_use]
    #[expect(
        clippy::unused_self,
        reason = "the AM class page declares this as an instance function of the constrainer type, so the receiver is part of the spec signature even though the answer depends only on the pattern"
    )]
    pub fn valid_pattern_constraint(&self, a_pattern: &str) -> bool {
        CTemporalDefinitions::default().valid_iso8601_duration_constraint_pattern(a_pattern)
    }

    /// Returns true if `a_pattern` may replace `an_other_pattern` in a
    /// specialised constraint.
    ///
    /// `valid_pattern_constraint_replacement`
    /// (`org.openehr.am.aom2.c_duration.adoc` §Functions): "Return
    /// `valid_duration_constraint_replacement (a_pattern, an_other_pattern)`".
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
            .valid_duration_constraint_replacement(a_pattern, an_other_pattern)
    }

    /// Whether `designator` appears in the pattern's date half (before `T`).
    fn date_designator(&self, designator: char) -> bool {
        match self.pattern_halves() {
            Some((date, _)) => date.contains(designator),
            None => true,
        }
    }

    /// Whether `designator` appears in the pattern's time half (after `T`).
    fn time_designator(&self, designator: char) -> bool {
        match self.pattern_halves() {
            Some((_, time)) => time.contains(designator),
            None => true,
        }
    }

    /// The pattern's date and time halves, uppercased, with the leading `P` and
    /// the `T` separator removed.
    fn pattern_halves(&self) -> Option<(String, String)> {
        let pattern = self.pattern_constraint.as_ref()?;
        let body: String = pattern.chars().skip(1).collect::<String>().to_uppercase();
        Some(match body.split_once('T') {
            Some((date, time)) => (date.to_owned(), time.to_owned()),
            None => (body, String::new()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn duration(pattern: Option<&str>) -> CDuration {
        CDuration {
            parent: None,
            soc_parent: None,
            rm_type_name: "DV_DURATION".to_owned(),
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
    fn the_m_designator_means_months_before_t_and_minutes_after_it() {
        let months = duration(Some("PYM"));
        assert!(months.months_allowed());
        assert!(!months.minutes_allowed());
        let minutes = duration(Some("PTM"));
        assert!(!minutes.months_allowed());
        assert!(minutes.minutes_allowed());
    }

    #[test]
    fn the_weeks_and_days_pattern_allows_only_those_slots() {
        let d = duration(Some("Pwd"));
        assert!(d.weeks_allowed());
        assert!(d.days_allowed());
        assert!(!d.years_allowed());
        assert!(!d.hours_allowed());
        assert!(!d.seconds_allowed());
    }

    #[test]
    fn a_range_only_constraint_leaves_every_slot_allowed() {
        let d = duration(None);
        assert!(d.years_allowed());
        assert!(d.months_allowed());
        assert!(d.weeks_allowed());
        assert!(d.days_allowed());
        assert!(d.hours_allowed());
        assert!(d.minutes_allowed());
        assert!(d.seconds_allowed());
    }

    #[test]
    fn the_full_time_pattern_allows_hours_minutes_and_seconds() {
        let d = duration(Some("PDTHMS"));
        assert!(d.days_allowed());
        assert!(d.hours_allowed());
        assert!(d.minutes_allowed());
        assert!(d.seconds_allowed());
        assert!(!d.years_allowed());
    }

    #[test]
    fn pattern_validity_and_replacement_follow_the_definitions_class() {
        let d = duration(None);
        assert!(d.valid_pattern_constraint("PYMD"));
        assert!(!d.valid_pattern_constraint("PDY"));
        assert!(d.valid_pattern_constraint_replacement("PYD", "PYMD"));
        assert!(!d.valid_pattern_constraint_replacement("PYD", "PY"));
    }
}
