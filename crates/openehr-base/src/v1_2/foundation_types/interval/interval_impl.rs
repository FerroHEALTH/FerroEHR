// @generated-from-template templates/openehr-base/foundation_types/interval/interval_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written BASE `Interval<T>` constraint-evaluation primitives.
//!
//! Implements the spec functions of the abstract `Interval<T>` class —
//! `has`, `intersects`, `contains`, `is_equal` — plus boundary accessors, on
//! the generated enum `Interval<T>` and on both concrete variants
//! (`Point_interval<T>` → [`PointInterval`], `Proper_interval<T>` →
//! [`ProperIntervalData`]). These are the primitives AM constraint evaluation
//! interrogates when checking a value against an interval constraint.
//!
//! Spec sources (vendored):
//! - `BASE/docs/UML/classes/org.openehr.base.foundation_types.interval.adoc`
//!   (the `Interval<T>` class: attributes, `has`/`intersects`/`contains`/
//!   `is_equal`, invariants).
//! - `BASE/docs/foundation_types/master05-interval.adoc` (prose: open/closed
//!   boundaries, unbounded = ±infinity).
//!
//! The truth tables are derived from the `has` post-condition and the
//! open/closed-boundary prose; `intersects`/`contains` follow the standard
//! boundary-aware interval algebra consistent with `has` (see the NOTEs).

use std::cmp::Ordering;

use super::interval::Interval;
use super::point_interval::PointInterval;
use super::proper_interval::{ProperInterval, ProperIntervalData};

// NOTE: BASE intervals range over an ordered foundation type, so the algebra is
// bounded on `T: PartialOrd`; RM `DV_INTERVAL<T: DV_ORDERED>` ordering is the
// separate `openehr_magnitude` concern.

/// A read-only view of the six `Interval<T>` boundary components, so the spec
/// algebra (`has`/`intersects`/`contains`) is written once and reused by every
/// concrete interval type and the `Interval<T>` enum.
///
/// A boundary is treated as ±infinity when its `*_unbounded` flag is set **or**
/// its value is absent: `Interval.lower`/`upper` are `0..1`
/// (`BASE docs/UML/classes/org.openehr.base.foundation_types.interval.adoc`
/// §Attributes) and no invariant ties absence to the flag, so an absent limit
/// has no value to compare against and can only constrain nothing.
///
/// NOTE: no openEHR spec states the absent-limit reading — our own design.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BoundaryView<'a, T> {
    pub lower: Option<&'a T>,
    pub upper: Option<&'a T>,
    pub lower_unbounded: bool,
    pub upper_unbounded: bool,
    pub lower_included: bool,
    pub upper_included: bool,
}

impl<T: PartialOrd> BoundaryView<'_, T> {
    /// `has` per the `Interval` post-condition
    /// (`org.openehr.base.foundation_types.interval.adoc`, `has`): a value is
    /// contained unless it falls below the lower limit or above the upper
    /// limit, with an excluded (non-`included`) boundary compared strictly and
    /// an unbounded/absent limit imposing no constraint on that side.
    pub(crate) fn has(&self, e: &T) -> bool {
        if !self.lower_unbounded
            && let Some(l) = self.lower
        {
            match e.partial_cmp(l) {
                Some(Ordering::Less) | None => return false,
                Some(Ordering::Equal) if !self.lower_included => return false,
                _ => {}
            }
        }
        if !self.upper_unbounded
            && let Some(u) = self.upper
        {
            match e.partial_cmp(u) {
                Some(Ordering::Greater) | None => return false,
                Some(Ordering::Equal) if !self.upper_included => return false,
                _ => {}
            }
        }
        true
    }

    /// Three-valued containment for a partially-ordered value space:
    /// `Some(true)` when `e` is definitely inside (both sides decidably
    /// satisfied), `Some(false)` when `e` is definitely outside (one side
    /// decidably violated), and `None` when containment is undecidable — a
    /// comparison against a bound returned `None` and no side was decidably
    /// violated.
    ///
    /// This is the honest-incomparability companion to [`BoundaryView::has`]
    /// (which collapses `None` comparisons to "not contained"): for a total
    /// order it agrees with `has` (`has(e) == (has_definite(e) == Some(true))`),
    /// but for a partial order (the `Iso8601_*` value spaces) it distinguishes
    /// "provably outside" from "cannot tell", so a caller can decline to act on
    /// an undecidable answer instead of treating it as exclusion.
    pub(crate) fn has_definite(&self, e: &T) -> Option<bool> {
        let lower_ok: Option<bool> = if self.lower_unbounded {
            Some(true)
        } else if let Some(l) = self.lower {
            match e.partial_cmp(l) {
                None => None,
                Some(Ordering::Less) => Some(false),
                Some(Ordering::Equal) => Some(self.lower_included),
                Some(Ordering::Greater) => Some(true),
            }
        } else {
            Some(true)
        };
        let upper_ok: Option<bool> = if self.upper_unbounded {
            Some(true)
        } else if let Some(u) = self.upper {
            match e.partial_cmp(u) {
                None => None,
                Some(Ordering::Greater) => Some(false),
                Some(Ordering::Equal) => Some(self.upper_included),
                Some(Ordering::Less) => Some(true),
            }
        } else {
            Some(true)
        };
        match (lower_ok, upper_ok) {
            // A decidably-violated side proves exclusion regardless of the other.
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            // Otherwise an undecidable comparison leaves containment unknown.
            _ => None,
        }
    }
}

