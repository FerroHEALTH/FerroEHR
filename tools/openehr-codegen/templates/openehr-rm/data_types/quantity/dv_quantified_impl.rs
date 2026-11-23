// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The `DV_QUANTIFIED` functions
//! (`org.openehr.rm.data_types.dv_quantified.adoc`), dispatched to the
//! `DV_ORDERED`, `DV_AMOUNT` and `DV_ABSOLUTE_QUANTITY` definitions.

use crate::v1_2::data_types::quantity::dv_amount::DvAmount;
use crate::v1_2::data_types::quantity::dv_ordered::DvOrdered;
use crate::v1_2::data_types::quantity::dv_quantified::DvQuantified;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;

impl DvQuantified {
    /// Whether `s` is in `{"=", "<", ">", "<=", ">=", "~"}`
    /// (`valid_magnitude_status`, whose free `s` is read as its parameter).
    #[must_use]
    pub fn valid_magnitude_status(s: &str) -> bool {
        matches!(s, "=" | "<" | ">" | "<=" | ">=" | "~")
    }

    /// The magnitude of this value (`magnitude`), as `DV_ORDERED` orders it, or
    /// `None` for an unreadable temporal value.
    #[must_use]
    pub fn magnitude(&self) -> Option<f64> {
        self.as_ordered().magnitude()
    }

    /// Whether accuracy is unknown (`accuracy_unknown`): `-1` or absent for a
    /// `DV_AMOUNT`, absent for a `DV_ABSOLUTE_QUANTITY` (`master06-quantity_package.adoc`
    /// §Accuracy and Uncertainty).
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

    /// Whether this value equals `other` in value (`is_equal`; BASE `any.adoc`),
    /// by each subtype's own equality; different types are never equal.
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

    /// Whether this value is less than `other` (`less_than`), or `None` when not
    /// strictly comparable or a magnitude is unavailable.
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

    /// This value as a `DV_AMOUNT`, `None` for the absolute quantities.
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

/// Whether two temporal values name the same instant, exactly; an unreadable
/// value matches only its identical string (`dv_date.adoc` §Functions).
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
    use crate::v1_2::data_types::quantity::date_time::dv_date::DvDate;
    use crate::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
    use crate::v1_2::data_types::quantity::date_time::dv_duration::DvDuration;
    use crate::v1_2::data_types::quantity::date_time::dv_time::DvTime;
    use crate::v1_2::data_types::quantity::dv_count::DvCount;
    use crate::v1_2::data_types::quantity::dv_quantity::DvQuantity;

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
            accuracy: accuracy.map(Box::new),
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

    /// Exactly the six-member set.
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

    /// `-1` or absent for an amount, absent for an absolute quantity, where `0.0`
    /// is a recorded accuracy.
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

    /// An unreadable value equals only its identical string.
    #[test]
    fn an_unreadable_temporal_value_equals_only_its_own_form() {
        assert!(time("not a time").is_equal(&time("not a time")));
        assert!(!time("not a time").is_equal(&time("also not a time")));
    }

    /// Quantities in different units give no `less_than`.
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
