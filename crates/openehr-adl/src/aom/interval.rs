// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Interval and multiplicity arithmetic shared by the AOM2 validity rules, the
//! flattener, and the cADL domain lowering.
//!
//! Two interval families meet here: `MULTIPLICITY_INTERVAL` (existence,
//! occurrences, cardinality —
//! `docs/specs/openehr/BASE/docs/foundation_types/master05-interval.adoc`
//! plus `docs/specs/openehr/AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §`C_ATTRIBUTE`), read through the [`Bounds`] view; and the generic
//! `INTERVAL<T>` a primitive constraint carries.
//!
//! One family of near-duplicate readings is deliberately kept side by side
//! rather than merged, because its callers depend on the difference: the bounds
//! renderers (`display_bounds` vs `display_bounds_always_range`). Their doc
//! comments state the divergence.

use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_base::prelude::{Interval, MultiplicityInterval, ProperInterval};

/// A multiplicity bound, extracted from an RM attribute or a cADL constraint.
/// `upper == None` denotes an unbounded (∞) upper limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    /// Inclusive lower bound.
    pub lower: i32,
    /// Inclusive upper bound; `None` = unbounded.
    pub upper: Option<i32>,
}

impl Bounds {
    /// A closed `{lower..upper}` bound.
    #[must_use]
    pub fn new(lower: i32, upper: Option<i32>) -> Self {
        Self { lower, upper }
    }

    /// The [`MultiplicityInterval`] this bound denotes, the inverse of the
    /// crate-internal `bounds` reader.
    #[must_use]
    pub fn to_multiplicity_interval(self) -> MultiplicityInterval {
        MultiplicityInterval {
            lower: Some(self.lower),
            upper: self.upper,
            lower_unbounded: false,
            upper_unbounded: self.upper.is_none(),
            lower_included: true,
            upper_included: self.upper.is_some(),
        }
    }

    /// True if `inner` is the same as, or narrower than (wholly contained
    /// within), `self` — the "conform, i.e. be the same or narrower" test the
    /// existence (VCAEX) and cardinality (VCACA) rules require (`master04.5`
    /// §Validity Rules: `C_ATTRIBUTE`).
    #[must_use]
    pub fn contains(self, inner: Bounds) -> bool {
        inner.lower >= self.lower
            && match (self.upper, inner.upper) {
                // `self` unbounded above ⇒ any inner upper is within it.
                (None, _) => true,
                // `self` bounded but `inner` unbounded ⇒ inner escapes above.
                (Some(_), None) => false,
                (Some(outer), Some(i)) => i <= outer,
            }
    }
}

/// [`Bounds`] view of a [`MultiplicityInterval`] (existence / occurrences /
/// cardinality bound), with `upper == None` denoting an unbounded (`*`) limit.
#[must_use]
pub(crate) fn bounds(mi: &MultiplicityInterval) -> Bounds {
    Bounds {
        lower: if mi.lower_unbounded {
            0
        } else {
            mi.lower.unwrap_or(0)
        },
        upper: if mi.upper_unbounded { None } else { mi.upper },
    }
}

/// The finite upper bound of a multiplicity interval, or `None` if unbounded.
pub(crate) fn finite_upper(mi: &MultiplicityInterval) -> Option<i32> {
    if mi.upper_unbounded { None } else { mi.upper }
}

/// The finite cardinality upper bound of an attribute, `None` if unbounded / no
/// cardinality.
pub(crate) fn finite_cardinality_upper(attr: &CAttribute) -> Option<i32> {
    attr.cardinality
        .as_ref()
        .and_then(|c| finite_upper(&c.interval))
}

/// Render [`Bounds`] as cADL multiplicity text, collapsing a degenerate bound:
/// `{1..1}` prints as `{1}`.
///
/// The sibling renderer [`display_bounds_always_range`] does NOT collapse. Both
/// feed user-visible validation-issue messages, so neither may adopt the
/// other's spelling.
pub(crate) fn display_bounds(b: Bounds) -> String {
    match b.upper {
        Some(u) if u == b.lower => format!("{{{}}}", b.lower),
        Some(u) => format!("{{{}..{u}}}", b.lower),
        None => format!("{{{}..*}}", b.lower),
    }
}

