// @generated-from-template templates/openehr-rm/data_types/quantity/dv_quantified_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM spec functions for `DV_QUANTIFIED`.
//!
//! `DV_QUANTIFIED` is abstract, so the generated `DvQuantified` is the closed
//! subtype enum over the seven quantified data types, and the five functions
//! the class declares are realized here as the dispatch across it. The
//! per-type magnitudes, comparability rules and accuracy readings are NOT
//! re-derived: they live once in the `DV_ORDERED` / `DV_AMOUNT` /
//! `DV_ABSOLUTE_QUANTITY` modules and are reached from here, so a quantified
//! value cannot order or compare differently from the ordered value it is.
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.dv_quantified.adoc`
//! §Functions + §Invariants, with the accuracy readings from
//! `docs/specs/openehr/RM/docs/data_types/master06-quantity_package.adoc`
//! §Accuracy and Uncertainty.

use crate::v1_1::data_types::quantity::dv_amount::DvAmount;
use crate::v1_1::data_types::quantity::dv_ordered::DvOrdered;
use crate::v1_1::data_types::quantity::dv_quantified::DvQuantified;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;

impl DvQuantified {
    /// Returns `true` when `s` is one of the values `magnitude_status` may
    /// take.
    ///
    /// Spec: `org.openehr.rm.data_types.dv_quantified.adoc` §Functions
    /// `valid_magnitude_status`, whose post-condition is the closed set
    /// `Result = s in {"=", "<", ">", "<=", ">=", "~"}` — the same set
    /// §Invariants `Magnitude_status_valid` tests `magnitude_status` against.
    ///
    /// The class declares the function with NO parameter while its own
    /// post-condition reads a free `s` and the invariant calls it with one
    /// argument, so the string it tests is taken as that parameter.
    #[must_use]
    pub fn valid_magnitude_status(s: &str) -> bool {
        matches!(s, "=" | "<" | ">" | "<=" | ">=" | "~")
    }

    /// Returns the precise magnitude of this value, or `None` when the value
    /// carries none that can be read.
    ///
    /// Spec: `org.openehr.rm.data_types.dv_quantified.adoc` §Functions
    /// `magnitude`, which the class declares abstract and each subtype
    /// effects. The dispatch is the `DV_ORDERED` one
    /// (`org.openehr.rm.data_types.dv_ordered.adoc`), so the magnitude that
    /// orders a value is the magnitude it reports: a count's stored integer, a
    /// quantity's `magnitude`, a proportion's ratio, a date's day count, a
    /// time's or date-time's seconds, a duration's nominal seconds. `None` is
    /// a temporal `value` that is not a readable ISO-8601 string, which has no
    /// magnitude to state.
    #[must_use]
    pub fn magnitude(&self) -> Option<f64> {
        self.as_ordered().magnitude()
    }

    /// Returns `true` when this value's accuracy was not recorded.
    ///
    /// Spec: `org.openehr.rm.data_types.dv_quantified.adoc` §Functions
    /// `accuracy_unknown` — "True if accuracy is not known, e.g. due to not
    /// being recorded or discernable" — effected differently by the two
    /// branches of the hierarchy, per
    /// `master06-quantity_package.adoc` §Accuracy and Uncertainty: "in
    /// `DV_AMOUNT`, a value of -1 for the accuracy attribute is used for this
    /// purpose", while "in the `DV_ABSOLUTE_QUANTITY` class, `accuracy_unknown`
    /// is represented by a Void (i.e. null) value for the accuracy attribute".
    /// An absent accuracy is unknown under both readings.
    #[must_use]
    pub fn accuracy_unknown(&self) -> bool {
        match self {
            Self::DvCount(value) => value.accuracy_unknown(),
            Self::DvDuration(value) => value.accuracy_unknown(),
            Self::DvProportion(value) => value.accuracy_unknown(),
            Self::DvQuantity(value) => value.accuracy_unknown(),
            Self::DvDate(value) => value.accuracy.is_none(),
            Self::DvDateTime(value) => value.accuracy.is_none(),
            Self::DvTime(value) => value.accuracy.is_none(),
        }
    }

    /// Returns `true` when this value is considered equal to `other`.
    ///
    /// Spec: `org.openehr.rm.data_types.dv_quantified.adoc` §Functions
    /// `is_equal`, which the class declares abstract; BASE
    /// `docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.foundation_types.any.adoc`
    /// §Functions defines what it means — "True if `this` and `other` are
    /// attached to objects considered to be equal in VALUE".
    ///
    /// Two values of different concrete types are never equal: `DV_ORDERED`
    /// compares only like with like. Each like pair is answered by that
    /// subtype's own equality — the `DV_AMOUNT` one for the four differential
    /// types, `DV_DATE`'s for a date — so this function cannot disagree with
    /// the equality the subtype already defines. The two remaining temporal
    /// types have no effecting definition of their own and take `DV_DATE`'s
    /// rule: equality of the instant, with an unreadable value equal only to
    /// the identical string.
    #[must_use]
    pub fn is_equal(&self, other: &Self) -> bool {
        if let (Some(left), Some(right)) = (self.as_amount(), other.as_amount()) {
            return left.is_equal(&right);
        }
        match (self, other) {
            (Self::DvDate(left), Self::DvDate(right)) => left.is_equal(right),
            (Self::DvDateTime(left), Self::DvDateTime(right)) => same_instant(
                left.magnitude(),
                right.magnitude(),
                &left.value,
                &right.value,
            ),
            (Self::DvTime(left), Self::DvTime(right)) => same_instant(
                left.magnitude(),
                right.magnitude(),
                &left.value,
                &right.value,
            ),
            _ => false,
        }
    }

    /// Returns `true` when this value is less than `other`, or `None` when the
    /// two are not strictly comparable or a magnitude is unavailable.
    ///
    /// Spec: `org.openehr.rm.data_types.dv_quantified.adoc` §Functions
    /// `less_than`, whose `Pre_comparable` is
    /// `is_strictly_comparable_to (other)` and whose `Post_result` is
    /// `Result = magnitude < other.magnitude`. Both are the `DV_ORDERED`
    /// ordering this forwards to, so an unmet precondition is `None` rather
    /// than a fabricated `false`.
    #[must_use]
    pub fn less_than(&self, other: &Self) -> Option<bool> {
        self.as_ordered().less_than(&other.as_ordered())
    }

    /// This quantified value as the `DV_ORDERED` it is.
    fn as_ordered(&self) -> DvOrdered {
        match self {
            Self::DvCount(value) => DvOrdered::DvCount(value.clone()),
            Self::DvDate(value) => DvOrdered::DvDate(value.clone()),
            Self::DvDateTime(value) => DvOrdered::DvDateTime(value.clone()),
            Self::DvDuration(value) => DvOrdered::DvDuration(value.clone()),
            Self::DvProportion(value) => DvOrdered::DvProportion(value.clone()),
            Self::DvQuantity(value) => DvOrdered::DvQuantity(value.clone()),
            Self::DvTime(value) => DvOrdered::DvTime(value.clone()),
        }
    }

    /// This value as the `DV_AMOUNT` it is, for the four differential
    /// subtypes; `None` for the three `DV_ABSOLUTE_QUANTITY` ones, which are
    /// points on a scale rather than amounts of it.
    fn as_amount(&self) -> Option<DvAmount> {
        match self {
            Self::DvCount(value) => Some(DvAmount::DvCount(value.clone())),
            Self::DvDuration(value) => Some(DvAmount::DvDuration(value.clone())),
            Self::DvProportion(value) => Some(DvAmount::DvProportion(value.clone())),
            Self::DvQuantity(value) => Some(DvAmount::DvQuantity(value.clone())),
            Self::DvDate(_) | Self::DvDateTime(_) | Self::DvTime(_) => None,
        }
    }
}

/// Whether two temporal values name the same instant.
///
/// The magnitudes are compared exactly, as decimals: the question is which
/// instants the two values denote, not how the seconds happen to be carried.
/// A value with no readable magnitude is not an instant this can compare, so
/// only an identical written form still counts as the same one — the rule
/// `DV_DATE.is_equal` states
/// (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.dv_date.adoc`
/// §Functions).
fn same_instant(
    left: Option<f64>,
    right: Option<f64>,
    left_value: &str,
    right_value: &str,
) -> bool {
    let (Some(left), Some(right)) = (left, right) else {
        return left_value == right_value;
    };
    let (Some(left), Some(right)) = (Decimal::from_f64(left), Decimal::from_f64(right)) else {
        return left_value == right_value;
    };
    left == right
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_1::data_types::quantity::date_time::dv_date::DvDate;
    use crate::v1_1::data_types::quantity::date_time::dv_date_time::DvDateTime;
    use crate::v1_1::data_types::quantity::date_time::dv_duration::DvDuration;
    use crate::v1_1::data_types::quantity::date_time::dv_time::DvTime;
    use crate::v1_1::data_types::quantity::dv_count::DvCount;
    use crate::v1_1::data_types::quantity::dv_quantity::DvQuantity;

    fn count(magnitude: i64, accuracy: Option<f64>) -> DvQuantified {
        DvQuantified::DvCount(DvCount {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: None,
            magnitude_status: None,
            accuracy,
            accuracy_is_percent: None,
            magnitude,
        })
    }

    fn quantity(magnitude: f64, units: &str) -> DvQuantified {
        DvQuantified::DvQuantity(DvQuantity {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: None,
            magnitude_status: None,
            accuracy: None,
            accuracy_is_percent: None,
            magnitude,
            units: units.to_owned(),
            precision: None,
            units_system: None,
            units_display_name: None,
        })
    }

    fn date(value: &str) -> DvQuantified {
        DvQuantified::DvDate(DvDate {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: None,
            magnitude_status: None,
            accuracy: None,
            value: value.to_owned(),
        })
    }

    fn date_time(value: &str, accuracy: Option<DvDuration>) -> DvQuantified {
        DvQuantified::DvDateTime(DvDateTime {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: None,
            magnitude_status: None,
            accuracy,
            value: value.to_owned(),
        })
    }

    fn time(value: &str) -> DvQuantified {
        DvQuantified::DvTime(DvTime {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: None,
            magnitude_status: None,
            accuracy: None,
            value: value.to_owned(),
        })
    }

    fn duration(value: &str) -> DvDuration {
        DvDuration {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: None,
            magnitude_status: None,
            accuracy: None,
            accuracy_is_percent: None,
            value: value.to_owned(),
        }
    }

    /// `Result = s in {"=", "<", ">", "<=", ">=", "~"}` — the whole set, and
    /// nothing beside it (including the empty string and a two-character
    /// near-miss).
    #[test]
    fn the_magnitude_status_set_is_closed() {
        for status in ["=", "<", ">", "<=", ">=", "~"] {
            assert!(DvQuantified::valid_magnitude_status(status), "{status:?}");
        }
        for status in ["", " ", "==", "=<", "≈", "!="] {
            assert!(!DvQuantified::valid_magnitude_status(status), "{status:?}");
        }
    }

    /// A magnitude as an exact decimal, so an assertion compares the quantity
    /// rather than the binary approximation carrying it.
    fn exact(value: Option<f64>) -> Option<Decimal> {
        value.and_then(Decimal::from_f64)
    }

    /// The magnitude is the ordering magnitude of whichever subtype this is —
    /// a count's integer and a date's day count alike.
    #[test]
    fn the_magnitude_is_the_subtypes_own() {
        assert_eq!(exact(count(7, None).magnitude()), Decimal::from_f64(7.0));
        assert_eq!(
            exact(quantity(2.5, "mm[Hg]").magnitude()),
            Decimal::from_f64(2.5)
        );
        // `DV_DATE.magnitude` counts days from the calendar origin itself.
        assert_eq!(
            exact(date("0001-01-01").magnitude()),
            Decimal::from_f64(0.0)
        );
        assert_eq!(exact(time("00:01:00").magnitude()), Decimal::from_f64(60.0));
        // A value that is not a readable ISO-8601 date states no magnitude.
        assert_eq!(exact(date("the fifth of never").magnitude()), None);
    }

    /// The two accuracy readings: `-1` (or absent) for a `DV_AMOUNT`
    /// descendant, absent for a `DV_ABSOLUTE_QUANTITY` one — where `0.0` is a
    /// recorded accuracy of zero, not an unknown one.
    #[test]
    fn accuracy_unknown_follows_each_branchs_own_sentinel() {
        assert!(count(1, None).accuracy_unknown());
        assert!(count(1, Some(-1.0)).accuracy_unknown());
        assert!(!count(1, Some(0.0)).accuracy_unknown());

        assert!(date_time("2026-01-01T00:00:00Z", None).accuracy_unknown());
        assert!(!date_time("2026-01-01T00:00:00Z", Some(duration("PT1S"))).accuracy_unknown());
    }

    /// Equality is of the value, across written forms, and never across
    /// concrete types.
    #[test]
    fn equality_is_of_the_value_and_only_within_one_type() {
        assert!(date("2024-01-01").is_equal(&date("20240101")));
        assert!(time("10:00:00").is_equal(&time("100000")));
        assert!(
            date_time("2026-01-01T00:00:00Z", None)
                .is_equal(&date_time("2026-01-01T01:00:00+01:00", None))
        );
        assert!(!count(7, None).is_equal(&quantity(7.0, "mm[Hg]")));
        assert!(!date("2024-01-01").is_equal(&date("2024-01-02")));
    }

    /// A value with no readable magnitude is equal only to the identical
    /// written form, so it is never simultaneously not-less, not-greater and
    /// not-equal to itself.
    #[test]
    fn an_unreadable_temporal_value_equals_only_its_own_form() {
        assert!(time("not a time").is_equal(&time("not a time")));
        assert!(!time("not a time").is_equal(&time("also not a time")));
    }

    /// `Pre_comparable: is_strictly_comparable_to (other)` — two quantities in
    /// different units are not comparable, so `less_than` states nothing
    /// rather than answering `false`.
    #[test]
    fn less_than_states_nothing_when_the_precondition_fails() {
        assert_eq!(
            quantity(1.0, "mm[Hg]").less_than(&quantity(2.0, "mm[Hg]")),
            Some(true)
        );
        assert_eq!(
            quantity(2.0, "mm[Hg]").less_than(&quantity(1.0, "mm[Hg]")),
            Some(false)
        );
        assert_eq!(
            quantity(1.0, "mm[Hg]").less_than(&quantity(2.0, "kg")),
            None
        );
        assert_eq!(count(1, None).less_than(&quantity(2.0, "kg")), None);
    }
}
