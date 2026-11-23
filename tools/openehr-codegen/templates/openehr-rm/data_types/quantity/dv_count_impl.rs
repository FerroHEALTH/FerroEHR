// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! `DV_COUNT` invariants: the inherited `DV_AMOUNT`, `DV_QUANTIFIED` and
//! `DV_ORDERED` ones.

use crate::v1_2::data_types::quantity::dv_amount_impl::{
    AmountAccuracy, CombinedAccuracy, combine, scale,
};
use crate::v1_2::data_types::quantity::dv_count::DvCount;
use crate::v1_2::data_types::quantity::dv_ordered_impl::push_normal_range_consistency;
use openehr_base::validate::{InvariantViolation, Validate};
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};

impl DvCount {
    // `less_than` and `is_strictly_comparable_to` come from `ordered_limit!`.

    /// The sum of this count and `other` (`dv_count.adoc` `add`), or `None` on
    /// overflow; the ranges are not carried, the accuracy is ([`combine`]).
    #[must_use]
    pub fn add(&self, other: &Self) -> Option<Self> {
        self.combined(other, self.magnitude.checked_add(other.magnitude)?)
    }

    /// The difference of this count and `other` (`subtract`), or `None` on
    /// overflow; a negative result is valid.
    #[must_use]
    pub fn subtract(&self, other: &Self) -> Option<Self> {
        self.combined(other, self.magnitude.checked_sub(other.magnitude)?)
    }

    /// This count times `factor` (`multiply`), or `None` when the product is not
    /// a whole number: no openEHR spec defines a rounding.
    #[must_use]
    pub fn multiply(&self, factor: f64) -> Option<Self> {
        // The decimal the float denotes; `None` for NaN and the infinities.
        let product = Decimal::from(self.magnitude).checked_mul(Decimal::from_f64(factor)?)?;
        let magnitude = product.is_integer().then(|| product.to_i64())??;
        Self::of_magnitude(magnitude, scale(self.amount_accuracy(), factor))
    }

    /// Whether this count's accuracy was not recorded (`accuracy_unknown`).
    #[must_use]
    pub fn accuracy_unknown(&self) -> bool {
        self.accuracy.is_none_or(|value| value < 0.0)
    }

    /// This count's accuracy as the `DV_AMOUNT` rule reads it.
    fn amount_accuracy(&self) -> AmountAccuracy {
        AmountAccuracy::counted(self.magnitude, self.accuracy, self.accuracy_is_percent)
    }

    /// This count combined with `other`, carrying the `DV_AMOUNT` accuracy.
    fn combined(&self, other: &Self, magnitude: i64) -> Option<Self> {
        Self::of_magnitude(
            magnitude,
            combine(self.amount_accuracy(), other.amount_accuracy()),
        )
    }

    /// A count of `magnitude` and `accuracy`, with the ranges cleared.
    fn of_magnitude(magnitude: i64, accuracy: CombinedAccuracy) -> Option<Self> {
        let (accuracy, accuracy_is_percent) = match accuracy {
            CombinedAccuracy::Unrepresentable => return None,
            CombinedAccuracy::Unknown => (None, None),
            CombinedAccuracy::Known {
                accuracy,
                is_percent,
            } => (Some(accuracy), Some(is_percent)),
        };
        Some(Self {
            magnitude,
            magnitude_status: None,
            accuracy,
            accuracy_is_percent,
            normal_range: None,
            normal_status: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
        })
    }
}

