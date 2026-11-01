// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `C_TEMPORAL` spec functions.
//!
//! Spec sources (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.c_temporal.adoc` §Functions and
//! `AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §Conformance semantics: C_TEMPORAL.

use crate::v2_4::aom2::constraint_model::primitive::c_date::CDate;
use crate::v2_4::aom2::constraint_model::primitive::c_date_time::CDateTime;
use crate::v2_4::aom2::constraint_model::primitive::c_duration::CDuration;
use crate::v2_4::aom2::constraint_model::primitive::c_ordered_impl::intervals_conform;
use crate::v2_4::aom2::constraint_model::primitive::c_temporal::CTemporal;
use crate::v2_4::aom2::constraint_model::primitive::c_time::CTime;
use openehr_base::v1_3::foundation_types::interval::interval::Interval;
use openehr_base::v1_3::foundation_types::time::iso8601_date::Iso8601Date;
use openehr_base::v1_3::foundation_types::time::iso8601_date_time::Iso8601DateTime;
use openehr_base::v1_3::foundation_types::time::iso8601_duration::Iso8601Duration;
use openehr_base::v1_3::foundation_types::time::iso8601_time::Iso8601Time;

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
        match self {
            Self::CDate(c) => temporal_any_allowed(c),
            Self::CDateTime(c) => temporal_any_allowed(c),
            Self::CDuration(c) => temporal_any_allowed(c),
            Self::CTime(c) => temporal_any_allowed(c),
        }
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
            (Self::CDate(own), Self::CDate(theirs)) => temporal_value_conforms(own, theirs),
            (Self::CDateTime(own), Self::CDateTime(theirs)) => temporal_value_conforms(own, theirs),
            (Self::CDuration(own), Self::CDuration(theirs)) => temporal_value_conforms(own, theirs),
            (Self::CTime(own), Self::CTime(theirs)) => temporal_value_conforms(own, theirs),
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
            (Self::CDate(own), Self::CDate(theirs)) => temporal_value_congruent(own, theirs),
            (Self::CDateTime(own), Self::CDateTime(theirs)) => {
                temporal_value_congruent(own, theirs)
            }
            (Self::CDuration(own), Self::CDuration(theirs)) => {
                temporal_value_congruent(own, theirs)
            }
            (Self::CTime(own), Self::CTime(theirs)) => temporal_value_congruent(own, theirs),
            _ => false,
        }
    }
}

/// The `C_TEMPORAL` features one concrete descendant (`C_DATE`, `C_TIME`,
/// `C_DATE_TIME`, `C_DURATION`) carries, which the conformance bodies below
/// read (`master04.5` §Conformance semantics: C_TEMPORAL). The generated
/// descendants hold them as fields of their own and share no Rust type.
pub(crate) trait TemporalLeaf {
    /// The ISO 8601 value type the interval constraint ranges over.
    type Value: PartialOrd;
    /// The interval constraint, empty when absent.
    fn interval_constraint(&self) -> &[Interval<Self::Value>];
    /// The pattern constraint, if any.
    fn pattern(&self) -> Option<&str>;
    /// The descendant's own `valid_pattern_constraint_replacement`, whose
    /// table is per temporal type.
    fn replaces(&self, a_pattern: &str, an_other_pattern: &str) -> bool;
}

/// Implements [`TemporalLeaf`] for each descendant over its value type.
macro_rules! temporal_leaf {
    ($($leaf:ty => $value:ty),* $(,)?) => {$(
        impl TemporalLeaf for $leaf {
            type Value = $value;
            fn interval_constraint(&self) -> &[Interval<$value>] {
                self.constraint.as_deref().unwrap_or_default()
            }
            fn pattern(&self) -> Option<&str> {
                self.pattern_constraint.as_deref()
            }
            fn replaces(&self, a_pattern: &str, an_other_pattern: &str) -> bool {
                self.valid_pattern_constraint_replacement(a_pattern, an_other_pattern)
            }
        }
    )*};
}

temporal_leaf!(
    CDate => Iso8601Date,
    CDateTime => Iso8601DateTime,
    CDuration => Iso8601Duration,
    CTime => Iso8601Time,
);

/// The `C_TEMPORAL` `any_allowed` of one descendant: no interval constraint
/// and no pattern (`org.openehr.am.aom2.c_temporal.adoc` §Functions).
pub(crate) fn temporal_any_allowed<L: TemporalLeaf>(leaf: &L) -> bool {
    leaf.interval_constraint().is_empty() && leaf.pattern().is_none_or(str::is_empty)
}

/// The `C_TEMPORAL` value-conformance body of one descendant
/// (`master04.5` §Conformance semantics: C_TEMPORAL).
pub(crate) fn temporal_value_conforms<L: TemporalLeaf>(own: &L, other: &L) -> bool {
    if !temporal_any_allowed(other)
        && !intervals_conform(own.interval_constraint(), other.interval_constraint())
    {
        return false;
    }
    let Some(parent_pattern) = other.pattern().filter(|p| !p.is_empty()) else {
        return true;
    };
    own.pattern()
        .filter(|p| !p.is_empty())
        .is_some_and(|child| own.replaces(child, parent_pattern))
}

/// The `C_TEMPORAL` value-congruence body of one descendant
/// (`master04.5` §Conformance semantics: C_TEMPORAL).
pub(crate) fn temporal_value_congruent<L: TemporalLeaf>(own: &L, other: &L) -> bool {
    own.interval_constraint() == other.interval_constraint()
        && own.pattern().unwrap_or_default() == other.pattern().unwrap_or_default()
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
