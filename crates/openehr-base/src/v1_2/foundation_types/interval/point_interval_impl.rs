// @generated-from-template templates/openehr-base/foundation_types/interval/point_interval_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! `Point_interval` invariants.
//!
//! Its own `Inv_point` (`org.openehr.base.foundation_types.point_interval.adoc`)
//! and the inherited `Interval` invariants, `Limits_comparable` excepted.

use super::point_interval::PointInterval;
use crate::validate::{InvariantViolation, Validate};

impl<T: PartialOrd> Validate for PointInterval<T> {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        // Inv_point: the two bounds denote the same value. Both bounds absent
        // satisfies it (Void = Void); one present and one absent does not.
        let is_point = match (self.lower.as_ref(), self.upper.as_ref()) {
            (None, None) => true,
            (Some(l), Some(u)) => l == u,
            _ => false,
        };
        if !is_point {
            out.push(InvariantViolation::here(
                "Invariant Inv_point failed on type POINT_INTERVAL",
            ));
        }
        if self.lower_unbounded && self.lower_included {
            out.push(InvariantViolation::here(
                "Invariant Lower_included_valid failed on type INTERVAL",
            ));
        }
        if self.upper_unbounded && self.upper_included {
            out.push(InvariantViolation::here(
                "Invariant Upper_included_valid failed on type INTERVAL",
            ));
        }
        if !self.lower_unbounded
            && !self.upper_unbounded
            && let (Some(l), Some(u)) = (self.lower.as_ref(), self.upper.as_ref())
            && l > u
        {
            out.push(InvariantViolation::here(
                "Invariant Limits_consistent failed on type INTERVAL",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(lower: Option<i32>, upper: Option<i32>) -> PointInterval<i32> {
        PointInterval {
            lower,
            upper,
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        }
    }

    #[test]
    fn point_interval_valid() {
        let p = PointInterval {
            lower: Some(5),
            upper: Some(5),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        };
        assert_eq!(p.invariants(), Vec::<InvariantViolation>::new());
    }

    /// `point_interval.adoc` §Invariants `Inv_point` (`lower = upper`) holds for
    /// equal bounds and for both bounds absent.
    #[test]
    fn inv_point_accepts_equal_and_both_absent_bounds() {
        assert_eq!(
            point(Some(5), Some(5)).invariants(),
            Vec::<InvariantViolation>::new()
        );
        assert_eq!(
            point(None, None).invariants(),
            Vec::<InvariantViolation>::new()
        );
    }

    /// `point_interval.adoc` §Invariants `Inv_point` fails when the two bounds
    /// denote different values, including one-present/one-absent.
    #[test]
    fn inv_point_refuses_differing_bounds() {
        for bad in [
            point(Some(5), Some(6)),
            point(Some(5), None),
            point(None, Some(6)),
        ] {
            let v = bad.invariants();
            assert!(
                v.iter()
                    .any(|m| m.message == "Invariant Inv_point failed on type POINT_INTERVAL"),
                "got {v:?}"
            );
        }
    }
}
