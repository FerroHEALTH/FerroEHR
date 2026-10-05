// @generated-from-template templates/openehr-rm/data_types/quantity/dv_quantity_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariants for `DV_QUANTITY`.
//!
//! `DV_QUANTITY` declares no own invariants; it inherits the DV_AMOUNT
//! (`Accuracy_is_percent_validity`, `Accuracy_valid`) and DV_QUANTIFIED
//! (`Magnitude_status_valid`) invariants — see the shared helper.
//!
//! Plus the DV_ORDERED `Normal_range_and_status_consistency` invariant, via
//! the ordered-magnitude machinery in `dv_ordered_impl`.
//!
//! NOTE: the DV_ORDERED `Normal_status_validity` invariant (terminology)
//! is deferred to the composition validator + `openehr-term` (this crate has
//! no terminology dependency).

use crate::v1_2::data_types::quantity::dv_amount_impl::{
    AmountAccuracy, CombinedAccuracy, combine, scale,
};
use crate::v1_2::data_types::quantity::dv_ordered_impl::push_normal_range_consistency;
use crate::v1_2::data_types::quantity::dv_quantity::DvQuantity;
use openehr_base::validate::{InvariantViolation, Validate};

impl DvQuantity {
    /// Sum of this quantity and `other`, or `None` when they are not strictly
    /// comparable or the result is one no valid quantity can carry.
    ///
    /// Spec: `dv_quantity.adoc` §Functions `add` — "Sum of this `DV_QUANTITY`
    /// and `other`" — redefining `dv_amount.adoc` §Functions `add`, whose
    /// `Pre_comparable` pre-condition is `is_strictly_comparable_to (other)`:
    /// the same `units`, and the same `units_system` where one is stated.
    ///
    /// The accuracy of the result follows the `DV_AMOUNT` rule — see
    /// [`combine`].
    #[must_use]
    pub fn add(&self, other: &Self) -> Option<Self> {
        self.combined(other, self.magnitude + other.magnitude)
    }

    /// Difference of this quantity and `other`, under the same conditions as
    /// [`Self::add`].
    ///
    /// Spec: `dv_quantity.adoc` §Functions `subtract`, redefining
    /// `dv_amount.adoc` §Functions `subtract` — the accuracies are summed for
    /// subtraction exactly as for addition.
    #[must_use]
    pub fn subtract(&self, other: &Self) -> Option<Self> {
        self.combined(other, self.magnitude - other.magnitude)
    }

    /// Product of this quantity and `factor`, or `None` when the result is one
    /// no valid quantity can carry.
    ///
    /// Spec: `dv_quantity.adoc` §Functions `multiply` — "Product of this
    /// `DV_QUANTITY` and `factor`."
    ///
    /// Unlike `DV_COUNT.multiply`, a fractional result needs no adjudication:
    /// a quantity's magnitude is a `Real`, so scaling one stays in the type.
    /// The units are unchanged — a scalar factor carries no dimension — and the
    /// accuracy scales per [`scale`].
    #[must_use]
    pub fn multiply(&self, factor: f64) -> Option<Self> {
        self.rescaled(
            self.magnitude * factor,
            scale(self.amount_accuracy(), factor),
        )
    }

    /// Returns `true` when this quantity's accuracy was not recorded.
    ///
    /// Spec: `dv_quantified.adoc` §Functions `accuracy_unknown`, effected for
    /// `DV_AMOUNT` by the `unknown_accuracy_value` sentinel.
    #[must_use]
    pub fn accuracy_unknown(&self) -> bool {
        self.accuracy.is_none_or(|value| value < 0.0)
    }

    /// This quantity's accuracy as the `DV_AMOUNT` rule reads it.
    fn amount_accuracy(&self) -> AmountAccuracy {
        AmountAccuracy::measured(self.magnitude, self.accuracy, self.accuracy_is_percent)
    }

    /// This quantity combined with `other`, if they are strictly comparable.
    fn combined(&self, other: &Self, magnitude: f64) -> Option<Self> {
        if !self.is_strictly_comparable_to(other) {
            return None;
        }
        self.rescaled(
            magnitude,
            combine(self.amount_accuracy(), other.amount_accuracy()),
        )
    }

    /// This quantity at a new magnitude and accuracy, keeping the units.
    ///
    /// `precision`, `magnitude_status` and the reference ranges are dropped:
    /// they describe how THIS value was measured, and the spec gives no rule
    /// for combining them, so carrying one would be this implementation
    /// inventing a rule and presenting it as the model's.
    fn rescaled(&self, magnitude: f64, accuracy: CombinedAccuracy) -> Option<Self> {
        // A non-finite magnitude is not a quantity: the class types it as a
        // `Real`, and NaN or an infinity would flow into comparisons and
        // serialization as a value no reader can act on.
        if !magnitude.is_finite() {
            return None;
        }
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
            units: self.units.clone(),
            units_system: self.units_system.clone(),
            units_display_name: self.units_display_name.clone(),
            precision: None,
            magnitude_status: None,
            accuracy,
            accuracy_is_percent,
            normal_range: None,
            normal_status: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
        })
    }
}

