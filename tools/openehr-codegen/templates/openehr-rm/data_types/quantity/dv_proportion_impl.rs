// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written RM class invariants for `DV_PROPORTION`.
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.dv_proportion.adoc`
//! §Invariants (the seven own invariants), over the kind constants of
//! `…org.openehr.rm.data_types.proportion_kind.adoc` §Constants (`pk_ratio` 0,
//! `pk_unitary` 1, `pk_percent` 2, `pk_fraction` 3, `pk_integer_fraction` 4),
//! plus the inherited DV_AMOUNT / DV_QUANTIFIED / DV_ORDERED invariants.

use crate::v1_2::data_types::quantity::dv_amount_impl::{
    AmountAccuracy, CombinedAccuracy, combine, scale,
};
use crate::v1_2::data_types::quantity::dv_ordered_impl::push_normal_range_consistency;
use crate::v1_2::data_types::quantity::dv_proportion::DvProportion;
use core::cmp::Ordering;
use openehr_base::validate::{InvariantViolation, Validate};
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};

impl DvProportion {
    /// Sum of this proportion and `other`, or `None` when they are not strictly
    /// comparable or the result is one no valid proportion can carry.
    ///
    /// Spec: `dv_proportion.adoc` §Functions `add` — "Sum of two strictly
    /// comparable proportions", over `is_strictly_comparable_to`: "True if the
    /// `type` of this proportion is the same as the `type` of `other`."
    ///
    /// Proportions are added over a common denominator, which is what keeps
    /// the result inside its own kind's invariants: `pk_percent` fixes the
    /// denominator at 100 and `pk_unitary` at 1, so equal denominators are kept
    /// rather than multiplied — cross-multiplying two percentages would yield a
    /// denominator of 10000 and fail `Percent_validity`.
    #[must_use]
    pub fn add(&self, other: &Self) -> Option<Self> {
        self.combined(other, Decimal::checked_add)
    }

    /// Difference of this proportion and `other`, under the same conditions as
    /// [`Self::add`].
    ///
    /// Spec: `dv_proportion.adoc` §Functions `subtract` — "Difference between
    /// two strictly comparable proportions."
    #[must_use]
    pub fn subtract(&self, other: &Self) -> Option<Self> {
        self.combined(other, Decimal::checked_sub)
    }

    /// Product of this proportion and `factor`, or `None` when the result is
    /// one no valid proportion can carry.
    ///
    /// Spec: `dv_proportion.adoc` §Functions `multiply` — "Product of this
    /// Proportion and `factor`."
    ///
    /// The factor scales the numerator and leaves the denominator alone, which
    /// is what keeps `pk_percent`'s 100 and `pk_unitary`'s 1 in place. A kind
    /// whose `Fraction_validity` demands integers refuses a numerator the
    /// factor made fractional, rather than rounding it into a different ratio.
    #[must_use]
    pub fn multiply(&self, factor: f64) -> Option<Self> {
        let (numerator, denominator) = self.ratio()?;
        self.carrying(
            numerator.checked_mul(Decimal::from_f64(factor)?)?,
            denominator,
            scale(self.amount_accuracy()?, factor),
        )
    }

    /// Returns `true` when this proportion is considered equal to `other`.
    ///
    /// Spec: `dv_proportion.adoc` §Functions `is_equal`, effecting
    /// `dv_amount.adoc`'s abstract `is_equal`.
    ///
    /// Equality is of the RATIO, not of how it was written: `1/2` and `2/4` are
    /// the same proportion, so the test is cross-multiplication rather than
    /// field equality. The `type` must still match, because a proportion is
    /// only comparable to one of its own kind and `1/100` as a percentage means
    /// something a `pk_ratio` does not.
    ///
    /// The cross-multiplication runs in [`Decimal`]: whether two ratios are the
    /// same is an exact question, and asking it of binary floating point makes
    /// `0.1/0.3` and `1.0/3.0` disagree for reasons that have nothing to do
    /// with the proportions.
    #[must_use]
    pub fn is_equal(&self, other: &Self) -> bool {
        self.r#type == other.r#type && self.compare_to(other) == Some(Ordering::Equal)
    }

    /// This proportion's ratio against `other`'s, or `None` when either has no
    /// exact decimal form or the comparison overflows.
    ///
    /// The ONE ordering primitive of this class: [`Self::is_equal`] and the
    /// `DV_ORDERED` ordering both run through it, so `<`, `=` and `>` partition
    /// the same pairs by construction. They did not always — the ordering used
    /// to divide in binary floating point while equality cross-multiplied
    /// exactly, and `1/3` versus `0.1/0.3` came out equal AND less-than.
    ///
    /// Cross-multiplication rather than division: `a/b < c/d` iff `a·d < c·b`
    /// when `b·d` is positive, and the sign flips when it is not — exact, where
    /// dividing first would round both sides before comparing them.
    #[must_use]
    pub fn compare_to(&self, other: &Self) -> Option<Ordering> {
        let ((left_numerator, left_denominator), (right_numerator, right_denominator)) =
            (self.ratio()?, other.ratio()?);
        let left = left_numerator.checked_mul(right_denominator)?;
        let right = right_numerator.checked_mul(left_denominator)?;
        let ordering = left.cmp(&right);
        let denominators = left_denominator.checked_mul(right_denominator)?;
        Some(if denominators.is_sign_negative() {
            ordering.reverse()
        } else {
            ordering
        })
    }

    /// Returns `true` when this proportion's accuracy was not recorded.
    ///
    /// Spec: `dv_quantified.adoc` §Functions `accuracy_unknown`, effected for
    /// `DV_AMOUNT` by the `unknown_accuracy_value` sentinel.
    #[must_use]
    pub fn accuracy_unknown(&self) -> bool {
        self.accuracy.is_none_or(|value| value < 0.0)
    }

    /// This proportion's accuracy as the `DV_AMOUNT` rule reads it, or `None`
    /// when the denominator is zero and there is no magnitude.
    fn amount_accuracy(&self) -> Option<AmountAccuracy> {
        Some(AmountAccuracy::measured(
            self.magnitude()?,
            self.accuracy,
            self.accuracy_is_percent,
        ))
    }

    /// This proportion's numerator and denominator as exact decimals.
    pub(crate) fn ratio(&self) -> Option<(Decimal, Decimal)> {
        Some((
            Decimal::from_f64(self.numerator)?,
            Decimal::from_f64(self.denominator)?,
        ))
    }

    /// This proportion combined with `other` over a common denominator.
    fn combined(&self, other: &Self, op: fn(Decimal, Decimal) -> Option<Decimal>) -> Option<Self> {
        if !self.is_strictly_comparable_to(other) {
            return None;
        }
        let accuracy = combine(self.amount_accuracy()?, other.amount_accuracy()?);
        let (left_numerator, left_denominator) = self.ratio()?;
        let (right_numerator, right_denominator) = other.ratio()?;
        if left_denominator == right_denominator {
            return self.carrying(
                op(left_numerator, right_numerator)?,
                left_denominator,
                accuracy,
            );
        }
        self.carrying(
            op(
                left_numerator.checked_mul(right_denominator)?,
                right_numerator.checked_mul(left_denominator)?,
            )?,
            left_denominator.checked_mul(right_denominator)?,
            accuracy,
        )
    }

    /// This proportion at a new ratio and accuracy, or `None` when the result
    /// would not satisfy its own class invariants.
    ///
    /// `precision` is dropped: it states the decimal places the operands were
    /// expressed to, and the spec gives no rule for the precision of a sum.
    fn carrying(
        &self,
        numerator: Decimal,
        denominator: Decimal,
        accuracy: CombinedAccuracy,
    ) -> Option<Self> {
        let (numerator, denominator) = (numerator.to_f64()?, denominator.to_f64()?);
        let (accuracy, accuracy_is_percent) = match accuracy {
            CombinedAccuracy::Unrepresentable => return None,
            CombinedAccuracy::Unknown => (None, None),
            CombinedAccuracy::Known {
                accuracy,
                is_percent,
            } => (Some(accuracy), Some(is_percent)),
        };
        let combined = Self {
            numerator,
            denominator,
            r#type: self.r#type,
            precision: None,
            magnitude_status: None,
            accuracy,
            accuracy_is_percent,
            normal_range: None,
            normal_status: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
        };
        // A proportion that fails its own invariants is not a proportion —
        // `Fraction_validity` in particular rejects a fractional numerator on
        // the two integral kinds, which is where a scaled fraction lands.
        combined.invariants().is_empty().then_some(combined)
    }
}

impl Validate for DvProportion {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        // DV_PROPORTION own invariants (Type_validity, Valid_denominator,
        // Precision_validity, Fraction_validity, Unitary_validity,
        // Percent_validity) via the generated core.
        crate::v1_2::validate::generated::dv_proportion_core(
            self.numerator,
            self.denominator,
            self.r#type,
            self.precision,
            out,
        );

        // Inherited DV_AMOUNT + DV_QUANTIFIED invariants.
        crate::v1_2::validate::generated::dv_amount_core(
            "DV_PROPORTION",
            self.accuracy,
            self.accuracy_is_percent,
            self.magnitude_status.as_deref(),
            out,
        );
        // Inherited DV_ORDERED Normal_range_and_status_consistency.
        push_normal_range_consistency(
            out,
            "DV_PROPORTION",
            self.normal_status.as_ref(),
            self.normal_range.as_deref(),
            self,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ProportionKind codes, as readable test inputs (the runtime codes live in
    // the generated `validate::generated` core).
    const PK_UNITARY: i32 = 1;
    const PK_PERCENT: i32 = 2;
    const PK_FRACTION: i32 = 3;
    const PK_INTEGER_FRACTION: i32 = 4;

    fn proportion(
        numerator: f64,
        denominator: f64,
        ty: i32,
        precision: Option<i32>,
    ) -> DvProportion {
        DvProportion {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
            magnitude_status: None,
            accuracy: None,
            accuracy_is_percent: None,
            numerator,
            denominator,
            r#type: ty,
            precision,
        }
    }

    fn messages(p: &DvProportion) -> Vec<String> {
        p.invariants().into_iter().map(|v| v.message).collect()
    }

    #[test]
    fn valid_proportions() {
        assert!(
            proportion(5.0, 1.0, PK_UNITARY, None)
                .invariants()
                .is_empty()
        );
        assert!(
            proportion(1.0, 100.0, PK_PERCENT, None)
                .invariants()
                .is_empty()
        );
        assert!(
            proportion(5.0, 100.0, PK_FRACTION, None)
                .invariants()
                .is_empty()
        );
        assert!(proportion(0.5, 100.6, 0, None).invariants().is_empty()); // ratio
    }

    #[test]
    fn unitary_requires_denominator_one() {
        assert!(
            messages(&proportion(5.5, 2.0, PK_UNITARY, None))
                .contains(&"Invariant Unitary_validity failed on type DV_PROPORTION".to_owned())
        );
    }

    #[test]
    fn percent_requires_denominator_hundred() {
        assert!(
            messages(&proportion(5.5, 2.0, PK_PERCENT, None))
                .contains(&"Invariant Percent_validity failed on type DV_PROPORTION".to_owned())
        );
    }

    #[test]
    fn fraction_requires_integral() {
        assert!(
            messages(&proportion(5.5, 2.0, PK_INTEGER_FRACTION, None))
                .contains(&"Invariant Fraction_validity failed on type DV_PROPORTION".to_owned())
        );
    }

    /// CNF `master17.3-content_tc_data_types-quantity.adoc`
    /// (CONT-DV_PROPORTION-validate_open): `type 3, num 10, den 500, precision 1
    /// | rejected | fraction_validity` (and the type-4 analogue). A
    /// fraction / integer_fraction with a present, non-zero precision is not
    /// integral (`is_integral()` is "True … if precision is 0", RM
    /// dv_proportion.adoc §Functions), so `Fraction_validity` must reject it even
    /// though the numerator/denominator are whole numbers.
    #[test]
    fn fraction_with_nonzero_precision_rejected() {
        let fraction_validity =
            "Invariant Fraction_validity failed on type DV_PROPORTION".to_owned();
        assert!(
            messages(&proportion(10.0, 500.0, PK_FRACTION, Some(1))).contains(&fraction_validity)
        );
        assert!(
            messages(&proportion(10.0, 500.0, PK_INTEGER_FRACTION, Some(1)))
                .contains(&fraction_validity)
        );
        // Precision 0 with integer numerator/denominator is the valid fraction
        // shape (CNF `type 3, 10/100, precision 0 | accepted`).
        assert!(
            proportion(10.0, 100.0, PK_FRACTION, Some(0))
                .invariants()
                .is_empty()
        );
        assert!(
            proportion(10.0, 100.0, PK_INTEGER_FRACTION, Some(0))
                .invariants()
                .is_empty()
        );
    }

    #[test]
    fn denominator_zero_invalid() {
        assert!(
            messages(&proportion(5.5, 0.0, 0, None))
                .contains(&"Invariant Valid_denominator failed on type DV_PROPORTION".to_owned())
        );
    }

    #[test]
    fn type_out_of_range_invalid() {
        assert!(
            messages(&proportion(5.5, 1.0, -1, None))
                .contains(&"Invariant Type_validity failed on type DV_PROPORTION".to_owned())
        );
        assert!(
            messages(&proportion(5.5, 1.0, 5, None))
                .contains(&"Invariant Type_validity failed on type DV_PROPORTION".to_owned())
        );
    }

    #[test]
    fn precision_zero_requires_integral() {
        assert!(
            messages(&proportion(5.5, 1.0, 0, Some(0)))
                .contains(&"Invariant Precision_validity failed on type DV_PROPORTION".to_owned())
        );
    }

    /// Only proportions of the same kind combine — `is_strictly_comparable_to`
    /// is "True if the `type` of this proportion is the same as the `type` of
    /// `other`", so a percentage and a ratio do not add.
    #[test]
    fn only_the_same_kind_combines() {
        let percent = proportion(20.0, 100.0, PK_PERCENT, None);
        let ratio = proportion(20.0, 100.0, 0, None);
        assert!(percent.add(&ratio).is_none());
        assert!(percent.subtract(&ratio).is_none());
    }

    /// Two percentages keep the denominator 100 that `Percent_validity`
    /// demands: cross-multiplying them would give 10000 and produce a value
    /// that fails its own class.
    #[test]
    fn equal_denominators_are_kept_not_multiplied() {
        let a = proportion(20.0, 100.0, PK_PERCENT, None);
        let b = proportion(35.0, 100.0, PK_PERCENT, None);

        let sum = a.add(&b).expect("same kind");
        assert!((sum.numerator - 55.0).abs() < f64::EPSILON);
        assert!((sum.denominator - 100.0).abs() < f64::EPSILON);
        assert!(sum.invariants().is_empty(), "Percent_validity still holds");

        let difference = b.subtract(&a).expect("same kind");
        assert!((difference.numerator - 15.0).abs() < f64::EPSILON);
    }

    /// Unequal denominators go over a common one, and the result is a valid
    /// fraction because both operands were integral.
    #[test]
    fn unequal_denominators_go_over_a_common_one() {
        let half = proportion(1.0, 2.0, PK_FRACTION, None);
        let third = proportion(1.0, 3.0, PK_FRACTION, None);

        let sum = half.add(&third).expect("same kind");
        assert!((sum.numerator - 5.0).abs() < f64::EPSILON);
        assert!((sum.denominator - 6.0).abs() < f64::EPSILON);
        assert!(sum.invariants().is_empty());
    }

    /// A scaled fraction whose numerator stops being whole is refused rather
    /// than rounded: `Fraction_validity` demands integers of `pk_fraction`, and
    /// rounding would silently record a different ratio.
    #[test]
    fn a_fractional_numerator_is_refused_on_an_integral_kind() {
        let fraction = proportion(1.0, 2.0, PK_INTEGER_FRACTION, None);
        assert!(fraction.multiply(0.5).is_none(), "0.5/2 is not integral");

        let doubled = fraction.multiply(2.0).expect("2/2 is integral");
        assert!((doubled.numerator - 2.0).abs() < f64::EPSILON);
        assert!((doubled.denominator - 2.0).abs() < f64::EPSILON);

        // A unitary proportion keeps its denominator of 1 under scaling.
        let unitary = proportion(3.0, 1.0, PK_UNITARY, None);
        let scaled = unitary.multiply(2.5).expect("no integrality rule");
        assert!((scaled.numerator - 7.5).abs() < f64::EPSILON);
        assert!((scaled.denominator - 1.0).abs() < f64::EPSILON);
    }

    /// `<`, `=` and `>` must partition the same pairs. They did not: equality
    /// cross-multiplied in `Decimal` while the ordering divided in `f64`, so
    /// `1/3` and `0.1/0.3` were simultaneously equal AND less-than. The
    /// ordering now runs through the same primitive, so this holds by
    /// construction — and it is asserted, because that key also backs
    /// `DvInterval::has` and a boundary misjudgement there raises a spurious
    /// `Normal_range_and_status_consistency` violation on a valid commit.
    #[test]
    fn equality_and_ordering_cannot_disagree() {
        let pairs = [
            ((1.0, 3.0), (0.1, 0.3)),
            ((1.0, 2.0), (2.0, 4.0)),
            ((1.0, 2.0), (1.0, 3.0)),
            ((2.0, 3.0), (1.0, 3.0)),
            // A negative denominator flips the comparison: -1/-2 is +0.5.
            ((-1.0, -2.0), (1.0, 2.0)),
            ((1.0, -2.0), (1.0, 2.0)),
        ];
        for ((ln, ld), (rn, rd)) in pairs {
            let left = proportion(ln, ld, 0, None);
            let right = proportion(rn, rd, 0, None);
            let equal = left.is_equal(&right);
            let less = left.less_than(&right);
            let greater = right.less_than(&left);
            assert_eq!(
                [equal, less == Some(true), greater == Some(true)]
                    .iter()
                    .filter(|held| **held)
                    .count(),
                1,
                "{ln}/{ld} vs {rn}/{rd}: exactly one of <, =, > must hold \
                 (equal={equal}, less={less:?}, greater={greater:?})"
            );
        }
    }

    /// `is_equal` compares the RATIO: 1/2 and 2/4 are the same proportion, and
    /// the decimal cross-multiplication makes 1/3 and 0.1/0.3 agree where
    /// binary floating point would not.
    #[test]
    fn is_equal_compares_the_ratio_not_the_fields() {
        let half = proportion(1.0, 2.0, PK_FRACTION, None);
        assert!(half.is_equal(&proportion(2.0, 4.0, PK_FRACTION, None)));
        assert!(!half.is_equal(&proportion(1.0, 3.0, PK_FRACTION, None)));

        // A different kind is never equal, whatever the ratio.
        assert!(!half.is_equal(&proportion(1.0, 2.0, 0, None)));

        let third = proportion(1.0, 3.0, 0, None);
        assert!(
            third.is_equal(&proportion(0.1, 0.3, 0, None)),
            "exact in Decimal, unequal in binary floating point"
        );
    }

    /// `DV_PROPORTION` is a `DV_AMOUNT`, so its arithmetic carries the same
    /// accuracy rule as every other descendant.
    #[test]
    fn accuracy_follows_the_dv_amount_rule() {
        let mut a = proportion(20.0, 100.0, PK_PERCENT, None);
        a.accuracy = Some(0.01);
        a.accuracy_is_percent = Some(false);
        let mut b = proportion(30.0, 100.0, PK_PERCENT, None);
        b.accuracy = Some(0.02);
        b.accuracy_is_percent = Some(false);

        let sum = a.add(&b).expect("same kind");
        assert!((sum.accuracy.expect("both recorded one") - 0.03).abs() < f64::EPSILON);
        assert_eq!(sum.accuracy_is_percent, Some(false));

        assert!(
            a.add(&proportion(30.0, 100.0, PK_PERCENT, None))
                .expect("same kind")
                .accuracy
                .is_none()
        );
    }

    /// `accuracy_unknown` reads both ways of not recording an accuracy.
    #[test]
    fn accuracy_unknown_reads_the_sentinel_and_the_absence() {
        let mut p = proportion(1.0, 2.0, PK_FRACTION, None);
        assert!(p.accuracy_unknown());
        p.accuracy = Some(-1.0);
        assert!(p.accuracy_unknown());
        p.accuracy = Some(0.0);
        assert!(!p.accuracy_unknown(), "0 means 100% accurate, not unknown");
    }
}
