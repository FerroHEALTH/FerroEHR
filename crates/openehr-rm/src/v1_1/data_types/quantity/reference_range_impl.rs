// @generated-from-template templates/openehr-rm/data_types/quantity/reference_range_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariant + functions for `REFERENCE_RANGE`.
//!
//! Spec: RM 1.2.0
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.reference_range.adoc`.
//!
//! - `Range_is_simple` (invariant): each present, bounded limit of the `range`
//!   must be a *simple* `DV_ORDERED` — one that itself carries no
//!   `normal_range` and no `other_reference_ranges` (so reference ranges do
//!   not nest).
//! - `is_in_range(v)` (function): whether `v` lies inside `range`, via the
//!   base `Interval.has` semantics over openEHR ordered magnitudes
//!   (`dv_interval_impl` / `dv_ordered_impl`). `None` when the comparison is
//!   undecidable (incomparable types/units or unavailable magnitude).
//!
//! The class is generic over its limit type `T` as `REFERENCE_RANGE<T>`
//! declares, with `range: DV_INTERVAL<T>` restored by the generator (the class
//! table leaves `T` unused, tracker issue #5196); `ReferenceRange<DvQuantity>`
//! is the range of a `DV_QUANTITY`, `ReferenceRange<DvOrdered>` the open form.

use crate::v1_1::data_types::quantity::dv_ordered_impl::OrderedLimit;
use crate::v1_1::data_types::quantity::reference_range::ReferenceRange;
use openehr_base::validate::{InvariantViolation, Validate};

/// One side of `Range_is_simple`, whose clause is
/// `range.lower_unbounded **or else** range.lower.is_simple`.
///
/// An absent bound makes the right operand unevaluable, so the clause is
/// undecidable rather than false — and undecidable raises nothing, the same
/// reading `DV_INTERVAL` applies to the interval this constrains. BASE
/// `interval.adoc` §Attributes declares `lower`/`upper` `0..1` and requires no
/// value when the flag is false, so demanding one here refused a range
/// `DV_INTERVAL` itself accepts.
/// A limit whose simplicity is undecidable (`OrderedLimit::is_simple` gives
/// `None`) raises nothing, by the same reading.
fn limit_ok<T: OrderedLimit>(unbounded: bool, limit: Option<&T>) -> bool {
    unbounded || limit.is_none_or(|l| l.is_simple() != Some(false))
}

impl<T: OrderedLimit> ReferenceRange<T> {
    /// RM `REFERENCE_RANGE.is_in_range(v)`: `true` when `v` is inside `range`.
    /// `None` when `v` is not strictly comparable to the range limits or a
    /// magnitude is unavailable.
    #[must_use]
    pub fn is_in_range(&self, v: &T) -> Option<bool> {
        self.range.has(v)
    }
}

impl<T: OrderedLimit> Validate for ReferenceRange<T> {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        let r = &self.range;
        if !(limit_ok(r.lower_unbounded, r.lower.as_ref())
            && limit_ok(r.upper_unbounded, r.upper.as_ref()))
        {
            out.push(InvariantViolation::here(
                "Invariant Range_is_simple failed on type REFERENCE_RANGE",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_1::data_types::quantity::dv_interval::DvInterval;
    use crate::v1_1::data_types::quantity::dv_ordered::DvOrdered;
    use crate::v1_1::data_types::quantity::dv_quantity::DvQuantity;
    use crate::v1_1::data_types::text::dv_text::{DvText, DvTextData};

    fn quantity(magnitude: f64) -> DvQuantity {
        DvQuantity {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
            magnitude_status: None,
            accuracy: None,
            accuracy_is_percent: None,
            magnitude,
            precision: None,
            units: "kg".to_owned(),
            units_system: None,
            units_display_name: None,
        }
    }

    fn meaning() -> DvText {
        DvText::DvText(DvTextData {
            value: "normal".to_owned(),
            hyperlink: None,
            formatting: None,
            mappings: openehr_base::containers::present_nonempty(Vec::new()),
            language: None,
            encoding: None,
        })
    }

    fn range_with(lower: DvQuantity, upper: DvQuantity) -> ReferenceRange<DvOrdered> {
        ReferenceRange {
            meaning: meaning(),
            range: DvInterval {
                lower: Some(DvOrdered::DvQuantity(lower)),
                upper: Some(DvOrdered::DvQuantity(upper)),
                lower_unbounded: false,
                upper_unbounded: false,
                lower_included: true,
                upper_included: true,
            },
        }
    }

    #[test]
    fn simple_range_valid() {
        assert_eq!(
            range_with(quantity(0.0), quantity(10.0)).invariants(),
            Vec::<InvariantViolation>::new()
        );
    }

    #[test]
    fn nested_reference_range_invalid() {
        let mut top = quantity(10.0);
        // Make the upper limit non-simple by giving it its own normal range.
        top.normal_range = Some(Box::new(DvInterval {
            lower: Some(quantity(0.0)),
            upper: Some(quantity(10.0)),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        }));
        let v = range_with(quantity(0.0), top).invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Range_is_simple failed on type REFERENCE_RANGE"),
            "got {v:?}"
        );
    }

    #[test]
    fn is_in_range_membership() {
        let r = range_with(quantity(0.0), quantity(10.0));
        assert_eq!(
            r.is_in_range(&DvOrdered::DvQuantity(quantity(5.0))),
            Some(true)
        );
        assert_eq!(
            r.is_in_range(&DvOrdered::DvQuantity(quantity(11.0))),
            Some(false)
        );
        // A value of a different (incomparable) subtype is undecidable.
        let mut other = quantity(5.0);
        other.units = "mm[Hg]".to_owned();
        assert_eq!(r.is_in_range(&DvOrdered::DvQuantity(other)), None);
    }

    fn quantity_range(lower: DvQuantity, upper: DvQuantity) -> ReferenceRange<DvQuantity> {
        ReferenceRange {
            meaning: meaning(),
            range: DvInterval {
                lower: Some(lower),
                upper: Some(upper),
                lower_unbounded: false,
                upper_unbounded: false,
                lower_included: true,
                upper_included: false,
            },
        }
    }

    /// The range of a `REFERENCE_RANGE<DV_QUANTITY>` holds quantities, so its
    /// limits and the value tested need no `DV_ORDERED` wrapping.
    #[test]
    fn a_quantity_range_holds_quantities() {
        let r = quantity_range(quantity(0.0), quantity(10.0));
        assert_eq!(r.invariants(), Vec::<InvariantViolation>::new());
        assert_eq!(r.is_in_range(&quantity(5.0)), Some(true));
        assert_eq!(r.is_in_range(&quantity(10.0)), Some(false));
    }

    #[test]
    fn a_nested_quantity_limit_breaks_range_is_simple() {
        let mut top = quantity(10.0);
        top.other_reference_ranges =
            openehr_base::containers::present_nonempty(vec![quantity_range(
                quantity(0.0),
                quantity(1.0),
            )]);
        let v = quantity_range(quantity(0.0), top).invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Range_is_simple failed on type REFERENCE_RANGE"),
            "got {v:?}"
        );
    }
}