/// True if `x` lies entirely below `y`, i.e. `x`'s upper limit is below `y`'s
/// lower limit with no shared point. Touching limits (equal values) count as
/// separated only if at least one of the touching boundaries is excluded.
fn strictly_below<T: PartialOrd>(x: &BoundaryView<T>, y: &BoundaryView<T>) -> bool {
    if x.upper_unbounded || y.lower_unbounded {
        return false;
    }
    let (Some(xu), Some(yl)) = (x.upper, y.lower) else {
        // A missing limit behaves as ±infinity: no separation on this side.
        return false;
    };
    match xu.partial_cmp(yl) {
        Some(Ordering::Less) => true,
        Some(Ordering::Equal) => !(x.upper_included && y.lower_included),
        _ => false,
    }
}

/// True if `outer`'s lower limit is at or below `inner`'s lower limit and
/// covers it (an equal, shared lower point is covered only if `outer` includes
/// it whenever `inner` does).
fn lower_covers<T: PartialOrd>(outer: &BoundaryView<T>, inner: &BoundaryView<T>) -> bool {
    if outer.lower_unbounded {
        return true;
    }
    if inner.lower_unbounded {
        return false;
    }
    match (outer.lower, inner.lower) {
        (None, _) => true,        // outer lower absent ⇒ -infinity ⇒ covers
        (Some(_), None) => false, // inner extends to -infinity, outer does not
        (Some(ol), Some(il)) => match ol.partial_cmp(il) {
            Some(Ordering::Less) => true,
            Some(Ordering::Greater) | None => false,
            Some(Ordering::Equal) => outer.lower_included || !inner.lower_included,
        },
    }
}

/// True if `outer`'s upper limit is at or above `inner`'s upper limit and
/// covers it (mirror of [`lower_covers`]).
fn upper_covers<T: PartialOrd>(outer: &BoundaryView<T>, inner: &BoundaryView<T>) -> bool {
    if outer.upper_unbounded {
        return true;
    }
    if inner.upper_unbounded {
        return false;
    }
    match (outer.upper, inner.upper) {
        (None, _) => true,        // outer upper absent ⇒ +infinity ⇒ covers
        (Some(_), None) => false, // inner extends to +infinity, outer does not
        (Some(ou), Some(iu)) => match ou.partial_cmp(iu) {
            Some(Ordering::Greater) => true,
            Some(Ordering::Less) | None => false,
            Some(Ordering::Equal) => outer.upper_included || !inner.upper_included,
        },
    }
}

