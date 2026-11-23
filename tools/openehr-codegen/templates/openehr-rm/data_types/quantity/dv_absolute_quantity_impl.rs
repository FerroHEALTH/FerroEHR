// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The `DV_ABSOLUTE_QUANTITY` accuracy rule, shared by `DV_DATE`, `DV_TIME` and
//! `DV_DATE_TIME`, whose accuracy is a `DV_DURATION`.

use crate::v1_2::data_types::quantity::date_time::dv_duration::DvDuration;
use crate::v1_2::data_types::quantity::dv_absolute_quantity::DvAbsoluteQuantity;
use crate::v1_2::data_types::quantity::dv_amount::DvAmount;
use crate::v1_2::data_types::quantity::dv_amount_impl::AmountAccuracy;
use crate::v1_2::data_types::quantity::dv_amount_impl::{CombinedAccuracy, combine};
use openehr_base::v1_3::foundation_types::time::iso8601_duration::Iso8601Duration;

/// The accuracy of a quantity displaced by a differential amount.
///
/// "the sum of the accuracies of the operands, if both present, or unknown"
/// (`dv_absolute_quantity.adoc` `add`); the amount's half-range counts as seconds.
#[must_use]
pub fn displaced_accuracy(
    accuracy: Option<&DvDuration>,
    amount: &DvDuration,
) -> Option<DvDuration> {
    let own = accuracy?;
    let amount_seconds = absolute_accuracy_seconds(amount)?;
    let sum = Iso8601Duration {
        value: own.value.clone(),
    }
    .add(&seconds_as_duration(amount_seconds))?;
    Some(DvDuration {
        value: sum.value,
        magnitude_status: None,
        accuracy: None,
        accuracy_is_percent: None,
        normal_range: None,
        normal_status: None,
        other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
    })
}

/// The accuracy of the difference of two quantities (`diff`), as `Real`
/// seconds.
#[must_use]
pub fn difference_accuracy(
    left: Option<&DvDuration>,
    right: Option<&DvDuration>,
) -> CombinedAccuracy {
    let (Some(left), Some(right)) = (left, right) else {
        return CombinedAccuracy::Unknown;
    };
    let (Some(left_seconds), Some(right_seconds)) = (left.magnitude(), right.magnitude()) else {
        return CombinedAccuracy::Unrepresentable;
    };
    // Through the `DV_AMOUNT` rule, for its finiteness and zero-form checks.
    combine(
        AmountAccuracy::measured(left_seconds, Some(left_seconds), Some(false)),
        AmountAccuracy::measured(right_seconds, Some(right_seconds), Some(false)),
    )
}

/// A duration's accuracy as an absolute half-range in seconds, resolving a
/// percentage against the duration's own magnitude.
fn absolute_accuracy_seconds(amount: &DvDuration) -> Option<f64> {
    let accuracy = amount.accuracy.filter(|value| *value >= 0.0)?;
    if amount.accuracy_is_percent == Some(true) {
        return Some(accuracy / 100.0 * amount.magnitude()?.abs());
    }
    Some(accuracy)
}

/// A number of seconds as an ISO-8601 duration.
fn seconds_as_duration(seconds: f64) -> Iso8601Duration {
    Iso8601Duration {
        value: format!("PT{seconds}S"),
    }
}

impl DvAbsoluteQuantity {
    /// This quantity plus a differential amount (`add`), or `None` for an
    /// amount that is not a `DV_DURATION`, the type each temporal descendant
    /// takes.
    #[must_use]
    pub fn add(&self, a_diff: &DvAmount) -> Option<Self> {
        let DvAmount::DvDuration(a_diff) = a_diff else {
            return None;
        };
        match self {
            Self::DvDate(date) => date.add(a_diff).map(Self::DvDate),
            Self::DvDateTime(date_time) => date_time.add(a_diff).map(Self::DvDateTime),
            Self::DvTime(time) => time.add(a_diff).map(Self::DvTime),
        }
    }

    /// This quantity minus a differential amount (`subtract`), under the
    /// conditions of [`Self::add`].
    #[must_use]
    pub fn subtract(&self, a_diff: &DvAmount) -> Option<Self> {
        let DvAmount::DvDuration(a_diff) = a_diff else {
            return None;
        };
        match self {
            Self::DvDate(date) => date.subtract(a_diff).map(Self::DvDate),
            Self::DvDateTime(date_time) => date_time.subtract(a_diff).map(Self::DvDateTime),
            Self::DvTime(time) => time.subtract(a_diff).map(Self::DvTime),
        }
    }

    /// The differential amount between two quantities of one concrete type
    /// (`diff`).
    #[must_use]
    pub fn diff(&self, other: &Self) -> Option<DvAmount> {
        match (self, other) {
            (Self::DvDate(left), Self::DvDate(right)) => left.diff(right),
            (Self::DvDateTime(left), Self::DvDateTime(right)) => left.diff(right),
            (Self::DvTime(left), Self::DvTime(right)) => left.diff(right),
            _ => None,
        }
        .map(DvAmount::DvDuration)
    }

    /// Whether the accuracy is Void (`accuracy_unknown`; `master06-quantity_package.adoc`
    /// §Accuracy and Uncertainty).
    #[must_use]
    pub fn accuracy_unknown(&self) -> bool {
        match self {
            Self::DvDate(date) => date.accuracy.is_none(),
            Self::DvDateTime(date_time) => date_time.accuracy.is_none(),
            Self::DvTime(time) => time.accuracy.is_none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn duration(value: &str) -> DvDuration {
        DvDuration {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
            magnitude_status: None,
            accuracy: None,
            accuracy_is_percent: None,
            value: value.to_owned(),
        }
    }

    /// The amount's half-range becomes seconds of duration before the sum.
    #[test]
    fn a_displaced_accuracy_sums_both_operands() {
        let mut amount = duration("P1D");
        amount.accuracy = Some(60.0);
        amount.accuracy_is_percent = Some(false);

        let combined = displaced_accuracy(Some(&duration("PT30M")), &amount).expect("both present");
        assert_eq!(
            combined.magnitude(),
            Some(1860.0),
            "30 minutes + 60 seconds"
        );
    }

    /// A percentage half-range is resolved against the amount's own magnitude
    /// before it can be added to a duration.
    #[test]
    fn a_percentage_resolves_against_its_own_magnitude() {
        let mut amount = duration("PT100S");
        amount.accuracy = Some(10.0);
        amount.accuracy_is_percent = Some(true);

        let combined = displaced_accuracy(Some(&duration("PT5S")), &amount).expect("both present");
        assert_eq!(combined.magnitude(), Some(15.0), "5s + 10% of 100s");
    }

    /// "Unknown, if either or both operand accuracies are unknown."
    #[test]
    fn either_operand_unknown_makes_the_result_unknown() {
        let plain = duration("P1D");
        assert!(displaced_accuracy(None, &plain).is_none());
        assert!(displaced_accuracy(Some(&duration("PT30M")), &plain).is_none());
    }

    /// `diff` returns a `DV_AMOUNT`, so the summed duration is reported as that
    /// amount's own accuracy in seconds.
    #[test]
    fn a_difference_accuracy_is_reported_in_seconds() {
        let combined = difference_accuracy(Some(&duration("PT30M")), Some(&duration("PT30S")));
        assert_eq!(
            combined,
            CombinedAccuracy::Known {
                accuracy: 1830.0,
                is_percent: false,
            }
        );
        assert_eq!(
            difference_accuracy(Some(&duration("PT30M")), None),
            CombinedAccuracy::Unknown
        );
    }
}