/// Render [`Bounds`] as cADL multiplicity text, always as a range: `{1..1}`
/// prints as `{1..1}`.
///
/// The sibling renderer [`display_bounds`] collapses a degenerate bound to
/// `{1}`. Both feed user-visible validation-issue messages, so neither may
/// adopt the other's spelling.
pub(crate) fn display_bounds_always_range(b: Bounds) -> String {
    match b.upper {
        Some(u) => format!("{{{}..{u}}}", b.lower),
        None => format!("{{{}..*}}", b.lower),
    }
}

/// The six `Interval<T>` boundary fields of any variant, read uniformly.
#[derive(Debug, Clone, Copy)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the four flags mirror `Interval<T>`'s own boolean attributes 1:1 (BASE `…foundation_types.interval.adoc` §Interval Class) — collapsing them would restate the spec type"
)]
struct IntervalFields<T> {
    /// Lower bound, if one is stored.
    lower: Option<T>,
    /// Upper bound, if one is stored.
    upper: Option<T>,
    /// True if the lower boundary is open (`-∞`).
    lower_unbounded: bool,
    /// True if the upper boundary is open (`+∞`).
    upper_unbounded: bool,
    /// True if the lower bound is part of the interval.
    lower_included: bool,
    /// True if the upper bound is part of the interval.
    upper_included: bool,
}

impl<T: Copy> IntervalFields<T> {
    /// Read any `Interval<T>` variant's boundary fields.
    ///
    /// The `MULTIPLICITY_INTERVAL` variant (structurally possible on the
    /// generic enum but never produced for a domain leaf constraint) carries
    /// `Integer` bounds rather than `T`, so it answers fully open with both
    /// bounds absent — every reading built on this view then declines to
    /// decide.
    fn read(iv: &Interval<T>) -> Self {
        match iv {
            Interval::PointInterval(p) => Self {
                lower: p.lower,
                upper: p.upper,
                lower_unbounded: p.lower_unbounded,
                upper_unbounded: p.upper_unbounded,
                lower_included: p.lower_included,
                upper_included: p.upper_included,
            },
            Interval::ProperInterval(ProperInterval::ProperInterval(p)) => Self {
                lower: p.lower,
                upper: p.upper,
                lower_unbounded: p.lower_unbounded,
                upper_unbounded: p.upper_unbounded,
                lower_included: p.lower_included,
                upper_included: p.upper_included,
            },
            Interval::ProperInterval(ProperInterval::MultiplicityInterval(_)) => Self {
                lower: None,
                upper: None,
                lower_unbounded: true,
                upper_unbounded: true,
                lower_included: false,
                upper_included: false,
            },
        }
    }

    /// True if both sides are bounded AND both bounds are included — the
    /// precondition for "this interval denotes exactly one value".
    ///
    /// The unbounded flags are load-bearing, not redundant with an absent
    /// bound: `Interval<T>`'s invariant set
    /// (`BASE/docs/UML/classes/org.openehr.base.foundation_types.interval.adoc`
    /// §Interval Class, Invariants) never forbids a stored bound alongside an
    /// unbounded flag, and `has()`'s postcondition short-circuits on the flag —
    /// so an unbounded side dominates any stored bound. `Point_interval` only
    /// *defaults* the flags (`{default = false}` / `{default = true}`,
    /// `…foundation_types.point_interval.adoc` §`Point_interval` Class), it
    /// does not fix them.
    fn is_closed_bounded(self) -> bool {
        !self.lower_unbounded && !self.upper_unbounded && self.lower_included && self.upper_included
    }
}

/// The single integer value an interval denotes, or `None` if it denotes a
/// range, an open side, or nothing decidable.
///
/// The one spec-correct predicate, applied irrespective of the
/// point-vs-proper tagging: both sides bounded, both bounds included, both
/// bounds present and equal.
///
/// - A bounds-equal CLOSED interval IS a single value, whichever subtype
///   carries it: `AOM2/master04.2-constraint_model-semantics.adoc` §Primitive
///   Types (the `Ordered`/`C_ORDERED` row — "A single value (which is a point
///   interval), a list of values (list of point intervals), a list of
///   intervals, which may be mixed proper and point intervals"),
///   `ADL2/master04.5-cadl_primitive_types.adoc` §Constraints on Ordered Types
///   ("a degenerate interval of the form `{N..N}`, i.e. effectively a single
///   value"), and `LANG/docs/expression_language/master03-basics.adoc`, which
///   defines a point as a closed interval whose boundaries are the same.
/// - An unbounded or bound-excluding interval is NOT a single value, even when
///   tagged `POINT_INTERVAL` — see [`IntervalFields::is_closed_bounded`] for
///   why the flags decide.
///
/// NOTE: `Proper_interval`'s `Inv_not_point` (`lower /= upper`, BASE
/// `…foundation_types.proper_interval.adoc`) is adjudicated AGAINST here:
/// BASE's own `Multiplicity_interval` inherits `Proper_interval` yet defines
/// `is_mandatory()` as `{1..1}` (`…multiplicity_interval.adoc`), so the
/// invariant cannot be read as forbidding bounds-equal proper intervals.
pub(crate) fn point_value_i32(iv: &Interval<i32>) -> Option<i32> {
    let f = IntervalFields::read(iv);
    if !f.is_closed_bounded() {
        return None;
    }
    match (f.lower, f.upper) {
        (Some(l), Some(u)) if l == u => Some(l),
        _ => None,
    }
}

