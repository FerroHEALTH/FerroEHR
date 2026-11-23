// @generated-from-template templates/openehr-rm/data_types/quantity/dv_amount_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! The `DV_AMOUNT` accuracy rule and arithmetic, shared by its descendants.

use crate::v1_1::data_types::quantity::dv_amount::DvAmount;
use crate::v1_1::data_types::quantity::dv_ordered::DvOrdered;
use crate::v1_1::validate::valid_percentage;
use core::cmp::Ordering;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};

/// The `accuracy` recording "not recorded": "`unknown_accuracy_value` = -1"
/// (`master06-quantity_package.adoc` §Accuracy and Uncertainty).
pub const UNKNOWN_ACCURACY_VALUE: f64 = -1.0;

/// One operand of a `DV_AMOUNT` operation: its accuracy with the magnitude a
/// percentage is relative to, as an exact [`Decimal`].
#[derive(Debug, Clone, Copy)]
pub struct AmountAccuracy {
    /// The operand's magnitude, absent when it has no decimal form — which
    /// only matters if the two operands disagree about the accuracy form.
    magnitude: Option<Decimal>,
    /// The operand's `accuracy`.
    accuracy: Option<f64>,
    /// The operand's `accuracy_is_percent`.
    accuracy_is_percent: Option<bool>,
}

impl AmountAccuracy {
    /// The accuracy of an amount whose magnitude is an integer, such as a
    /// `DV_COUNT`.
    #[must_use]
    pub fn counted(
        magnitude: i64,
        accuracy: Option<f64>,
        accuracy_is_percent: Option<bool>,
    ) -> Self {
        Self {
            magnitude: Some(Decimal::from(magnitude)),
            accuracy,
            accuracy_is_percent,
        }
    }

    /// The accuracy of an amount whose magnitude is a real number, such as a
    /// `DV_QUANTITY`.
    #[must_use]
    pub fn measured(
        magnitude: f64,
        accuracy: Option<f64>,
        accuracy_is_percent: Option<bool>,
    ) -> Self {
        Self {
            // The decimal the float denotes: 0.1_f64 is 0.1.
            magnitude: Decimal::from_f64(magnitude),
            accuracy,
            accuracy_is_percent,
        }
    }
}

/// The accuracy a combined or scaled `DV_AMOUNT` carries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CombinedAccuracy {
    /// At least one operand did not record an accuracy, so neither does the
    /// result.
    Unknown,
    /// Every operand recorded one; this is the result, in the form named.
    Known {
        /// The combined accuracy value.
        accuracy: f64,
        /// Whether that value is a percentage rather than an absolute
        /// half-range.
        is_percent: bool,
    },
    /// The result exists as a number but no valid `DV_AMOUNT` can carry it —
    /// a percentage past 100, or a form conversion the operands do not admit.
    Unrepresentable,
}

impl AmountAccuracy {
    /// This operand's accuracy and form, or `None` when unrecorded; any negative
    /// value reads as unrecorded.
    fn recorded(self) -> Option<(f64, bool)> {
        let value = self.accuracy?;
        (value >= 0.0).then(|| (value, self.accuracy_is_percent == Some(true)))
    }

    /// This accuracy expressed as a percentage (`to_percent`) or as an absolute
    /// half-range, or `None` when the operand admits no such form.
    fn in_form(self, value: f64, is_percent: bool, to_percent: bool) -> Option<Decimal> {
        let value = Decimal::from_f64(value)?;
        if is_percent == to_percent {
            return Some(value);
        }
        let magnitude = self.magnitude?.abs();
        if is_percent {
            return value.checked_mul(magnitude)?.checked_div(hundred());
        }
        // A zero magnitude has no percentage form.
        if magnitude.is_zero() {
            return None;
        }
        value.checked_mul(hundred())?.checked_div(magnitude)
    }
}

/// The percentage base, as a `Decimal`.
fn hundred() -> Decimal {
    Decimal::from(100_u8)
}

/// Whether two magnitudes are the same decimal; an unavailable one equals
/// nothing.
fn equal_magnitudes(left: Option<f64>, right: Option<f64>) -> bool {
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    let (Some(left), Some(right)) = (Decimal::from_f64(left), Decimal::from_f64(right)) else {
        return false;
    };
    left == right
}