impl Validate for DvQuantity {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_2::validate::generated::dv_amount_core(
            "DV_QUANTITY",
            self.accuracy,
            self.accuracy_is_percent,
            self.magnitude_status.as_deref(),
            out,
        );
        push_normal_range_consistency(
            out,
            "DV_QUANTITY",
            self.normal_status.as_ref(),
            self.normal_range.as_deref(),
            self,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quantity() -> DvQuantity {
        DvQuantity {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
            magnitude_status: None,
            accuracy: None,
            accuracy_is_percent: None,
            magnitude: 43.0,
            precision: Some(0),
            units: "kg".to_owned(),
            units_system: None,
            units_display_name: None,
        }
    }

    fn messages(q: &DvQuantity) -> Vec<String> {
        q.invariants().into_iter().map(|v| v.message).collect()
    }

    /// Unlike quantities do not add. `DV_ORDERED` says instances "can only be
    /// compared if they measure the same kind of physical quantity", and a sum
    /// of kg and m is no more meaningful than a comparison of them — so the
    /// refusal is asserted, not just the success.
    #[test]
    fn unlike_units_do_not_combine() {
        let kg = quantity();
        let mut metres = quantity();
        metres.units = "m".to_owned();

        assert!(kg.add(&metres).is_none(), "kg + m has no unit");
        assert!(kg.subtract(&metres).is_none());

        let mut other_kg = quantity();
        other_kg.magnitude = 7.0;
        let sum = kg.add(&other_kg).expect("same units");
        assert!((sum.magnitude - 50.0).abs() < f64::EPSILON);
        assert_eq!(sum.units, "kg", "the sum stays in the operands' units");
    }

    /// A units_system stated on one side and not the other is NOT the same
    /// quantity kind: the existing comparability rule requires both to match,
    /// and arithmetic follows the same rule rather than a looser one.
    #[test]
    fn a_units_system_mismatch_does_not_combine() {
        let plain = quantity();
        let mut systematised = quantity();
        systematised.units_system = Some("http://unitsofmeasure.org".to_owned());
        assert!(plain.add(&systematised).is_none());
    }

    /// "If accuracies are present in both quantities, they are added in the
    /// result, for both addition and subtraction operations" — the accuracy of
    /// a sum is specified, so it is carried rather than dropped.
    #[test]
    fn arithmetic_carries_the_accuracy_the_spec_specifies() {
        let mut a = quantity(); // 43 kg
        a.accuracy = Some(0.5);
        a.accuracy_is_percent = Some(false);
        let mut b = quantity();
        b.magnitude = 7.0;
        b.accuracy = Some(0.25);
        b.accuracy_is_percent = Some(false);

        for combined in [
            a.add(&b).expect("same units"),
            a.subtract(&b).expect("same units"),
        ] {
            assert_eq!(combined.accuracy_is_percent, Some(false));
            let accuracy = combined.accuracy.expect("both operands recorded one");
            assert!((accuracy - 0.75).abs() < f64::EPSILON);
        }

        // An unrecorded accuracy on either side makes the result's unknown.
        let mut plain = quantity();
        plain.magnitude = 7.0;
        let sum = a.add(&plain).expect("same units");
        assert!(sum.accuracy.is_none() && sum.accuracy_is_percent.is_none());
    }

    /// Scaling has no spec rule, so ours is asserted: a percentage of a
    /// magnitude survives the magnitude changing, and an absolute half-range —
    /// being in the quantity's own units — scales with it.
    #[test]
    fn scaling_carries_the_accuracy_forward() {
        let mut absolute = quantity();
        absolute.accuracy = Some(0.5);
        absolute.accuracy_is_percent = Some(false);
        let scaled = absolute.multiply(2.0).expect("finite");
        assert!((scaled.magnitude - 86.0).abs() < f64::EPSILON);
        assert_eq!(scaled.units, "kg");
        assert_eq!(scaled.accuracy_is_percent, Some(false));
        assert!((scaled.accuracy.expect("scaled") - 1.0).abs() < f64::EPSILON);

        let mut percent = quantity();
        percent.accuracy = Some(5.0);
        percent.accuracy_is_percent = Some(true);
        let scaled = percent.multiply(2.0).expect("finite");
        assert_eq!(scaled.accuracy_is_percent, Some(true));
        assert!((scaled.accuracy.expect("scaled") - 5.0).abs() < f64::EPSILON);

        // `precision` and `magnitude_status` have no combination rule and are
        // dropped rather than guessed at.
        assert!(scaled.precision.is_none() && scaled.magnitude_status.is_none());
    }

    /// A sum whose accuracy no valid quantity can carry is refused outright
    /// rather than returned as a value that fails its own class invariant.
    #[test]
    fn an_unrepresentable_accuracy_refuses_the_whole_operation() {
        let mut a = quantity();
        a.accuracy = Some(60.0);
        a.accuracy_is_percent = Some(true);
        let mut b = quantity();
        b.accuracy = Some(60.0);
        b.accuracy_is_percent = Some(true);
        assert!(a.add(&b).is_none(), "120% fails Accuracy_validity");
    }

    /// `accuracy_unknown` reads both ways of not recording an accuracy: the
    /// attribute being absent, and the `unknown_accuracy_value` sentinel.
    #[test]
    fn accuracy_unknown_reads_the_sentinel_and_the_absence() {
        let mut q = quantity();
        assert!(q.accuracy_unknown());
        q.accuracy = Some(-1.0);
        assert!(q.accuracy_unknown());
        q.accuracy = Some(0.0);
        assert!(!q.accuracy_unknown(), "0 means 100% accurate, not unknown");
    }

    /// A magnitude that is not finite is not a quantity: NaN or an infinity
    /// would flow into comparison and serialization as a value no reader can
    /// act on.
    #[test]
    fn a_non_finite_result_is_refused() {
        let q = quantity();
        assert!(q.multiply(f64::INFINITY).is_none());
        assert!(q.multiply(f64::NAN).is_none());
    }

    #[test]
    fn valid_quantity() {
        let mut q = quantity();
        q.magnitude_status = Some("=".to_owned());
        q.accuracy = Some(100.0);
        q.accuracy_is_percent = Some(true);
        assert!(q.invariants().is_empty());
    }

    #[test]
    fn percent_accuracy_out_of_range() {
        let mut q = quantity();
        q.accuracy_is_percent = Some(true);
        q.accuracy = Some(101.0);
        assert!(
            messages(&q)
                .contains(&"Invariant Accuracy_validity failed on type DV_QUANTITY".to_owned())
        );
        q.accuracy = Some(-1.0);
        assert!(
            messages(&q)
                .contains(&"Invariant Accuracy_validity failed on type DV_QUANTITY".to_owned())
        );
    }

    #[test]
    fn zero_accuracy_cannot_be_percent() {
        let mut q = quantity();
        q.accuracy_is_percent = Some(true);
        q.accuracy = Some(0.0);
        assert!(messages(&q).contains(
            &"Invariant Accuracy_is_percent_validity failed on type DV_QUANTITY".to_owned()
        ));
    }

    #[test]
    fn invalid_magnitude_status() {
        let mut q = quantity();
        q.magnitude_status = Some("bigger than".to_owned());
        assert!(
            messages(&q).contains(
                &"Invariant Magnitude_status_valid failed on type DV_QUANTITY".to_owned()
            )
        );
    }

    #[test]
    fn normal_range_and_status_consistency() {
        use crate::v1_2::data_types::quantity::dv_interval::DvInterval;
        use crate::v1_2::data_types::text::code_phrase::CodePhrase;
        use openehr_base::v1_3::prelude::TerminologyId;

        let range = |lo: f64, hi: f64| {
            Box::new(DvInterval {
                lower: Some(quantity_with(lo)),
                upper: Some(quantity_with(hi)),
                lower_unbounded: false,
                upper_unbounded: false,
                lower_included: true,
                upper_included: true,
            })
        };
        let status = |code: &str| CodePhrase {
            terminology_id: TerminologyId {
                value: "openehr_normal_statuses".to_owned(),
            },
            code_string: code.to_owned(),
            preferred_term: None,
        };

        // In range + status "N": consistent.
        let mut q = quantity(); // magnitude 43.0
        q.normal_range = Some(range(0.0, 100.0));
        q.normal_status = Some(status("N"));
        assert!(q.invariants().is_empty());

        // In range + status "H": inconsistent.
        let mut q = quantity();
        q.normal_range = Some(range(0.0, 100.0));
        q.normal_status = Some(status("H"));
        assert!(messages(&q).contains(
            &"Invariant Normal_range_and_status_consistency failed on type DV_QUANTITY".to_owned()
        ));

        // Out of range + status "N": inconsistent.
        let mut q = quantity();
        q.normal_range = Some(range(100.0, 200.0));
        q.normal_status = Some(status("N"));
        assert!(messages(&q).contains(
            &"Invariant Normal_range_and_status_consistency failed on type DV_QUANTITY".to_owned()
        ));

        // Out of range + status "H": consistent.
        let mut q = quantity();
        q.normal_range = Some(range(100.0, 200.0));
        q.normal_status = Some(status("H"));
        assert!(q.invariants().is_empty());
    }

    fn quantity_with(magnitude: f64) -> DvQuantity {
        let mut q = quantity();
        q.magnitude = magnitude;
        q
    }
}
