// @generated-from-template templates/openehr-rm/data_types/quantity/dv_interval_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! `DV_INTERVAL` invariants and `has` (RM 1.2.0
//! `org.openehr.rm.data_types.dv_interval.adoc`): the inherited
//! `Lower_included_valid` and `Upper_included_valid`, and the redefined
//! `Limits_consistent` through [`OrderedLimit`], an undecidable comparison
//! not reported. All violations name `DV_INTERVAL`.

#![allow(
    clippy::disallowed_types,
    reason = "the carrier is #[cfg(test)]-only (an undecidable-element fixture), so an expect \
              would be unfulfilled on the lib lane (#1694 boundary class)"
)]

use crate::v1_2::data_types::quantity::dv_interval::DvInterval;
use crate::v1_2::data_types::quantity::dv_ordered_impl::OrderedLimit;
use openehr_base::validate::{InvariantViolation, Validate};

impl<T: OrderedLimit> DvInterval<T> {
    /// `Interval.has(v)` honouring the `*_included` flags, or `None` when a bound
    /// is missing or a comparison is undecidable.
    #[must_use]
    pub fn has(&self, v: &T) -> Option<bool> {
        let lower_ok = if self.lower_unbounded {
            true
        } else {
            let lower = self.lower.as_ref()?;
            if self.lower_included {
                lower.less_or_equal(v)?
            } else {
                lower.less_than(v)?
            }
        };
        if !lower_ok {
            return Some(false);
        }
        let upper_ok = if self.upper_unbounded {
            true
        } else {
            let upper = self.upper.as_ref()?;
            if self.upper_included {
                v.less_or_equal(upper)?
            } else {
                v.less_than(upper)?
            }
        };
        Some(upper_ok)
    }
}