impl<T: PartialOrd> BoundaryView<'_, T> {
    /// `intersects` (`org.openehr.base.foundation_types.interval.adoc`,
    /// `intersects`): true if there is any overlap between the two intervals.
    ///
    /// NOTE: the spec's elaborating sentence ("at least one limit of
    /// `other` is strictly inside the limits of this interval") is informal and
    /// fails for equal intervals (whose limits coincide rather than lie strictly
    /// inside); the operative definition is "any overlap". We implement the
    /// standard boundary-aware overlap consistent with `has`: two intervals
    /// overlap unless one lies entirely below the other.
    pub(crate) fn intersects(&self, other: &BoundaryView<T>) -> bool {
        !strictly_below(self, other) && !strictly_below(other, self)
    }

    /// `contains` (`org.openehr.base.foundation_types.interval.adoc`,
    /// `contains`): true if every point of `inner` (`other`) lies inside `self`.
    ///
    /// NOTE: the spec heading says "properly contains" but the operative
    /// definition is "all points of `_other_` are inside the current interval",
    /// which is reflexive (an interval contains itself). We implement the
    /// operative (reflexive) definition, not strict/proper subset containment.
    pub(crate) fn contains(&self, other: &BoundaryView<T>) -> bool {
        lower_covers(self, other) && upper_covers(self, other)
    }
}

// ── Point_interval<T> ───────────────────────────────────────────────────────

impl<T> PointInterval<T> {
    /// Boundary view over this point interval's six `Interval` components.
    pub(crate) fn boundary_view(&self) -> BoundaryView<'_, T> {
        BoundaryView {
            lower: self.lower.as_ref(),
            upper: self.upper.as_ref(),
            lower_unbounded: self.lower_unbounded,
            upper_unbounded: self.upper_unbounded,
            lower_included: self.lower_included,
            upper_included: self.upper_included,
        }
    }
}

impl<T: PartialOrd> PointInterval<T> {
    /// `Interval.has` for a point interval
    /// (`org.openehr.base.foundation_types.interval.adoc`).
    #[must_use]
    pub fn has(&self, e: &T) -> bool {
        self.boundary_view().has(e)
    }

    /// `Interval.intersects` for a point interval.
    #[must_use]
    pub fn intersects(&self, other: &Interval<T>) -> bool {
        self.boundary_view().intersects(&other.boundary_view())
    }

    /// `Interval.contains` for a point interval.
    #[must_use]
    pub fn contains(&self, other: &Interval<T>) -> bool {
        self.boundary_view().contains(&other.boundary_view())
    }
}

// ── Proper_interval<T> (data variant) ────────────────────────────────────────

impl<T> ProperIntervalData<T> {
    /// Boundary view over this proper interval's six `Interval` components.
    pub(crate) fn boundary_view(&self) -> BoundaryView<'_, T> {
        BoundaryView {
            lower: self.lower.as_ref(),
            upper: self.upper.as_ref(),
            lower_unbounded: self.lower_unbounded,
            upper_unbounded: self.upper_unbounded,
            lower_included: self.lower_included,
            upper_included: self.upper_included,
        }
    }
}

impl<T: PartialOrd> ProperIntervalData<T> {
    /// `Interval.has` for a proper interval
    /// (`org.openehr.base.foundation_types.interval.adoc`).
    #[must_use]
    pub fn has(&self, e: &T) -> bool {
        self.boundary_view().has(e)
    }

    /// `Interval.intersects` for a proper interval.
    #[must_use]
    pub fn intersects(&self, other: &Interval<T>) -> bool {
        self.boundary_view().intersects(&other.boundary_view())
    }

    /// `Interval.contains` for a proper interval.
    #[must_use]
    pub fn contains(&self, other: &Interval<T>) -> bool {
        self.boundary_view().contains(&other.boundary_view())
    }
}

// ── Interval<T> (the closed subtype enum) ────────────────────────────────────

impl<T> Interval<T> {
    /// Lower bound value, if any.
    ///
    /// NOTE: the generated `Proper_interval<T>` closed-subtype enum
    /// includes `Multiplicity_interval` (an `Interval<Integer>`) as a variant
    /// for every `T`, because the BMM closed-subtype expansion is type-erased.
    /// A real clinical `Interval<T>` value is never that variant; for it the
    /// `T`-typed accessors return `None` (its bounds are `Integer`, not `T`).
    /// Use [`MultiplicityInterval`](super::multiplicity_interval::MultiplicityInterval)
    /// directly for multiplicity intervals.
    #[must_use]
    pub fn lower(&self) -> Option<&T> {
        match self {
            Interval::PointInterval(p) => p.lower.as_ref(),
            Interval::ProperInterval(ProperInterval::ProperInterval(pi)) => pi.lower.as_ref(),
            Interval::ProperInterval(ProperInterval::MultiplicityInterval(_)) => None,
        }
    }