impl Validate for DvCount {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_2::validate::generated::dv_amount_core(
            "DV_COUNT",
            self.accuracy,
            self.accuracy_is_percent,
            self.magnitude_status.as_deref(),
            out,
        );
        push_normal_range_consistency(
            out,
            "DV_COUNT",
            self.normal_status.as_deref(),
            self.normal_range.as_deref(),
            self,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count() -> DvCount {
        DvCount {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
            magnitude_status: None,
            accuracy: None,
            accuracy_is_percent: None,
            magnitude: 3,
        }
    }

    /// The ranges are not carried; the accuracy is.
    #[test]
    fn add_and_subtract_are_magnitude_arithmetic() {
        let a = count();
        let mut b = count();
        b.magnitude = 4;

        let sum = a.add(&b).expect("no overflow");
        assert_eq!(sum.magnitude, a.magnitude + 4);
        assert!(
            sum.normal_range.is_none(),
            "the ranges belong to the operands, not the result"
        );
        assert!(sum.accuracy.is_none(), "neither operand recorded one");
        assert_eq!(
            a.subtract(&b).expect("no overflow").magnitude,
            a.magnitude - 4
        );
    }

    /// Recorded accuracies are added.
    #[test]
    fn accuracy_follows_the_dv_amount_rule() {
        let mut a = count();
        a.accuracy = Some(0.5);
        a.accuracy_is_percent = Some(false);
        let mut b = count();
        b.magnitude = 4;
        b.accuracy = Some(0.25);
        b.accuracy_is_percent = Some(false);

        let sum = a.add(&b).expect("no overflow");
        assert_eq!(sum.accuracy_is_percent, Some(false));
        assert!((sum.accuracy.expect("both recorded one") - 0.75).abs() < f64::EPSILON);

        // Scaling keeps a percentage and scales an absolute half-range.
        let scaled = a.multiply(2.0).expect("whole product");
        assert_eq!(scaled.magnitude, 6);
        assert!((scaled.accuracy.expect("scaled") - 1.0).abs() < f64::EPSILON);
    }

    /// Ten counts times a tenth is one count.
    #[test]
    fn a_decimal_factor_means_the_decimal_not_its_binary_approximation() {
        let mut ten = count();
        ten.magnitude = 10;
        assert_eq!(ten.multiply(0.1).expect("one tenth of ten").magnitude, 1);
        assert_eq!(ten.multiply(0.3).expect("three tenths of ten").magnitude, 3);
        assert!(ten.multiply(0.25).is_none(), "2.5 is not a whole count");
    }

    /// `accuracy_unknown` reads both ways of not recording an accuracy.
    #[test]
    fn accuracy_unknown_reads_the_sentinel_and_the_absence() {
        let mut c = count();
        assert!(c.accuracy_unknown());
        c.accuracy = Some(-1.0);
        assert!(c.accuracy_unknown());
        c.accuracy = Some(0.0);
        assert!(!c.accuracy_unknown(), "0 means 100% accurate, not unknown");
    }

    /// Overflow is refused.
    #[test]
    fn overflow_is_refused_not_wrapped() {
        let mut a = count();
        a.magnitude = i64::MAX;
        let mut b = count();
        b.magnitude = 1;
        assert!(a.add(&b).is_none());

        a.magnitude = i64::MIN;
        assert!(a.subtract(&b).is_none());
    }

    /// A fractional product is refused.
    #[test]
    fn multiply_returns_whole_products_and_refuses_the_rest() {
        let mut a = count();
        a.magnitude = 6;
        assert_eq!(a.multiply(2.0).expect("whole").magnitude, 12);
        assert_eq!(a.multiply(0.5).expect("whole").magnitude, 3);
        assert!(a.multiply(0.4).is_none(), "2.4 is not a count");
        assert!(a.multiply(f64::INFINITY).is_none());
        assert!(a.multiply(f64::NAN).is_none());
    }

    #[test]
    fn valid_count() {
        assert_eq!(count().invariants(), Vec::<InvariantViolation>::new());
    }

    #[test]
    fn invalid_magnitude_status() {
        let mut c = count();
        c.magnitude_status = Some("approx".to_owned());
        assert!(
            c.invariants()
                .iter()
                .any(|v| v.message == "Invariant Magnitude_status_valid failed on type DV_COUNT")
        );
    }
}
