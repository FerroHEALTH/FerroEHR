// @generated-from-template templates/openehr-base/foundation_types/interval/proper_interval_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM/BASE class invariants for `Proper_interval`.
//!
//! The three enforced checks are the inherited `Interval` invariants
//! `Lower_included_valid`, `Upper_included_valid` and `Limits_consistent` of
//! `docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.foundation_types.interval.adoc`
//! §Invariants, reported under `INTERVAL` — the class that declares them.
//! `DV_INTERVAL` carries its own redefinition (see `openehr-rm`
//! `dv_interval_impl`).
//!
//! Two declared invariants are deliberately not enforced here:
//! - `Proper_interval.Inv_not_point` (`lower /= upper`,
//!   `…foundation_types.proper_interval.adoc`) is adjudicated AGAINST, because
//!   BASE itself relies on bounds-equal proper intervals:
//!   `Multiplicity_interval` inherits `Proper_interval` yet defines
//!   `is_mandatory()` as `{1..1}` and `is_prohibited()` as `{0..0}`
//!   (`…foundation_types.multiplicity_interval.adoc`), so the invariant cannot
//!   be read as forbidding them.
//! - `Interval.Limits_comparable` (`lower.strictly_comparable_to (upper)`) names
//!   a function the released BASE text declares on no class — neither `Ordered`
//!   nor `Any` carries `strictly_comparable_to` — so this layer has no
//!   definition to check. Its RM counterpart is decidable and IS checked, where
//!   `DV_INTERVAL.Limits_consistent` folds
//!   `DV_ORDERED.is_strictly_comparable_to` into itself.

use super::proper_interval::ProperIntervalData;
use crate::validate::{InvariantViolation, Validate};

// NOTE: `interval.adoc` §Invariants states `Limits_consistent` as `lower <=
// upper`, which needs an ordering on `T`; BASE interval bounds are ordered
// foundation types, so the bound is `PartialOrd`.
impl<T: PartialOrd> Validate for ProperIntervalData<T> {
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

    fn interval(lower: Option<i32>, upper: Option<i32>) -> ProperIntervalData<i32> {
        ProperIntervalData {
            lower,
            upper,
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        }
    }

    #[test]
    fn valid_interval_has_no_violations() {
        assert!(interval(Some(1), Some(3)).invariants().is_empty());
        assert!(interval(Some(3), Some(3)).invariants().is_empty());
        let mut open = interval(None, Some(3));
        open.lower_unbounded = true;
        open.lower_included = false;
        assert!(open.invariants().is_empty());
    }

    #[test]
    fn limits_inconsistent() {
        let v = interval(Some(4), Some(3)).invariants();
        assert_eq!(v.len(), 1);
        assert_eq!(
            v[0].message,
            "Invariant Limits_consistent failed on type INTERVAL"
        );
    }

    #[test]
    fn lower_included_invalid() {
        let mut i = interval(Some(1), Some(4));
        i.lower_unbounded = true; // still lower_included == true
        let v = i.invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Lower_included_valid failed on type INTERVAL"),
            "got {v:?}"
        );
    }

    #[test]
    fn upper_included_invalid() {
        let mut i = interval(Some(1), Some(4));
        i.upper_unbounded = true; // still upper_included == true
        let v = i.invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Upper_included_valid failed on type INTERVAL"),
            "got {v:?}"
        );
    }
}
