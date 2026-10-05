// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `C_ORDERED` spec functions.
//!
//! Spec source (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.c_ordered.adoc` §Functions, with the
//! temporal descendants redefining `any_allowed` per
//! `org.openehr.am.aom2.c_temporal.adoc` §Functions.

use crate::v2_4::aom2::constraint_model::primitive::c_ordered::COrdered;
use crate::v2_4::aom2::constraint_model::primitive::c_temporal_impl::{
    temporal_value_conforms, temporal_value_congruent,
};
use openehr_base::v1_3::foundation_types::interval::interval::Interval;

impl COrdered {
    /// Returns true if any value of the constrained ordered type would be
    /// allowed.
    ///
    /// `any_allowed` (`org.openehr.am.aom2.c_ordered.adoc` §Functions),
    /// post-condition `Result = constraint.is_empty`. The temporal descendants
    /// redefine it to also require an empty `pattern_constraint`, so this
    /// dispatches to `C_TEMPORAL` for them.
    #[must_use]
    pub fn any_allowed(&self) -> bool {
        let (range_empty, pattern) = match self {
            Self::CDate(c) => (
                c.constraint.as_ref().is_none_or(Vec::is_empty),
                c.pattern_constraint.as_deref(),
            ),
            Self::CDateTime(c) => (
                c.constraint.as_ref().is_none_or(Vec::is_empty),
                c.pattern_constraint.as_deref(),
            ),
            Self::CDuration(c) => (
                c.constraint.as_ref().is_none_or(Vec::is_empty),
                c.pattern_constraint.as_deref(),
            ),
            Self::CTime(c) => (
                c.constraint.as_ref().is_none_or(Vec::is_empty),
                c.pattern_constraint.as_deref(),
            ),
            Self::CInteger(c) => (c.constraint.as_ref().is_none_or(Vec::is_empty), None),
            Self::CReal(c) => (c.constraint.as_ref().is_none_or(Vec::is_empty), None),
        };
        range_empty && pattern.is_none_or(str::is_empty)
    }

    /// Returns true if this node's value constraint is the same as, or narrower
    /// than, `other`'s.
    ///
    /// `c_value_conforms_to` (`master04.5` §Conformance semantics: C_ORDERED):
    /// `other.any_allowed or for_all c:constraint | there_exists
    /// oc:other.constraint | oc.contains (c)`. The temporal descendants redefine
    /// it (`master04.5` §Conformance semantics: C_TEMPORAL), so those pairs run
    /// the `C_TEMPORAL` body; a pair of different primitive types never
    /// conforms, since the Eiffel declares the parameter `like Current`.
    #[must_use]
    pub fn c_value_conforms_to(&self, other: &COrdered) -> bool {
        match (self, other) {
            (Self::CInteger(own), Self::CInteger(theirs)) => {
                theirs.constraint.as_deref().unwrap_or_default().is_empty()
                    || intervals_conform(
                        own.constraint.as_deref().unwrap_or_default(),
                        theirs.constraint.as_deref().unwrap_or_default(),
                    )
            }
            (Self::CReal(own), Self::CReal(theirs)) => {
                theirs.constraint.as_deref().unwrap_or_default().is_empty()
                    || intervals_conform(
                        own.constraint.as_deref().unwrap_or_default(),
                        theirs.constraint.as_deref().unwrap_or_default(),
                    )
            }
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
    /// `c_value_congruent_to` (`master04.5` §Conformance semantics: C_ORDERED):
    /// `constraint.count = other.constraint.count and for_all c:constraint |
    /// c.is_equal (other.constraint.i_th (constraint.index_of (c)))`, i.e. equal
    /// interval-by-interval in declaration order, with the temporal descendants
    /// redefining it to also require an equal `pattern_constraint`.
    #[must_use]
    pub fn c_value_congruent_to(&self, other: &COrdered) -> bool {
        match (self, other) {
            (Self::CInteger(own), Self::CInteger(theirs)) => {
                own.constraint.as_deref().unwrap_or_default()
                    == theirs.constraint.as_deref().unwrap_or_default()
            }
            (Self::CReal(own), Self::CReal(theirs)) => {
                own.constraint.as_deref().unwrap_or_default()
                    == theirs.constraint.as_deref().unwrap_or_default()
            }
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

/// The `C_ORDERED` interval-conformance test: every child interval is contained
/// by some parent interval (`master04.5` §Conformance semantics: C_ORDERED).
pub(crate) fn intervals_conform<T: PartialOrd>(
    child: &[Interval<T>],
    other: &[Interval<T>],
) -> bool {
    child
        .iter()
        .all(|own| other.iter().any(|theirs| theirs.contains(own)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2_4::aom2::constraint_model::primitive::c_date::CDate;
    use crate::v2_4::aom2::constraint_model::primitive::c_integer::CInteger;
    use openehr_base::v1_3::foundation_types::interval::interval::Interval;
    use openehr_base::v1_3::foundation_types::interval::point_interval::PointInterval;

    fn integer(constraint: Option<Vec<Interval<i32>>>) -> COrdered {
        COrdered::CInteger(CInteger {
            parent: None,
            soc_parent: None,
            rm_type_name: "Integer".to_owned(),
            occurrences: None,
            node_id: "at9999".to_owned(),
            alternative_ids: None,
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value: None,
            is_enumerated_type_constraint: None,
            constraint,
        })
    }

    fn date(pattern: Option<&str>) -> COrdered {
        COrdered::CDate(CDate {
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
        })
    }

    #[test]
    fn an_unstated_range_allows_any_value() {
        assert!(integer(None).any_allowed());
        assert!(integer(Some(Vec::new())).any_allowed());
    }

    #[test]
    fn one_interval_is_already_a_constraint() {
        let point = Interval::PointInterval(PointInterval {
            lower: Some(1),
            upper: Some(1),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        });
        assert!(!integer(Some(vec![point])).any_allowed());
    }

    #[test]
    fn a_temporal_descendant_also_answers_for_its_pattern() {
        assert!(date(None).any_allowed());
        assert!(!date(Some("YYYY-??-XX")).any_allowed());
    }
}
