// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `C_TEMPORAL` spec functions.
//!
//! Spec sources (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.c_temporal.adoc` §Functions and
//! `AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §Conformance semantics: C_TEMPORAL.

use crate::v2_4::aom2::constraint_model::primitive::c_ordered_impl::intervals_conform;
use crate::v2_4::aom2::constraint_model::primitive::c_temporal::CTemporal;
use openehr_base::v1_3::foundation_types::interval::interval::Interval;

impl CTemporal {
    /// Returns true if any value of the constrained temporal type would be
    /// allowed.
    ///
    /// `any_allowed` (`org.openehr.am.aom2.c_temporal.adoc` §Functions),
    /// post-condition `Result = precursor and pattern_constraint.is_empty` —
    /// the precursor being `C_ORDERED.any_allowed`, `Result =
    /// constraint.is_empty` (`org.openehr.am.aom2.c_ordered.adoc` §Functions).
    #[must_use]
    pub fn any_allowed(&self) -> bool {
        let (range_empty, pattern) = match self {
            Self::CDate(c) => (
                c.constraint.as_ref().is_none_or(Vec::is_empty),
                &c.pattern_constraint,
            ),
            Self::CDateTime(c) => (
                c.constraint.as_ref().is_none_or(Vec::is_empty),
                &c.pattern_constraint,
            ),
            Self::CDuration(c) => (
                c.constraint.as_ref().is_none_or(Vec::is_empty),
                &c.pattern_constraint,
            ),
            Self::CTime(c) => (
                c.constraint.as_ref().is_none_or(Vec::is_empty),
                &c.pattern_constraint,
            ),
        };
        range_empty && pattern.as_ref().is_none_or(String::is_empty)
    }

    /// Returns true if `a_pattern` is a valid constraint pattern for this
    /// temporal type.
    ///
    /// `valid_pattern_constraint` (`org.openehr.am.aom2.c_temporal.adoc`
    /// §Functions), "Define in concrete descendants" — so this dispatches to
    /// the descendant that owns the pattern table.
    #[must_use]
    pub fn valid_pattern_constraint(&self, a_pattern: &str) -> bool {
        match self {
            Self::CDate(c) => c.valid_pattern_constraint(a_pattern),
            Self::CDateTime(c) => c.valid_pattern_constraint(a_pattern),
            Self::CDuration(c) => c.valid_pattern_constraint(a_pattern),
            Self::CTime(c) => c.valid_pattern_constraint(a_pattern),
        }
    }

    /// Returns true if `a_pattern` may replace `an_other_pattern` in a
    /// specialised constraint.
    ///
    /// `valid_pattern_constraint_replacement`
    /// (`org.openehr.am.aom2.c_temporal.adoc` §Functions), "Define in concrete
    /// subtypes" — so this dispatches to the descendant that owns the
    /// replacement table.
    #[must_use]
    pub fn valid_pattern_constraint_replacement(
        &self,
        a_pattern: &str,
        an_other_pattern: &str,
    ) -> bool {
        match self {
            Self::CDate(c) => c.valid_pattern_constraint_replacement(a_pattern, an_other_pattern),
            Self::CDateTime(c) => {
                c.valid_pattern_constraint_replacement(a_pattern, an_other_pattern)
            }
            Self::CDuration(c) => {
                c.valid_pattern_constraint_replacement(a_pattern, an_other_pattern)
            }
            Self::CTime(c) => c.valid_pattern_constraint_replacement(a_pattern, an_other_pattern),
        }
    }

    /// Returns true if this node's value constraint is the same as, or narrower
    /// than, `other`'s.
    ///
    /// `c_value_conforms_to` (`master04.5` §Conformance semantics: C_TEMPORAL):
    /// the `C_ORDERED` precursor over the interval constraint, and then either
    /// an empty `other.pattern_constraint` or a
    /// `valid_pattern_constraint_replacement` of it.
    ///
    /// NOTE: the class page's gloss spells the same rule as three `or else`
    /// disjuncts, which no pattern could ever fail; the conjunctive Eiffel body
    /// in `master04.5` is the reading implemented here.
    #[must_use]
    pub fn c_value_conforms_to(&self, other: &CTemporal) -> bool {
        match (self, other) {
            (Self::CDate(own), Self::CDate(theirs)) => temporal_value_conforms(
                own.constraint.as_deref().unwrap_or_default(),
                own.pattern_constraint.as_deref(),
                theirs.constraint.as_deref().unwrap_or_default(),
                theirs.pattern_constraint.as_deref(),
                &|child, parent| own.valid_pattern_constraint_replacement(child, parent),
            ),
            (Self::CDateTime(own), Self::CDateTime(theirs)) => temporal_value_conforms(
                own.constraint.as_deref().unwrap_or_default(),
                own.pattern_constraint.as_deref(),
                theirs.constraint.as_deref().unwrap_or_default(),
                theirs.pattern_constraint.as_deref(),
                &|child, parent| own.valid_pattern_constraint_replacement(child, parent),
            ),
            (Self::CDuration(own), Self::CDuration(theirs)) => temporal_value_conforms(
                own.constraint.as_deref().unwrap_or_default(),
                own.pattern_constraint.as_deref(),
                theirs.constraint.as_deref().unwrap_or_default(),
                theirs.pattern_constraint.as_deref(),
                &|child, parent| own.valid_pattern_constraint_replacement(child, parent),
            ),
            (Self::CTime(own), Self::CTime(theirs)) => temporal_value_conforms(
                own.constraint.as_deref().unwrap_or_default(),
                own.pattern_constraint.as_deref(),
                theirs.constraint.as_deref().unwrap_or_default(),
                theirs.pattern_constraint.as_deref(),
                &|child, parent| own.valid_pattern_constraint_replacement(child, parent),
            ),
            _ => false,
        }
    }