/// The accuracy of the sum or difference of `left` and `right`: added when
/// both are present, unknown otherwise, in "the form used in the larger of the
/// two quantities" (`master06-quantity_package.adoc` §Accuracy and
/// Uncertainty).
///
/// On equal magnitudes the absolute form wins: no openEHR spec governs the
/// tie, our own design.
#[must_use]
pub fn combine(left: AmountAccuracy, right: AmountAccuracy) -> CombinedAccuracy {
    let (Some((left_value, left_percent)), Some((right_value, right_percent))) =
        (left.recorded(), right.recorded())
    else {
        return CombinedAccuracy::Unknown;
    };

    let as_percent = match (left_percent, right_percent) {
        (true, true) => true,
        (false, false) => false,
        _ => match (left.magnitude, right.magnitude) {
            (Some(left_magnitude), Some(right_magnitude)) => {
                match left_magnitude.abs().cmp(&right_magnitude.abs()) {
                    Ordering::Greater => left_percent,
                    Ordering::Less => right_percent,
                    Ordering::Equal => false,
                }
            }
            _ => false,
        },
    };

    let (Some(left_value), Some(right_value)) = (
        left.in_form(left_value, left_percent, as_percent),
        right.in_form(right_value, right_percent, as_percent),
    ) else {
        return CombinedAccuracy::Unrepresentable;
    };

    let Some(sum) = left_value.checked_add(right_value) else {
        return CombinedAccuracy::Unrepresentable;
    };
    settle(sum, as_percent)
}

/// The accuracy of this amount scaled by `factor`: a percentage unchanged, an
/// absolute half-range scaled. No openEHR spec governs `multiply`: our own
/// design.
#[must_use]
pub fn scale(amount: AmountAccuracy, factor: f64) -> CombinedAccuracy {
    let Some((value, is_percent)) = amount.recorded() else {
        return CombinedAccuracy::Unknown;
    };
    let (Some(value), Some(factor)) = (Decimal::from_f64(value), Decimal::from_f64(factor)) else {
        return CombinedAccuracy::Unrepresentable;
    };
    if is_percent {
        return settle(value, true);
    }
    let Some(scaled) = value.checked_mul(factor.abs()) else {
        return CombinedAccuracy::Unrepresentable;
    };
    settle(scaled, false)
}

/// A computed accuracy as the result can carry it, or `Unrepresentable`.
fn settle(accuracy: Decimal, is_percent: bool) -> CombinedAccuracy {
    let Some(value) = accuracy.to_f64() else {
        return CombinedAccuracy::Unrepresentable;
    };
    if !value.is_finite() || (is_percent && !valid_percentage(value)) {
        return CombinedAccuracy::Unrepresentable;
    }
    CombinedAccuracy::Known {
        accuracy: value,
        // Accuracy_is_percent_validity: `accuracy = 0 implies not
        // accuracy_is_percent`, so a zero result is recorded as absolute.
        is_percent: is_percent && accuracy.is_sign_positive() && !accuracy.is_zero(),
    }
}

impl DvAmount {
    /// The sum of this amount and `other` (`dv_amount.adoc` `add`), or `None`
    /// when they differ in concrete type (`Pre_comparable`) or no valid amount
    /// carries the sum.
    #[must_use]
    pub fn add(&self, other: &Self) -> Option<Self> {
        match (self, other) {
            (Self::DvCount(left), Self::DvCount(right)) => left.add(right).map(Self::DvCount),
            (Self::DvDuration(left), Self::DvDuration(right)) => {
                left.add(right).map(Self::DvDuration)
            }
            (Self::DvProportion(left), Self::DvProportion(right)) => {
                left.add(right).map(Self::DvProportion)
            }
            (Self::DvQuantity(left), Self::DvQuantity(right)) => {
                left.add(right).map(Self::DvQuantity)
            }
            _ => None,
        }
    }

    /// The difference of this amount and `other` (`subtract`), under the
    /// conditions of [`Self::add`].
    #[must_use]
    pub fn subtract(&self, other: &Self) -> Option<Self> {
        match (self, other) {
            (Self::DvCount(left), Self::DvCount(right)) => left.subtract(right).map(Self::DvCount),
            (Self::DvDuration(left), Self::DvDuration(right)) => {
                left.subtract(right).map(Self::DvDuration)
            }
            (Self::DvProportion(left), Self::DvProportion(right)) => {
                left.subtract(right).map(Self::DvProportion)
            }
            (Self::DvQuantity(left), Self::DvQuantity(right)) => {
                left.subtract(right).map(Self::DvQuantity)
            }
            _ => None,
        }
    }

