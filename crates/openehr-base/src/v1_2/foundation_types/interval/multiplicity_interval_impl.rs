// @generated-from-template templates/openehr-base/foundation_types/interval/multiplicity_interval_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written BASE `Multiplicity_interval` spec functions + invariants.
//!
//! `Multiplicity_interval` is an `Interval<Integer>` used to express
//! multiplicity, cardinality and optionality in models (it is the interval the
//! AM occurrence/existence/cardinality validator interrogates). It inherits the
//! `Interval` boundary algebra (`has`/`intersects`/`contains`, reused here via
//! the shared `BoundaryView`) and adds the classification predicates
//! `is_open`/`is_optional`/`is_mandatory`/`is_prohibited`.
//!
//! Spec sources (vendored):
//! - `docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.foundation_types.multiplicity_interval.adoc`
//!   §Functions (the four classification functions).
//! - `docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.foundation_types.interval.adoc`
//!   §Functions + §Invariants (inherited `has`/`intersects`/`contains`).

use super::interval_impl::BoundaryView;
use super::multiplicity_interval::MultiplicityInterval;
use crate::validate::{InvariantViolation, Validate};

impl MultiplicityInterval {
    /// Boundary view over this multiplicity interval's six `Interval`
    /// components (bounds are `Integer`).
    fn boundary_view(&self) -> BoundaryView<'_, i32> {
        BoundaryView {
            lower: self.lower.as_ref(),
            upper: self.upper.as_ref(),
            lower_unbounded: self.lower_unbounded,
            upper_unbounded: self.upper_unbounded,
            lower_included: self.lower_included,
            upper_included: self.upper_included,
        }
    }

    /// Inherited `Interval.has` for the `Integer` bounds
    /// (`org.openehr.base.foundation_types.interval.adoc`, `has`).
    #[must_use]
    pub fn has(&self, e: i32) -> bool {
        self.boundary_view().has(&e)
    }

    /// Inherited `Interval.intersects`
    /// (`org.openehr.base.foundation_types.interval.adoc`, `intersects`).
    #[must_use]
    pub fn intersects(&self, other: &MultiplicityInterval) -> bool {
        self.boundary_view().intersects(&other.boundary_view())
    }

    /// Inherited `Interval.contains`
    /// (`org.openehr.base.foundation_types.interval.adoc`, `contains`).
    #[must_use]
    pub fn contains(&self, other: &MultiplicityInterval) -> bool {
        self.boundary_view().contains(&other.boundary_view())
    }

    /// `is_open` (`multiplicity_interval.adoc`): true if this interval imposes
    /// no constraints, i.e. is set to `0..*` (lower 0, upper unbounded).
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.lower == Some(0) && self.upper_unbounded
    }

    /// `is_optional` (`multiplicity_interval.adoc`): true if this interval
    /// expresses optionality, i.e. `0..1`.
    #[must_use]
    pub fn is_optional(&self) -> bool {
        self.lower == Some(0) && !self.upper_unbounded && self.upper == Some(1)
    }

    /// `is_mandatory` (`multiplicity_interval.adoc`): true if this interval
    /// expresses mandation, i.e. `1..1`.
    #[must_use]
    pub fn is_mandatory(&self) -> bool {
        self.lower == Some(1) && !self.upper_unbounded && self.upper == Some(1)
    }

    /// `is_prohibited` (`multiplicity_interval.adoc`): true if this interval is
    /// set to `0..0`.
    #[must_use]
    pub fn is_prohibited(&self) -> bool {
        self.lower == Some(0) && !self.upper_unbounded && self.upper == Some(0)
    }
}