    /// Returns true if this node's value constraint is the same as `other`'s.
    ///
    /// `c_value_congruent_to` (`master04.5` §Conformance semantics: C_TEMPORAL):
    /// `precursor (other) and pattern_constraint ~ other.pattern_constraint`.
    #[must_use]
    pub fn c_value_congruent_to(&self, other: &CTemporal) -> bool {
        match (self, other) {
            (Self::CDate(own), Self::CDate(theirs)) => temporal_value_congruent(
                own.constraint.as_deref().unwrap_or_default(),
                own.pattern_constraint.as_deref(),
                theirs.constraint.as_deref().unwrap_or_default(),
                theirs.pattern_constraint.as_deref(),
            ),
            (Self::CDateTime(own), Self::CDateTime(theirs)) => temporal_value_congruent(
                own.constraint.as_deref().unwrap_or_default(),
                own.pattern_constraint.as_deref(),
                theirs.constraint.as_deref().unwrap_or_default(),
                theirs.pattern_constraint.as_deref(),
            ),
            (Self::CDuration(own), Self::CDuration(theirs)) => temporal_value_congruent(
                own.constraint.as_deref().unwrap_or_default(),
                own.pattern_constraint.as_deref(),
                theirs.constraint.as_deref().unwrap_or_default(),
                theirs.pattern_constraint.as_deref(),
            ),
            (Self::CTime(own), Self::CTime(theirs)) => temporal_value_congruent(
                own.constraint.as_deref().unwrap_or_default(),
                own.pattern_constraint.as_deref(),
                theirs.constraint.as_deref().unwrap_or_default(),
                theirs.pattern_constraint.as_deref(),
            ),
            _ => false,
        }
    }
}

/// The `C_TEMPORAL` value-conformance body over one leaf type's constraint and
/// pattern (`master04.5` §Conformance semantics: C_TEMPORAL).
///
/// `valid_replacement` is the leaf's own
/// `valid_pattern_constraint_replacement`, whose table is per temporal type.
pub(crate) fn temporal_value_conforms<T: PartialOrd>(
    child_constraint: &[Interval<T>],
    child_pattern: Option<&str>,
    other_constraint: &[Interval<T>],
    other_pattern: Option<&str>,
    valid_replacement: &dyn Fn(&str, &str) -> bool,
) -> bool {
    let other_any_allowed = other_constraint.is_empty() && other_pattern.is_none_or(str::is_empty);
    if !other_any_allowed && !intervals_conform(child_constraint, other_constraint) {
        return false;
    }
    let Some(parent_pattern) = other_pattern.filter(|p| !p.is_empty()) else {
        return true;
    };
    child_pattern
        .filter(|p| !p.is_empty())
        .is_some_and(|own| valid_replacement(own, parent_pattern))
}

/// The `C_TEMPORAL` value-congruence body over one leaf type's constraint and
/// pattern (`master04.5` §Conformance semantics: C_TEMPORAL).
pub(crate) fn temporal_value_congruent<T: PartialEq>(
    child_constraint: &[Interval<T>],
    child_pattern: Option<&str>,
    other_constraint: &[Interval<T>],
    other_pattern: Option<&str>,
) -> bool {
    child_constraint == other_constraint
        && child_pattern.unwrap_or_default() == other_pattern.unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2_4::aom2::constraint_model::primitive::c_date::CDate;
    use openehr_base::v1_3::foundation_types::interval::interval::Interval;
    use openehr_base::v1_3::foundation_types::interval::point_interval::PointInterval;
    use openehr_base::v1_3::foundation_types::time::iso8601_date::Iso8601Date;

    fn date(constraint: Option<Vec<Interval<Iso8601Date>>>, pattern: Option<&str>) -> CTemporal {
        CTemporal::CDate(CDate {
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
            constraint,
            pattern_constraint: pattern.map(str::to_owned),
        })
    }

    fn a_range() -> Vec<Interval<Iso8601Date>> {
        vec![Interval::PointInterval(PointInterval {
            lower: Some(Iso8601Date {
                value: "2004-05-20".to_owned(),
            }),
            upper: Some(Iso8601Date {
                value: "2004-05-20".to_owned(),
            }),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        })]
    }

    #[test]
    fn neither_a_range_nor_a_pattern_allows_anything() {
        assert!(date(None, None).any_allowed());
        assert!(date(Some(Vec::new()), None).any_allowed());
    }

    #[test]
    fn either_a_range_or_a_pattern_is_already_a_constraint() {
        assert!(!date(Some(a_range()), None).any_allowed());
        assert!(!date(None, Some("YYYY-??-XX")).any_allowed());
        assert!(!date(Some(a_range()), Some("YYYY-??-XX")).any_allowed());
    }
}