    /// This amount times `factor` (`multiply`).
    #[must_use]
    pub fn multiply(&self, factor: f64) -> Option<Self> {
        match self {
            Self::DvCount(count) => count.multiply(factor).map(Self::DvCount),
            Self::DvDuration(duration) => duration.multiply(factor).map(Self::DvDuration),
            Self::DvProportion(proportion) => proportion.multiply(factor).map(Self::DvProportion),
            Self::DvQuantity(quantity) => quantity.multiply(factor).map(Self::DvQuantity),
        }
    }

    /// This amount negated (`negative`), "e.g. a weight loss".
    #[must_use]
    pub fn negative(&self) -> Option<Self> {
        match self {
            Self::DvDuration(duration) => duration.negative().map(Self::DvDuration),
            other => other.multiply(-1.0),
        }
    }

    /// Whether this amount equals `other` in value (`is_equal`, from BASE
    /// `any.adoc`), so `PT60M` equals `PT1H`; different concrete types never do.
    #[must_use]
    pub fn is_equal(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::DvProportion(left), Self::DvProportion(right)) => left.is_equal(right),
            (Self::DvCount(left), Self::DvCount(right)) => left.magnitude == right.magnitude,
            (Self::DvDuration(left), Self::DvDuration(right)) => {
                equal_magnitudes(left.magnitude(), right.magnitude())
            }
            // `DV_QUANTITY` orders only within one unit, so equality takes the
            // same gate: 1000 g is not 1 kg to a class that cannot compare them.
            (Self::DvQuantity(left), Self::DvQuantity(right)) => {
                left.is_strictly_comparable_to(right)
                    && equal_magnitudes(Some(left.magnitude), Some(right.magnitude))
            }
            _ => false,
        }
    }

    /// Whether this amount is less than `other` (`less_than`:
    /// `magnitude < other.magnitude`), or `None` when not strictly comparable or
    /// a magnitude is unavailable.
    #[must_use]
    pub fn less_than(&self, other: &Self) -> Option<bool> {
        self.as_ordered().less_than(&other.as_ordered())
    }

    /// This amount as the `DV_ORDERED` it is.
    fn as_ordered(&self) -> DvOrdered {
        match self {
            Self::DvCount(count) => DvOrdered::DvCount(count.clone()),
            Self::DvDuration(duration) => DvOrdered::DvDuration(duration.clone()),
            Self::DvProportion(proportion) => DvOrdered::DvProportion(proportion.clone()),
            Self::DvQuantity(quantity) => DvOrdered::DvQuantity(quantity.clone()),
        }
    }

    /// Whether `number` is between 0 and 100 (`valid_percentage`).
    #[must_use]
    pub fn valid_percentage(number: f64) -> bool {
        valid_percentage(number)
    }

    /// Whether accuracy is unknown: the `-1` sentinel (`accuracy_unknown`).
    #[must_use]
    pub fn accuracy_unknown(&self) -> bool {
        let accuracy = match self {
            Self::DvCount(count) => count.accuracy,
            Self::DvDuration(duration) => duration.accuracy,
            Self::DvProportion(proportion) => proportion.accuracy,
            Self::DvQuantity(quantity) => quantity.accuracy,
        };
        accuracy.is_none_or(|value| value < 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accuracy(magnitude: f64, accuracy: f64, is_percent: bool) -> AmountAccuracy {
        AmountAccuracy::measured(magnitude, Some(accuracy), Some(is_percent))
    }

    fn unrecorded(magnitude: f64) -> AmountAccuracy {
        AmountAccuracy::measured(magnitude, None, None)
    }

    fn known(combined: CombinedAccuracy) -> (f64, bool) {
        match combined {
            CombinedAccuracy::Known {
                accuracy,
                is_percent,
            } => (accuracy, is_percent),
            other => panic!("expected a known accuracy, got {other:?}"),
        }
    }

    /// Recorded accuracies are added.
    #[test]
    fn two_recorded_accuracies_are_added() {
        let (value, is_percent) = known(combine(
            accuracy(50.0, 0.5, false),
            accuracy(30.0, 0.25, false),
        ));
        assert!((value - 0.75).abs() < f64::EPSILON);
        assert!(!is_percent);
    }

    /// "If either or both quantities has an unknown accuracy, the accuracy of
    /// the result is also unknown."
    #[test]
    fn one_unrecorded_accuracy_makes_the_result_unknown() {
        assert_eq!(
            combine(accuracy(50.0, 0.5, false), unrecorded(30.0)),
            CombinedAccuracy::Unknown
        );
        let sentinel = AmountAccuracy::measured(30.0, Some(UNKNOWN_ACCURACY_VALUE), None);
        assert_eq!(
            combine(accuracy(50.0, 0.5, false), sentinel),
            CombinedAccuracy::Unknown
        );
    }

    /// The smaller operand's half-range becomes a percentage of its magnitude when
    /// the larger is a percentage.
    #[test]
    fn the_larger_operand_names_the_form() {
        let (value, is_percent) = known(combine(
            accuracy(200.0, 10.0, true), // the larger: percent
            accuracy(50.0, 5.0, false),  // 5 on 50 == 10%
        ));
        assert!(is_percent, "the larger operand recorded a percentage");
        assert!((value - 20.0).abs() < 1e-9);

        let (value, is_percent) = known(combine(
            accuracy(200.0, 10.0, false), // the larger: absolute
            accuracy(50.0, 10.0, true),   // 10% of 50 == 5
        ));
        assert!(!is_percent);
        assert!((value - 15.0).abs() < 1e-9);
    }

    /// On equal magnitudes the absolute form wins.
    #[test]
    fn equal_magnitudes_settle_on_the_absolute_form() {
        let (value, is_percent) = known(combine(
            accuracy(50.0, 10.0, true), // 10% of 50 == 5
            accuracy(50.0, 2.0, false),
        ));
        assert!(!is_percent);
        assert!((value - 7.0).abs() < 1e-9);
    }

    /// A percentage sum past 100 is refused (`Accuracy_validity`).
    #[test]
    fn a_percentage_past_one_hundred_is_unrepresentable() {
        assert_eq!(
            combine(accuracy(50.0, 60.0, true), accuracy(50.0, 60.0, true)),
            CombinedAccuracy::Unrepresentable
        );
    }

    /// A zero magnitude has no percentage form: every percentage of it is the
    /// same zero, so an absolute half-range cannot be converted into one.
    #[test]
    fn a_zero_magnitude_has_no_percentage_form() {
        assert_eq!(
            combine(accuracy(200.0, 10.0, true), accuracy(0.0, 5.0, false)),
            CombinedAccuracy::Unrepresentable
        );
    }

    /// A zero result is absolute (`Accuracy_is_percent_validity`).
    #[test]
    fn a_zero_result_is_never_a_percentage() {
        let (value, is_percent) = known(combine(
            accuracy(50.0, 0.0, false),
            accuracy(50.0, 0.0, false),
        ));
        assert!((value - 0.0).abs() < f64::EPSILON);
        assert!(!is_percent);
    }

    /// Scaling leaves a percentage alone and scales an absolute half-range with
    /// the magnitude it measures.
    #[test]
    fn scaling_keeps_a_percentage_and_scales_a_half_range() {
        let (value, is_percent) = known(scale(accuracy(50.0, 10.0, true), 3.0));
        assert!(is_percent);
        assert!((value - 10.0).abs() < f64::EPSILON);

        let (value, is_percent) = known(scale(accuracy(50.0, 0.5, false), -3.0));
        assert!(!is_percent);
        assert!((value - 1.5).abs() < 1e-9, "the magnitude of the factor");

        assert_eq!(scale(unrecorded(50.0), 3.0), CombinedAccuracy::Unknown);
    }

    /// Two spellings of one duration are equal.
    #[test]
    fn equality_is_of_the_value_not_the_record() {
        use crate::v1_1::data_types::quantity::date_time::dv_duration::DvDuration;

        let duration = |value: &str| {
            DvAmount::DvDuration(DvDuration {
                normal_status: None,
                normal_range: None,
                other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
                magnitude_status: None,
                accuracy: None,
                accuracy_is_percent: None,
                value: value.to_owned(),
            })
        };

        let hour = duration("PT1H");
        let sixty_minutes = duration("PT60M");
        assert!(
            hour.is_equal(&sixty_minutes),
            "PT1H and PT60M are the same duration"
        );
        assert_eq!(hour.less_than(&sixty_minutes), Some(false));
        assert_eq!(sixty_minutes.less_than(&hour), Some(false));

        assert!(!hour.is_equal(&duration("PT2H")));
        // A malformed value equals nothing, including another malformed one.
        assert!(!duration("nope").is_equal(&duration("nope")));

        // Different concrete types are never equal — `DV_ORDERED` compares only
        // like with like.
        let count = DvAmount::DvCount(crate::v1_1::data_types::quantity::dv_count::DvCount {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
            magnitude_status: None,
            accuracy: None,
            accuracy_is_percent: None,
            magnitude: 3600,
        });
        assert!(!hour.is_equal(&count));
    }
}