    /// Upper bound value, if any. See [`Interval::lower`] for the
    /// `Multiplicity_interval`-variant caveat.
    #[must_use]
    pub fn upper(&self) -> Option<&T> {
        match self {
            Interval::PointInterval(p) => p.upper.as_ref(),
            Interval::ProperInterval(ProperInterval::ProperInterval(pi)) => pi.upper.as_ref(),
            Interval::ProperInterval(ProperInterval::MultiplicityInterval(_)) => None,
        }
    }

    /// True if the lower boundary is open (= -infinity).
    #[must_use]
    pub fn lower_unbounded(&self) -> bool {
        match self {
            Interval::PointInterval(p) => p.lower_unbounded,
            Interval::ProperInterval(ProperInterval::ProperInterval(pi)) => pi.lower_unbounded,
            Interval::ProperInterval(ProperInterval::MultiplicityInterval(m)) => m.lower_unbounded,
        }
    }

    /// True if the upper boundary is open (= +infinity).
    #[must_use]
    pub fn upper_unbounded(&self) -> bool {
        match self {
            Interval::PointInterval(p) => p.upper_unbounded,
            Interval::ProperInterval(ProperInterval::ProperInterval(pi)) => pi.upper_unbounded,
            Interval::ProperInterval(ProperInterval::MultiplicityInterval(m)) => m.upper_unbounded,
        }
    }

    /// True if the lower boundary value is included in the range.
    #[must_use]
    pub fn lower_included(&self) -> bool {
        match self {
            Interval::PointInterval(p) => p.lower_included,
            Interval::ProperInterval(ProperInterval::ProperInterval(pi)) => pi.lower_included,
            Interval::ProperInterval(ProperInterval::MultiplicityInterval(m)) => m.lower_included,
        }
    }

    /// True if the upper boundary value is included in the range.
    #[must_use]
    pub fn upper_included(&self) -> bool {
        match self {
            Interval::PointInterval(p) => p.upper_included,
            Interval::ProperInterval(ProperInterval::ProperInterval(pi)) => pi.upper_included,
            Interval::ProperInterval(ProperInterval::MultiplicityInterval(m)) => m.upper_included,
        }
    }

    /// Boundary view over this interval's six `Interval` components. The
    /// `Multiplicity_interval` variant (see [`Interval::lower`]) contributes
    /// only its boolean flags; its `Integer` bounds surface as absent.
    fn boundary_view(&self) -> BoundaryView<'_, T> {
        BoundaryView {
            lower: self.lower(),
            upper: self.upper(),
            lower_unbounded: self.lower_unbounded(),
            upper_unbounded: self.upper_unbounded(),
            lower_included: self.lower_included(),
            upper_included: self.upper_included(),
        }
    }
}

impl<T: PartialEq> Interval<T> {
    /// `is_equal` (`org.openehr.base.foundation_types.interval.adoc`,
    /// `is_equal`): true if this interval is semantically the same as `other`.
    /// The generated types derive structural equality, which is exactly this.
    #[must_use]
    pub fn is_equal(&self, other: &Interval<T>) -> bool {
        self == other
    }
}

impl<T: PartialOrd> Interval<T> {
    /// `Interval.has` (`org.openehr.base.foundation_types.interval.adoc`).
    ///
    /// NOTE: the `Multiplicity_interval` variant (an `Interval<Integer>`,
    /// see [`Interval::lower`]) cannot be evaluated against a `T` value and
    /// returns `false`; it is unreachable for a genuine clinical `Interval<T>`.
    #[must_use]
    pub fn has(&self, e: &T) -> bool {
        match self {
            Interval::PointInterval(p) => p.has(e),
            Interval::ProperInterval(ProperInterval::ProperInterval(pi)) => pi.has(e),
            Interval::ProperInterval(ProperInterval::MultiplicityInterval(_)) => false,
        }
    }

