// @generated-from-template templates/openehr-rm/data_types/quantity/date_time/dv_duration_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariants for `DV_DURATION`.
//!
//! `Value_valid`: `value` is a valid ISO-8601 duration (openEHR permits a
//! leading sign and a `W` designator mixed with the others). Plus the inherited
//! DV_AMOUNT / DV_QUANTIFIED invariants (`DV_DURATION` extends `DV_AMOUNT`). See
//! `dv_date_impl` for the NOTE on why value well-formedness is explicit.

use crate::v1_1::data_types::quantity::date_time::dv_duration::DvDuration;
use crate::v1_1::data_types::quantity::dv_amount_impl::{
    AmountAccuracy, CombinedAccuracy, combine, scale,
};
use crate::v1_1::data_types::quantity::dv_ordered::DvOrdered;
use crate::v1_1::data_types::quantity::dv_ordered_impl::push_normal_range_consistency;
use crate::v1_1::validate::valid_iso8601_duration;
use openehr_base::v1_2::foundation_types::time::iso8601_duration::Iso8601Duration;
use openehr_base::validate::{InvariantViolation, Validate};

impl DvDuration {
    /// Sum of this duration and `other`, or `None` when either value is not a
    /// valid ISO-8601 duration or the result is one no valid duration can
    /// carry.
    ///
    /// Spec: `dv_duration.adoc` §Functions `add` — "Sum of this Duration and
    /// `other`" — redefining `dv_amount.adoc` §Functions `add`. `DV_DURATION`
    /// effects `is_strictly_comparable_to` as "True, for any two Durations", so
    /// the inherited pre-condition never refuses a pair.
    ///
    /// The arithmetic is BASE's — `Iso8601_duration.add` — so the result stays
    /// a duration STRING with its designators intact, rather than a second of
    /// seconds this crate would have to render back.
    #[must_use]
    pub fn add(&self, other: &Self) -> Option<Self> {
        Self::carrying(
            self.iso().add(&other.iso())?,
            combine(self.amount_accuracy()?, other.amount_accuracy()?),
        )
    }

    /// Difference of this duration and `other`, under the same conditions as
    /// [`Self::add`].
    ///
    /// Spec: `dv_duration.adoc` §Functions `subtract`.
    #[must_use]
    pub fn subtract(&self, other: &Self) -> Option<Self> {
        Self::carrying(
            self.iso().subtract(&other.iso())?,
            combine(self.amount_accuracy()?, other.amount_accuracy()?),
        )
    }

    /// Product of this duration and `factor`.
    ///
    /// Spec: `dv_duration.adoc` §Functions `multiply` — "Product of this
    /// Duration and `factor`."
    #[must_use]
    pub fn multiply(&self, factor: f64) -> Option<Self> {
        Self::carrying(
            self.iso().multiply(factor)?,
            scale(self.amount_accuracy()?, factor),
        )
    }

    /// Negated version of this duration.
    ///
    /// Spec: `dv_duration.adoc` §Functions `negative` — "Assuming the current
    /// duration is positive, the negated version represents a time prior to
    /// some origin point, or a negative age (e.g. so-called 'adjusted age' of
    /// premature infant)."
    ///
    /// Negation changes the sign of the magnitude, not how well it was
    /// measured, so the accuracy is carried through unchanged — the one
    /// arithmetic function here for which that is a fact rather than a rule.
    #[must_use]
    pub fn negative(&self) -> Option<Self> {
        let mut negated = self.clone();
        negated.value = self.iso().negative()?.value;
        negated.normal_range = None;
        negated.normal_status = None;
        negated.magnitude_status = None;
        negated.other_reference_ranges = openehr_base::containers::present_nonempty(Vec::new());
        Some(negated)
    }

    /// Returns `true` when this duration's accuracy was not recorded.
    ///
    /// Spec: `dv_quantified.adoc` §Functions `accuracy_unknown`, effected for
    /// `DV_AMOUNT` by the `unknown_accuracy_value` sentinel.
    #[must_use]
    pub fn accuracy_unknown(&self) -> bool {
        self.accuracy.is_none_or(|value| value < 0.0)
    }

    /// This duration's `value` as the BASE ISO-8601 type that owns the
    /// arithmetic.
    fn iso(&self) -> Iso8601Duration {
        Iso8601Duration {
            value: self.value.clone(),
        }
    }

    /// This duration's accuracy as the `DV_AMOUNT` rule reads it, or `None`
    /// when `value` has no magnitude to measure a percentage against.
    fn amount_accuracy(&self) -> Option<AmountAccuracy> {
        Some(AmountAccuracy::measured(
            self.magnitude()?,
            self.accuracy,
            self.accuracy_is_percent,
        ))
    }