/// The single real value an interval denotes, or `None` if it denotes a range,
/// an open side, or nothing decidable.
///
/// The real counterpart of [`point_value_i32`], with the identical predicate
/// and the identical spec grounds.
#[expect(
    clippy::float_cmp,
    reason = "the question is whether the two stored bounds are the SAME value (`{N..N}`); a tolerance would report `|1.0..1.0000001|` as a single value"
)]
pub(crate) fn point_value_f64(iv: &Interval<f64>) -> Option<f64> {
    let f = IntervalFields::read(iv);
    if !f.is_closed_bounded() {
        return None;
    }
    match (f.lower, f.upper) {
        (Some(l), Some(u)) if l == u => Some(l),
        _ => None,
    }
}

/// The `(lower, upper)` bounds of an interval as `f64`, each `None` when open or
/// unbounded, plus the two inclusivity flags. A `MultiplicityInterval` variant
/// (structurally possible on the generic enum but never produced for a domain
/// leaf constraint) yields fully-open bounds via [`IntervalFields::read`], so
/// membership is undecided and the conservative `true` answer stands.
pub(crate) fn interval_bounds_f64<T: Copy + Into<f64>>(
    iv: &Interval<T>,
) -> (Option<f64>, Option<f64>, bool, bool) {
    let f = IntervalFields::read(iv);
    let (lower, upper, lower_unbounded, upper_unbounded, lower_included, upper_included) = (
        f.lower,
        f.upper,
        f.lower_unbounded,
        f.upper_unbounded,
        f.lower_included,
        f.upper_included,
    );
    (
        if lower_unbounded {
            None
        } else {
            lower.map(Into::into)
        },
        if upper_unbounded {
            None
        } else {
            upper.map(Into::into)
        },
        lower_included,
        upper_included,
    )
}

/// True if the real interval `iv` contains `v` (honouring open/closed bounds).
pub(crate) fn real_interval_contains(iv: &Interval<f64>, v: f64) -> bool {
    bounds_admit(v, interval_bounds_f64(iv))
}

/// True if the integer interval `iv` contains `v` (honouring open/closed bounds).
pub(crate) fn int_interval_contains(iv: &Interval<i32>, v: i32) -> bool {
    bounds_admit(f64::from(v), interval_bounds_f64(iv))
}