    /// Three-valued containment (`BoundaryView::has_definite`): `Some(true)`
    /// definitely inside, `Some(false)` definitely outside, `None` undecidable
    /// (a comparison against a bound was incomparable). Use this instead of
    /// [`Interval::has`] over a partially-ordered value space (the `Iso8601_*`
    /// types) where an undecidable comparison must not be treated as exclusion.
    ///
    /// NOTE: the `Multiplicity_interval` variant (an `Interval<Integer>`, see
    /// [`Interval::lower`]) cannot be evaluated against a `T` value and returns
    /// `None`; it is unreachable for a genuine clinical `Interval<T>`.
    #[must_use]
    pub fn has_definite(&self, e: &T) -> Option<bool> {
        match self {
            Interval::PointInterval(_)
            | Interval::ProperInterval(ProperInterval::ProperInterval(_)) => {
                self.boundary_view().has_definite(e)
            }
            Interval::ProperInterval(ProperInterval::MultiplicityInterval(_)) => None,
        }
    }

    /// `Interval.intersects` (`org.openehr.base.foundation_types.interval.adoc`).
    #[must_use]
    pub fn intersects(&self, other: &Interval<T>) -> bool {
        self.boundary_view().intersects(&other.boundary_view())
    }

    /// `Interval.contains` (`org.openehr.base.foundation_types.interval.adoc`).
    #[must_use]
    pub fn contains(&self, other: &Interval<T>) -> bool {
        self.boundary_view().contains(&other.boundary_view())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a two-sided `Proper_interval<i32>` with explicit inclusion flags.
    fn proper(lower: i32, li: bool, upper: i32, ui: bool) -> ProperIntervalData<i32> {
        ProperIntervalData {
            lower: Some(lower),
            upper: Some(upper),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: li,
            upper_included: ui,
        }
    }

    fn proper_iv(lower: i32, li: bool, upper: i32, ui: bool) -> Interval<i32> {
        Interval::ProperInterval(ProperInterval::ProperInterval(proper(lower, li, upper, ui)))
    }

    fn point(v: i32) -> PointInterval<i32> {
        PointInterval {
            lower: Some(v),
            upper: Some(v),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        }
    }

    // ── Interval.has: boundary inclusion / exclusion ──────────────────────────

    #[test]
    fn has_closed_boundaries_include_endpoints() {
        let iv = proper(1, true, 5, true); // [1, 5]
        assert!(iv.has(&1));
        assert!(iv.has(&3));
        assert!(iv.has(&5));
        assert!(!iv.has(&0));
        assert!(!iv.has(&6));
    }

    #[test]
    fn has_open_boundaries_exclude_endpoints() {
        let iv = proper(1, false, 5, false); // (1, 5)
        assert!(!iv.has(&1));
        assert!(iv.has(&2));
        assert!(!iv.has(&5));
    }

    #[test]
    fn has_definite_is_three_valued_over_a_partial_order() {
        // f64 is a partial order (NaN is incomparable), so has_definite must
        // distinguish "provably outside" from "cannot tell".
        let iv = Interval::ProperInterval(ProperInterval::ProperInterval(ProperIntervalData {
            lower: Some(1.0_f64),
            upper: Some(5.0_f64),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        }));
        assert_eq!(iv.has_definite(&3.0), Some(true)); // definitely inside
        assert_eq!(iv.has_definite(&0.0), Some(false)); // definitely below
        assert_eq!(iv.has_definite(&9.0), Some(false)); // definitely above
        assert_eq!(iv.has_definite(&f64::NAN), None); // incomparable ⇒ undecidable
    }

    #[test]
    fn has_half_open_boundaries() {
        let lo_open = proper(1, false, 5, true); // (1, 5]
        assert!(!lo_open.has(&1));
        assert!(lo_open.has(&5));
        let hi_open = proper(1, true, 5, false); // [1, 5)
        assert!(hi_open.has(&1));
        assert!(!hi_open.has(&5));
    }

    #[test]
    fn has_unbounded_sides_are_infinite() {
        // (-inf, 5]
        let iv = ProperIntervalData {
            lower: None,
            upper: Some(5),
            lower_unbounded: true,
            upper_unbounded: false,
            lower_included: false,
            upper_included: true,
        };
        assert!(iv.has(&-1000));
        assert!(iv.has(&5));
        assert!(!iv.has(&6));
    }

    #[test]
    fn has_point_interval_matches_only_the_point() {
        let p = point(7);
        assert!(p.has(&7));
        assert!(!p.has(&6));
        assert!(!p.has(&8));
    }

    // ── Interval.intersects ───────────────────────────────────────────────────

    #[test]
    fn intersects_overlapping_and_disjoint() {
        let a = proper_iv(1, true, 5, true); // [1, 5]
        let b = proper_iv(4, true, 9, true); // [4, 9]
        let c = proper_iv(6, true, 9, true); // [6, 9]
        assert!(a.intersects(&b), "[1,5] overlaps [4,9]");
        assert!(!a.intersects(&c), "[1,5] disjoint from [6,9]");
    }

    #[test]
    fn intersects_touching_boundary_closed_vs_open() {
        // [1,5] and [5,9]: touch at 5, both closed ⇒ overlap.
        let closed = proper_iv(1, true, 5, true);
        let closed2 = proper_iv(5, true, 9, true);
        assert!(closed.intersects(&closed2));
        // [1,5) and [5,9]: touch at 5, left excludes it ⇒ no overlap.
        let hi_open = proper_iv(1, true, 5, false);
        assert!(!hi_open.intersects(&closed2));
    }

    #[test]
    fn intersects_is_symmetric_and_reflexive() {
        let a = proper_iv(1, true, 5, true);
        let b = proper_iv(4, true, 9, true);
        assert!(a.intersects(&b) && b.intersects(&a));
        assert!(a.intersects(&a));
    }

    // ── Interval.contains ─────────────────────────────────────────────────────

    #[test]
    fn contains_proper_subinterval() {
        let outer = proper_iv(1, true, 10, true); // [1, 10]
        let inner = proper_iv(3, true, 7, true); // [3, 7]
        assert!(outer.contains(&inner));
        assert!(!inner.contains(&outer));
    }

    #[test]
    fn contains_is_reflexive() {
        let a = proper_iv(1, true, 5, true);
        assert!(a.contains(&a), "operative definition is reflexive");
    }

    #[test]
    fn contains_respects_boundary_inclusion() {
        // [1,5) does not contain [1,5] (5 ∈ inner but ∉ outer).
        let hi_open = proper_iv(1, true, 5, false);
        let closed = proper_iv(1, true, 5, true);
        assert!(!hi_open.contains(&closed));
        // [1,5] contains [1,5) (outer includes everything inner does).
        assert!(closed.contains(&hi_open));
    }

    #[test]
    fn contains_unbounded_outer_contains_everything() {
        // (-inf, +inf)
        let universe =
            Interval::ProperInterval(ProperInterval::ProperInterval(ProperIntervalData {
                lower: None,
                upper: None,
                lower_unbounded: true,
                upper_unbounded: true,
                lower_included: false,
                upper_included: false,
            }));
        let inner = proper_iv(3, true, 7, true);
        assert!(universe.contains(&inner));
        assert!(!inner.contains(&universe));
    }

    #[test]
    fn point_interval_contained_in_range() {
        let range = proper_iv(1, true, 10, true);
        let p = Interval::PointInterval(point(5));
        assert!(range.contains(&p));
        assert!(p.intersects(&range));
    }

    // ── boundary accessors + is_equal ─────────────────────────────────────────

    #[test]
    fn accessors_report_generated_fields() {
        let iv = proper_iv(1, false, 5, true);
        assert_eq!(iv.lower(), Some(&1));
        assert_eq!(iv.upper(), Some(&5));
        assert!(!iv.lower_unbounded());
        assert!(!iv.upper_unbounded());
        assert!(!iv.lower_included());
        assert!(iv.upper_included());
    }

    #[test]
    fn is_equal_matches_structural_equality() {
        let a = proper_iv(1, true, 5, true);
        let b = proper_iv(1, true, 5, true);
        let c = proper_iv(1, true, 6, true);
        assert!(a.is_equal(&b));
        assert!(!a.is_equal(&c));
    }
}