    /// This duration at a new value and accuracy.
    fn carrying(value: Iso8601Duration, accuracy: CombinedAccuracy) -> Option<Self> {
        let (accuracy, accuracy_is_percent) = match accuracy {
            CombinedAccuracy::Unrepresentable => return None,
            CombinedAccuracy::Unknown => (None, None),
            CombinedAccuracy::Known {
                accuracy,
                is_percent,
            } => (Some(accuracy), Some(is_percent)),
        };
        Some(Self {
            value: value.value,
            magnitude_status: None,
            accuracy,
            accuracy_is_percent,
            normal_range: None,
            normal_status: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
        })
    }
}

impl Validate for DvDuration {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_1::validate::generated::temporal_value_core(
            "DV_DURATION",
            valid_iso8601_duration(&self.value),
            out,
        );
        crate::v1_1::validate::generated::dv_amount_core(
            "DV_DURATION",
            self.accuracy,
            self.accuracy_is_percent,
            self.magnitude_status.as_deref(),
            out,
        );
        // Inherited DV_ORDERED Normal_range_and_status_consistency.
        push_normal_range_consistency(
            out,
            "DV_DURATION",
            self.normal_status.as_ref(),
            self.normal_range.as_deref(),
            &DvOrdered::DvDuration(self.clone()),
        );
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

    #[test]
    fn valid_duration() {
        assert!(duration("P1Y2M10DT2H30M").invariants().is_empty());
        assert!(duration("PT10H").invariants().is_empty());
    }

    #[test]
    fn invalid_duration() {
        let v = duration("10 hours").invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Value_valid failed on type DV_DURATION")
        );
    }

    /// `is_strictly_comparable_to` is "True, for any two Durations", so unlike
    /// `DV_QUANTITY` no pair is refused for incomparability — the arithmetic
    /// runs on the ISO-8601 values themselves.
    #[test]
    fn arithmetic_runs_on_the_iso_value() {
        let a = duration("PT2H");
        let b = duration("PT30M");

        assert_eq!(a.add(&b).expect("valid").magnitude(), Some(9000.0));
        assert_eq!(a.subtract(&b).expect("valid").magnitude(), Some(5400.0));
        assert_eq!(a.multiply(2.0).expect("valid").magnitude(), Some(14400.0));
        assert_eq!(a.negative().expect("valid").magnitude(), Some(-7200.0));
    }

    /// A value that is not a valid ISO-8601 duration has no arithmetic: BASE
    /// refuses to parse it, and the refusal is propagated rather than turned
    /// into a zero.
    #[test]
    fn an_invalid_value_has_no_arithmetic() {
        let bad = duration("10 hours");
        let good = duration("PT1H");
        assert!(bad.add(&good).is_none());
        assert!(good.add(&bad).is_none());
        assert!(bad.multiply(2.0).is_none());
        assert!(bad.negative().is_none());
    }

    /// `DV_DURATION` is a `DV_AMOUNT`, so its arithmetic carries the same
    /// accuracy rule — summed for add and subtract, scaled for multiply, and
    /// unchanged by negation, which alters the sign rather than the
    /// measurement.
    #[test]
    fn accuracy_follows_the_dv_amount_rule() {
        let mut a = duration("PT2H");
        a.accuracy = Some(60.0);
        a.accuracy_is_percent = Some(false);
        let mut b = duration("PT30M");
        b.accuracy = Some(30.0);
        b.accuracy_is_percent = Some(false);

        let sum = a.add(&b).expect("valid");
        assert!((sum.accuracy.expect("both recorded one") - 90.0).abs() < f64::EPSILON);
        assert_eq!(sum.accuracy_is_percent, Some(false));

        let scaled = a.multiply(3.0).expect("valid");
        assert!((scaled.accuracy.expect("scaled") - 180.0).abs() < f64::EPSILON);

        let negated = a.negative().expect("valid");
        assert!((negated.accuracy.expect("kept") - 60.0).abs() < f64::EPSILON);

        // Unrecorded on either side makes the result's unknown.
        let plain = duration("PT30M");
        assert!(a.add(&plain).expect("valid").accuracy.is_none());
    }

    /// `accuracy_unknown` reads both ways of not recording an accuracy.
    #[test]
    fn accuracy_unknown_reads_the_sentinel_and_the_absence() {
        let mut d = duration("PT1H");
        assert!(d.accuracy_unknown());
        d.accuracy = Some(-1.0);
        assert!(d.accuracy_unknown());
        d.accuracy = Some(0.0);
        assert!(!d.accuracy_unknown(), "0 means 100% accurate, not unknown");
    }
}