// The inherited `Interval` invariants (`interval.adoc` §Invariants), reported
// under `INTERVAL` — the class that declares them. `Inv_not_point` and
// `Limits_comparable` are not enforced; see `proper_interval_impl.rs` for the
// two adjudications, the first of which turns on this very class.
impl Validate for MultiplicityInterval {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        // Lower_included_valid: an unbounded lower boundary is not included.
        if self.lower_unbounded && self.lower_included {
            out.push(InvariantViolation::here(
                "Invariant Lower_included_valid failed on type INTERVAL",
            ));
        }
        // Upper_included_valid: an unbounded upper boundary is not included.
        if self.upper_unbounded && self.upper_included {
            out.push(InvariantViolation::here(
                "Invariant Upper_included_valid failed on type INTERVAL",
            ));
        }
        // Limits_consistent: with both boundaries bounded and present, lower <= upper.
        if !self.lower_unbounded
            && !self.upper_unbounded
            && let (Some(l), Some(u)) = (self.lower, self.upper)
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

    /// `n..m` (both bounded, both included) — the common multiplicity shape.
    fn range(lower: i32, upper: i32) -> MultiplicityInterval {
        MultiplicityInterval {
            lower: Some(lower),
            upper: Some(upper),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        }
    }

    /// `lower..*` (upper unbounded).
    fn from(lower: i32) -> MultiplicityInterval {
        MultiplicityInterval {
            lower: Some(lower),
            upper: None,
            lower_unbounded: false,
            upper_unbounded: true,
            lower_included: true,
            upper_included: false,
        }
    }

    #[test]
    fn is_open_only_for_zero_star() {
        assert!(from(0).is_open()); // 0..*
        assert!(!from(1).is_open()); // 1..*
        assert!(!range(0, 1).is_open()); // 0..1
    }

    #[test]
    fn is_optional_only_for_zero_one() {
        assert!(range(0, 1).is_optional()); // 0..1
        assert!(!range(1, 1).is_optional());
        assert!(!range(0, 0).is_optional());
        assert!(!from(0).is_optional());
    }

    #[test]
    fn is_mandatory_only_for_one_one() {
        assert!(range(1, 1).is_mandatory()); // 1..1
        assert!(!range(0, 1).is_mandatory());
        assert!(!range(1, 2).is_mandatory());
    }

    #[test]
    fn is_prohibited_only_for_zero_zero() {
        assert!(range(0, 0).is_prohibited()); // 0..0
        assert!(!range(0, 1).is_prohibited());
        assert!(!range(1, 1).is_prohibited());
    }

    #[test]
    fn has_covers_the_integer_range() {
        let m = range(1, 3); // [1, 3]
        assert!(m.has(1) && m.has(2) && m.has(3));
        assert!(!m.has(0) && !m.has(4));
        // 1..* accepts any occurrence count >= 1.
        let open = from(1);
        assert!(open.has(1) && open.has(1000));
        assert!(!open.has(0));
    }

    #[test]
    fn prohibited_has_only_zero() {
        let p = range(0, 0); // 0..0
        assert!(p.has(0));
        assert!(!p.has(1));
    }

    #[test]
    fn intersects_and_contains_on_integer_ranges() {
        assert!(range(0, 5).intersects(&range(3, 9)));
        assert!(!range(0, 2).intersects(&range(3, 9)));
        assert!(range(0, 10).contains(&range(2, 4)));
        assert!(!range(2, 4).contains(&range(0, 10)));
        // 0..* contains any bounded multiplicity.
        assert!(from(0).contains(&range(3, 7)));
    }

    #[test]
    fn invariant_limits_consistent_flags_reversed_bounds() {
        let bad = range(3, 1); // 3..1
        let v = bad.invariants();
        assert_eq!(v.len(), 1);
        assert_eq!(
            v[0].message,
            "Invariant Limits_consistent failed on type INTERVAL"
        );
    }

    #[test]
    fn invariant_included_flags_valid_for_normal_ranges() {
        assert!(range(0, 1).invariants().is_empty());
        assert!(from(0).invariants().is_empty());
    }

    #[test]
    fn invariant_lower_included_valid_flags_unbounded_included_lower() {
        let mut bad = range(0, 5);
        bad.lower_unbounded = true; // still lower_included == true
        assert!(
            bad.invariants()
                .iter()
                .any(|m| m.message == "Invariant Lower_included_valid failed on type INTERVAL"),
        );
    }
}