/// Interval membership over `f64` bounds, shared by the real/integer tests.
pub(crate) fn bounds_admit(v: f64, bounds: (Option<f64>, Option<f64>, bool, bool)) -> bool {
    let (lower, upper, lower_included, upper_included) = bounds;
    if let Some(lo) = lower
        && (v < lo || (!lower_included && (v - lo).abs() < f64::EPSILON))
    {
        return false;
    }
    if let Some(hi) = upper
        && (v > hi || (!upper_included && (v - hi).abs() < f64::EPSILON))
    {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_containment_is_same_or_narrower() {
        let one_to_one = Bounds::new(1, Some(1));
        assert!(one_to_one.contains(Bounds::new(1, Some(1))));
        assert!(!one_to_one.contains(Bounds::new(0, Some(0)))); // {0} not within {1..1}
        let star = Bounds::new(0, None);
        assert!(star.contains(Bounds::new(1, Some(5))));
        assert!(!Bounds::new(1, Some(5)).contains(star)); // {0..*} escapes {1..5}
    }

    /// The two renderers differ ONLY on a degenerate bound, and both spellings
    /// are load-bearing message text.
    #[test]
    fn the_two_bounds_renderers_differ_on_a_degenerate_bound() {
        let point = Bounds::new(1, Some(1));
        assert_eq!(display_bounds(point), "{1}");
        assert_eq!(display_bounds_always_range(point), "{1..1}");
        let range = Bounds::new(0, Some(3));
        assert_eq!(display_bounds(range), "{0..3}");
        assert_eq!(display_bounds_always_range(range), "{0..3}");
        let open = Bounds::new(2, None);
        assert_eq!(display_bounds(open), "{2..*}");
        assert_eq!(display_bounds_always_range(open), "{2..*}");
    }

    /// A `PROPER_INTERVAL<Integer>` with explicit bounds + flags.
    fn proper_i32(lower: Option<i32>, upper: Option<i32>, flags: [bool; 4]) -> Interval<i32> {
        let [
            lower_unbounded,
            upper_unbounded,
            lower_included,
            upper_included,
        ] = flags;
        Interval::ProperInterval(ProperInterval::ProperInterval(
            openehr_base::prelude::ProperIntervalData {
                lower,
                upper,
                lower_unbounded,
                upper_unbounded,
                lower_included,
                upper_included,
            },
        ))
    }

    /// A `POINT_INTERVAL<Integer>` with explicit flags.
    fn point_i32(value: i32, flags: [bool; 4]) -> Interval<i32> {
        let [
            lower_unbounded,
            upper_unbounded,
            lower_included,
            upper_included,
        ] = flags;
        Interval::PointInterval(openehr_base::prelude::PointInterval {
            lower: Some(value),
            upper: Some(value),
            lower_unbounded,
            upper_unbounded,
            lower_included,
            upper_included,
        })
    }

    /// `[lower_unbounded, upper_unbounded, lower_included, upper_included]` for a
    /// closed, fully bounded interval.
    const CLOSED: [bool; 4] = [false, false, true, true];

    /// A closed `{n..n}` PROPER interval denotes a single value
    /// (`ADL2/master04.5-cadl_primitive_types.adoc` §Constraints on Ordered
    /// Types; `AOM2/master04.2` §Primitive Types, `C_ORDERED` row) — the
    /// point/proper tagging does not decide.
    #[test]
    fn a_closed_bounds_equal_proper_interval_is_a_single_value() {
        assert_eq!(
            point_value_i32(&proper_i32(Some(4), Some(4), CLOSED)),
            Some(4)
        );
        assert_eq!(
            point_value_f64(&crate::aom::build::proper_interval(
                Some(4.5),
                Some(4.5),
                true,
                true,
                false,
                false
            )),
            Some(4.5)
        );
    }

    /// A plain point interval with the `Point_interval` defaults answers.
    #[test]
    fn a_default_flagged_point_interval_is_a_single_value() {
        assert_eq!(point_value_i32(&point_i32(7, CLOSED)), Some(7));
        assert_eq!(
            point_value_f64(&crate::aom::build::point_real(2.5)),
            Some(2.5)
        );
    }

    /// The unbounded flags are load-bearing: `Point_interval` only DEFAULTS
    /// them (`…foundation_types.point_interval.adoc`), and `has()`
    /// short-circuits on the flag (`…foundation_types.interval.adoc`), so a
    /// flagged side dominates any stored bound.
    #[test]
    fn an_unbounded_flag_defeats_a_stored_bound_even_on_a_point_interval() {
        assert_eq!(
            point_value_i32(&point_i32(7, [true, false, false, true])),
            None
        );
        assert_eq!(
            point_value_i32(&point_i32(7, [false, true, true, false])),
            None
        );
    }

    /// `{n..n}` with an EXCLUDED bound denotes the empty set, never a value.
    #[test]
    fn an_excluded_bound_is_not_a_single_value() {
        assert_eq!(
            point_value_i32(&proper_i32(Some(4), Some(4), [false, false, false, true])),
            None
        );
        assert_eq!(
            point_value_i32(&proper_i32(Some(4), Some(4), [false, false, true, false])),
            None
        );
    }

    /// A genuine range, a half-line, and an absent bound all decline.
    #[test]
    fn ranges_and_absent_bounds_are_not_single_values() {
        assert_eq!(point_value_i32(&proper_i32(Some(1), Some(5), CLOSED)), None);
        assert_eq!(
            point_value_i32(&proper_i32(Some(1), None, [false, true, true, false])),
            None
        );
        assert_eq!(point_value_i32(&proper_i32(Some(1), None, CLOSED)), None);
    }
}