impl<T: OrderedLimit> Validate for DvInterval<T> {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        if self.lower_unbounded && self.lower_included {
            out.push(InvariantViolation::here(
                "Invariant Lower_included_valid failed on type DV_INTERVAL",
            ));
        }
        if self.upper_unbounded && self.upper_included {
            out.push(InvariantViolation::here(
                "Invariant Upper_included_valid failed on type DV_INTERVAL",
            ));
        }
        // NOTE: BASE `interval.adoc` §Invariants has no bound-presence rule.
        if !self.lower_unbounded
            && !self.upper_unbounded
            && let (Some(lower), Some(upper)) = (self.lower.as_ref(), self.upper.as_ref())
        {
            let violated = match lower.strictly_comparable(upper) {
                Some(false) => true,
                Some(true) => lower.less_or_equal(upper) == Some(false),
                None => false, // undecidable at this element type/value
            };
            if violated {
                out.push(InvariantViolation::here(
                    "Invariant Limits_consistent failed on type DV_INTERVAL",
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::data_types::quantity::dv_ordered::DvOrdered;
    use crate::v1_2::data_types::quantity::dv_quantity::DvQuantity;

    /// The wire flags a decoded `DV_INTERVAL<DV_QUANTITY>` carries, as
    /// `(lower_unbounded, lower_included, upper_unbounded, upper_included)`.
    fn decoded_flags(json: &str) -> (bool, bool, bool, bool) {
        let v: DvInterval<DvQuantity> = serde_json::from_str(json).unwrap();
        (
            v.lower_unbounded,
            v.lower_included,
            v.upper_unbounded,
            v.upper_included,
        )
    }

    // ITS-JSON INTERVAL requires all four flags, and BASE interval.adoc ties
    // none to its bound; an omitted flag follows its bound (our own design, #5203).
    #[test]
    fn an_omitted_flag_follows_its_bound() {
        let q = r#"{"_type": "DV_QUANTITY", "magnitude": 1.0, "units": "kg"}"#;
        let lower_only = format!(r#"{{"_type": "DV_INTERVAL", "lower": {q}}}"#);
        assert_eq!(decoded_flags(&lower_only), (false, true, true, false));
        let upper_only = format!(r#"{{"_type": "DV_INTERVAL", "upper": {q}}}"#);
        assert_eq!(decoded_flags(&upper_only), (true, false, false, true));
        let both = format!(r#"{{"_type": "DV_INTERVAL", "lower": {q}, "upper": {q}}}"#);
        assert_eq!(decoded_flags(&both), (false, true, false, true));
        assert_eq!(
            decoded_flags(r#"{"_type": "DV_INTERVAL"}"#),
            (true, false, true, false)
        );
    }

    #[test]
    fn a_stated_flag_is_kept_and_the_canonical_form_round_trips() {
        let json = r#"{"_type": "DV_INTERVAL", "upper": {"_type": "DV_QUANTITY", "magnitude": 1.0, "units": "kg"}, "lower_unbounded": false, "upper_unbounded": false, "lower_included": true, "upper_included": false}"#;
        assert_eq!(decoded_flags(json), (false, true, false, false));
        let v: DvInterval<DvQuantity> = serde_json::from_str(json).unwrap();
        let written = serde_json::to_string(&v).unwrap();
        let again: DvInterval<DvQuantity> = serde_json::from_str(&written).unwrap();
        assert_eq!(again, v);
        assert_eq!(
            serde_json::to_value(&again).unwrap(),
            serde_json::from_str::<serde_json::Value>(json).unwrap()
        );
    }

    fn quantity(magnitude: f64, units: &str) -> DvQuantity {
        DvQuantity {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
            magnitude_status: None,
            accuracy: None,
            accuracy_is_percent: None,
            magnitude,
            precision: None,
            units: units.to_owned(),
            units_system: None,
            units_display_name: None,
        }
    }

    fn interval(lower: f64, upper: f64) -> DvInterval<DvQuantity> {
        DvInterval {
            lower: Some(quantity(lower, "kg")),
            upper: Some(quantity(upper, "kg")),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        }
    }

    #[test]
    fn valid_interval() {
        assert_eq!(
            interval(1.0, 4.0).invariants(),
            Vec::<InvariantViolation>::new()
        );
        // Equal limits are consistent (lower <= upper).
        assert_eq!(
            interval(4.0, 4.0).invariants(),
            Vec::<InvariantViolation>::new()
        );
    }

    #[test]
    fn lower_included_invalid() {
        let mut i = interval(1.0, 4.0);
        i.lower_unbounded = true; // still lower_included
        let v = i.invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Lower_included_valid failed on type DV_INTERVAL"),
            "got {v:?}"
        );
    }

    #[test]
    fn absent_bound_with_bounded_flag_is_accepted() {
        // An absent bound with a false `*_unbounded` flag violates nothing.
        let half_open = DvInterval::<DvQuantity> {
            lower: None,
            upper: Some(quantity(1.0, "s")),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: false,
            upper_included: false,
        };
        assert_eq!(half_open.invariants(), Vec::<InvariantViolation>::new());
        // The flagged half-open form is equally clean.
        let proper = DvInterval::<DvQuantity> {
            lower: None,
            upper: Some(quantity(1.0, "s")),
            lower_unbounded: true,
            upper_unbounded: false,
            lower_included: false,
            upper_included: true,
        };
        assert_eq!(proper.invariants(), Vec::<InvariantViolation>::new());
        // Mirror on the upper side.
        let no_upper = DvInterval::<DvQuantity> {
            lower: Some(quantity(1.0, "s")),
            upper: None,
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: false,
        };
        assert_eq!(no_upper.invariants(), Vec::<InvariantViolation>::new());
    }

    #[test]
    fn inverted_limits_fail_limits_consistent() {
        let v = interval(10.0, 2.0).invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Limits_consistent failed on type DV_INTERVAL"),
            "got {v:?}"
        );
    }

    #[test]
    fn incomparable_limits_fail_limits_consistent() {
        let mut i = interval(1.0, 4.0);
        i.upper = Some(quantity(4.0, "mm[Hg]")); // different units
        let v = i.invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Limits_consistent failed on type DV_INTERVAL"),
            "got {v:?}"
        );
    }

    #[test]
    fn unbounded_side_skips_limits_consistent() {
        let mut i = interval(10.0, 2.0);
        i.upper_unbounded = true;
        i.upper_included = false;
        i.upper = None;
        assert_eq!(i.invariants(), Vec::<InvariantViolation>::new());
    }

    #[test]
    fn undecidable_value_elements_skip_limits_consistent() {
        // serde_json::Value elements (the dispatcher fallback) can't be
        // compared → only boundary-flag invariants run.
        let i: DvInterval<serde_json::Value> = DvInterval {
            lower: Some(serde_json::json!({"magnitude": 10.0})),
            upper: Some(serde_json::json!({"magnitude": 2.0})),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        };
        assert_eq!(i.invariants(), Vec::<InvariantViolation>::new());
    }

    #[test]
    fn has_membership() {
        let i = interval(1.0, 4.0);
        assert_eq!(i.has(&quantity(2.0, "kg")), Some(true));
        assert_eq!(i.has(&quantity(1.0, "kg")), Some(true)); // inclusive lower
        assert_eq!(i.has(&quantity(5.0, "kg")), Some(false));
        assert_eq!(i.has(&quantity(2.0, "mm[Hg]")), None); // incomparable

        let mut open = interval(1.0, 4.0);
        open.lower_included = false;
        assert_eq!(open.has(&quantity(1.0, "kg")), Some(false)); // exclusive lower
    }

    #[test]
    fn has_over_dv_ordered_enum() {
        let i: DvInterval<DvOrdered> = DvInterval {
            lower: Some(DvOrdered::DvQuantity(quantity(1.0, "kg"))),
            upper: Some(DvOrdered::DvQuantity(quantity(4.0, "kg"))),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        };
        assert_eq!(
            i.has(&DvOrdered::DvQuantity(quantity(3.0, "kg"))),
            Some(true)
        );
        assert_eq!(i.invariants(), Vec::<InvariantViolation>::new());
    }
}
