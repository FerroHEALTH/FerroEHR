// @generated-from-template templates/openehr-base/foundation_types/time/iso8601_duration_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written `Iso8601_duration` spec behaviour.
//!
//! Covers the accessor functions (component counts, `to_seconds`, `is_partial`,
//! `is_extended`, `is_decimal_sign_comma`, `as_string`), the arithmetic
//! functions (`add`/`subtract`/`multiply`/`divide`/`negative`) and a
//! `PartialOrd` ordering durations by their total-seconds reduction.
//!
//! Spec sources (vendored):
//! - `BASE/docs/UML/classes/org.openehr.base.foundation_types.iso8601_duration.adoc`
//!   (§Functions: the component accessors, `to_seconds`, `is_partial`,
//!   `is_extended`, `is_decimal_sign_comma`, `as_string`; the `add`/`subtract`
//!   semantics that reduce via `to_seconds`, and `multiply`/`divide`/
//!   `negative`).
//! - `BASE/docs/UML/classes/org.openehr.base.foundation_types.time_definitions.adoc`
//!   (§Constants `Average_days_in_year`/`Average_days_in_month` used by
//!   `to_seconds`).
//! - `BASE/docs/foundation_types/master06-time_types.adoc` (§Primitive Time
//!   Types: the negative-duration and mixed-`W` deviations; §Computational
//!   Functions).
//!
//! Invariants — the EIGHT entries the class table declares under §Invariants
//! (`Years_valid` … `Seconds_valid`, each `>= 0`, and `Fractional_second_valid`
//! `>= 0.0 and < 1.0`) are ALL structurally satisfied by the reader: a
//! component is a digit run scanned into an unsigned count, and a fraction is
//! scanned as `0.<digits>`, so no parsed value can break one. What a value CAN
//! be is not a duration at all, which the class table names no rule for, so it
//! is reported under our own `Value_lexical_form_valid` (the same rule
//! `iso8601_date_impl.rs` names).
//!
//! NOTE: the openEHR negative-duration deviation (`master06` §Primitive Time
//! Types) is a leading sign on the VALUE, not on any component, so `-P1Y` still
//! satisfies `Years_valid` — the class declares no sign accessor to read it
//! from.
//!
//! NOTE: the class doc gives an algorithm for `add`/`subtract` (reduce both
//! operands via `to_seconds`) but none for `multiply`/`divide` or comparison —
//! our own design/extension on the spec's own reduction, since component-wise
//! scaling is not expressible (`P1Y * 1.5` would need a fractional year).
//!
//! All four use `to_seconds`, results render in the canonical
//! definite-designator form, and ordering by that scalar puts `P1M` (30.42
//! days) above `P30D`. `partial_cmp` returns `Some(Equal)` only for equal raw
//! strings, so `PT1H30M` and `PT90M` are incomparable, consistent with the
//! derived `PartialEq`.

use std::cmp::Ordering;

use super::iso8601_duration::Iso8601Duration;
use super::iso8601_parse::{ExactSeconds, ParsedDuration, parse_duration, render_duration};
use crate::validate::{InvariantViolation, Validate};

impl Iso8601Duration {
    /// Parsed components, or `None` when `value` is not a valid ISO 8601
    /// duration. Crate-visible because a duration is the argument of every
    /// other time type's computational functions.
    pub(crate) fn parsed(&self) -> Option<ParsedDuration> {
        parse_duration(&self.value)
    }

    /// `Iso8601_duration.years`, or `None` when the value does not parse.
    #[must_use]
    pub fn years(&self) -> Option<u64> {
        self.parsed().map(|p| p.years)
    }

    /// `Iso8601_duration.months`, or `None` when the value does not parse.
    #[must_use]
    pub fn months(&self) -> Option<u64> {
        self.parsed().map(|p| p.months)
    }

    /// `Iso8601_duration.weeks`, or `None` when the value does not parse.
    #[must_use]
    pub fn weeks(&self) -> Option<u64> {
        self.parsed().map(|p| p.weeks)
    }

    /// `Iso8601_duration.days`, or `None` when the value does not parse.
    #[must_use]
    pub fn days(&self) -> Option<u64> {
        self.parsed().map(|p| p.days)
    }

    /// `Iso8601_duration.hours`, or `None` when the value does not parse.
    #[must_use]
    pub fn hours(&self) -> Option<u64> {
        self.parsed().map(|p| p.hours)
    }

    /// `Iso8601_duration.minutes`, or `None` when the value does not parse.
    #[must_use]
    pub fn minutes(&self) -> Option<u64> {
        self.parsed().map(|p| p.minutes)
    }

    /// `Iso8601_duration.seconds` (integral), or `None` when the value does not
    /// parse.
    #[must_use]
    pub fn seconds(&self) -> Option<u64> {
        self.parsed().map(|p| p.seconds)
    }

    /// `Iso8601_duration.fractional_seconds`, or `None` when the value does not
    /// parse.
    #[must_use]
    pub fn fractional_seconds(&self) -> Option<f64> {
        self.parsed().map(|p| p.fractional_seconds)
    }

    /// True when the value carries a leading negative sign (an openEHR
    /// deviation — `master06` §Primitive Time Types), or `None` when the value
    /// does not parse.
    #[must_use]
    pub fn is_negative(&self) -> Option<bool> {
        self.parsed().map(|p| p.negative)
    }

    /// `Iso8601_duration.is_partial`: always `False` for a duration (effected
    /// in the spec), or `None` when the value does not parse.
    #[must_use]
    pub fn is_partial(&self) -> Option<bool> {
        self.parsed().map(|_| false)
    }

    /// `Iso8601_duration.is_extended`: always `True` for a duration (effected in
    /// the spec — a duration has no compact form), or `None` when the value does
    /// not parse.
    #[must_use]
    pub fn is_extended(&self) -> Option<bool> {
        self.parsed().map(|_| true)
    }

    /// `Iso8601_duration.is_decimal_sign_comma`: true when the fractional
    /// seconds are written with `','` rather than `'.'`. False when the value
    /// carries no fraction or does not parse.
    #[must_use]
    pub fn is_decimal_sign_comma(&self) -> bool {
        self.parsed().is_some_and(|p| p.decimal_sign_comma)
    }

    /// `Iso8601_duration.to_seconds`: total seconds equivalent (sign applied),
    /// with years/months reduced via `Time_definitions.Average_days_in_year`
    /// / `Average_days_in_month`. `None` when the value does not parse.
    #[must_use]
    pub fn to_seconds(&self) -> Option<f64> {
        self.parsed().map(ParsedDuration::to_seconds)
    }

    /// `Iso8601_duration.as_string`: "Return the duration string value" — the
    /// stored value verbatim (unlike the date/time types, the duration doc asks
    /// for the value itself, and a duration has no compact form to re-spell).
    /// A value that does not parse is likewise returned verbatim.
    #[must_use]
    pub fn as_string(&self) -> String {
        self.value.clone()
    }

    /// `Iso8601_duration.add` (alias `'+'`): "Arithmetic addition of a duration
    /// to a duration, via conversion to seconds, using
    /// `Time_definitions.Average_days_in_year` and `Average_days_in_month`" —
    /// the class doc's own algorithm.
    ///
    /// `None` when either value does not parse or the result overflows.
    #[must_use]
    pub fn add(&self, a_val: &Self) -> Option<Self> {
        Self::rendered(self.exact_seconds()?.checked_add(a_val.exact_seconds()?)?)
    }

    /// `Iso8601_duration.subtract` (alias `'-'`): the same seconds reduction as
    /// [`Iso8601Duration::add`], subtracted.
    ///
    /// `None` when either value does not parse or the result overflows.
    #[must_use]
    pub fn subtract(&self, a_val: &Self) -> Option<Self> {
        Self::rendered(self.exact_seconds()?.checked_sub(a_val.exact_seconds()?)?)
    }

    /// `Iso8601_duration.multiply` (alias `'*'`): the duration scaled by a
    /// `Real` factor.
    ///
    /// `None` when the value does not parse or the result is not representable.
    #[must_use]
    pub fn multiply(&self, a_val: f64) -> Option<Self> {
        Self::rendered(ExactSeconds::from_f64(
            self.exact_seconds()?.as_f64() * a_val,
        )?)
    }

    /// `Iso8601_duration.divide` (alias `'/'`): the duration divided by a `Real`
    /// factor.
    ///
    /// `None` when the value does not parse or the result is not representable —
    /// which includes division by zero (an infinite result).
    #[must_use]
    pub fn divide(&self, a_val: f64) -> Option<Self> {
        Self::rendered(ExactSeconds::from_f64(
            self.exact_seconds()?.as_f64() / a_val,
        )?)
    }

    /// `Iso8601_duration.negative` (alias `'-'`): "Generate negative of current
    /// duration value" — the sign is flipped and the components are kept exactly
    /// as written (the openEHR negative-duration deviation, `master06`
    /// §Primitive Time Types).
    ///
    /// `None` when the value does not parse.
    ///
    /// NOTE: unlike `add`/`subtract`, this does NOT go through the seconds
    /// reduction: the spec asks for the negative of *this* duration value, so
    /// flipping the leading sign preserves the `Y`/`M`/`W` designators, the
    /// fractional precision and the decimal sign — a reduction would destroy
    /// all four. No openEHR spec prescribes the output spelling — our own
    /// design/extension.
    #[must_use]
    pub fn negative(&self) -> Option<Self> {
        let value = if self.parsed()?.negative {
            self.value.strip_prefix('-')?.to_owned()
        } else {
            format!("-{}", self.value)
        };
        Some(Self { value })
    }

    /// The duration as an exact signed second quantity (the `to_seconds`
    /// reduction, computed in whole seconds), or `None` when the value does not
    /// parse or the reduction overflows.
    fn exact_seconds(&self) -> Option<ExactSeconds> {
        self.parsed()?.to_exact_seconds()
    }

    /// A computed second quantity as a duration value, in the canonical
    /// definite-designator form (see [`render_duration`]).
    fn rendered(total: ExactSeconds) -> Option<Self> {
        Some(Self {
            value: render_duration(total)?,
        })
    }
}

impl PartialOrd for Iso8601Duration {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        if self.value == other.value {
            return Some(Ordering::Equal); // consistent with the derived PartialEq
        }
        let a = self.parsed()?.to_seconds();
        let b = other.parsed()?.to_seconds();
        match a.partial_cmp(&b)? {
            // Equal magnitude but (given the string check above) different
            // spellings ⇒ incomparable per decision 4, not equal.
            Ordering::Equal => None,
            ord => Some(ord),
        }
    }
}

impl Validate for Iso8601Duration {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        if self.parsed().is_none() {
            out.push(InvariantViolation::here(
                "Invariant Value_lexical_form_valid failed on type Iso8601_duration",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dur(v: &str) -> Iso8601Duration {
        Iso8601Duration {
            value: v.to_owned(),
        }
    }

    /// The value of a computed duration, or `"None"`.
    fn value(d: Option<Iso8601Duration>) -> String {
        d.map_or_else(|| "None".to_owned(), |d| d.value)
    }

    // ── ordering by total-seconds reduction ──────────────────────────────────

    #[test]
    fn nominal_month_exceeds_thirty_days() {
        // P1M = 30.42 days > P30D (decision 5).
        assert_eq!(
            dur("P1M").partial_cmp(&dur("P30D")),
            Some(Ordering::Greater)
        );
        assert_eq!(
            dur("P1Y").partial_cmp(&dur("P365D")),
            Some(Ordering::Greater)
        ); // 365.24 > 365
    }

    #[test]
    fn plain_ordering() {
        assert_eq!(dur("PT30M").partial_cmp(&dur("PT1H")), Some(Ordering::Less));
        assert_eq!(dur("P2W").partial_cmp(&dur("P7D")), Some(Ordering::Greater));
    }

    #[test]
    fn equal_strings_are_equal() {
        assert_eq!(
            dur("PT1H30M").partial_cmp(&dur("PT1H30M")),
            Some(Ordering::Equal)
        );
    }

    #[test]
    fn equal_magnitude_different_spelling_is_incomparable() {
        // PT1H30M and PT90M are both 5400 s but written differently ⇒ None.
        assert_eq!(dur("PT1H30M").partial_cmp(&dur("PT90M")), None);
    }

    #[test]
    fn negative_durations_order_below_positive() {
        assert_eq!(dur("-P3M").partial_cmp(&dur("P0D")), Some(Ordering::Less));
        assert_eq!(dur("-P1Y").partial_cmp(&dur("-P1M")), Some(Ordering::Less));
        assert_eq!(
            dur("P1D").partial_cmp(&dur("-P1D")),
            Some(Ordering::Greater)
        );
    }

    #[test]
    fn fractional_seconds_order() {
        assert_eq!(
            dur("PT0.5S").partial_cmp(&dur("PT1S")),
            Some(Ordering::Less)
        );
    }

    // ── malformed / weeks-mixing ─────────────────────────────────────────────

    #[test]
    fn weeks_mixed_with_other_designators_parse() {
        // openEHR deviation: W mixable with other designators.
        assert_eq!(
            dur("P1W3D").to_seconds(),
            Some(10.0 * super::super::iso8601_parse::SECONDS_IN_DAY)
        );
        assert_eq!(dur("P2W").partial_cmp(&dur("P1W")), Some(Ordering::Greater));
    }

    #[test]
    fn malformed_values_are_incomparable() {
        assert_eq!(dur("nonsense").partial_cmp(&dur("P1D")), None);
        assert_eq!(dur("P").partial_cmp(&dur("P1D")), None); // no components
        assert_eq!(dur("PT").partial_cmp(&dur("P1D")), None);
        assert_eq!(dur("1D").partial_cmp(&dur("P1D")), None); // missing leading P
        assert!(dur("P1.5Y").years().is_none()); // fraction only allowed on seconds
    }

    // ── accessors ────────────────────────────────────────────────────────────

    #[test]
    fn accessors_report_components() {
        let d = dur("P1Y2M3W4DT5H6M7.5S");
        assert_eq!(d.years(), Some(1));
        assert_eq!(d.months(), Some(2));
        assert_eq!(d.weeks(), Some(3));
        assert_eq!(d.days(), Some(4));
        assert_eq!(d.hours(), Some(5));
        assert_eq!(d.minutes(), Some(6));
        assert_eq!(d.seconds(), Some(7));
        assert_eq!(d.fractional_seconds(), Some(0.5));
        assert_eq!(d.is_partial(), Some(false));
        assert_eq!(d.is_negative(), Some(false));
        assert_eq!(dur("-P3M").is_negative(), Some(true));
    }

    #[test]
    fn to_seconds_uses_average_lengths() {
        // PT1H = 3600 s.
        assert_eq!(dur("PT1H").to_seconds(), Some(3600.0));
        // P1D = 86400 s.
        assert_eq!(dur("P1D").to_seconds(), Some(86_400.0));
    }

    // ── lexical predicates / as_string ───────────────────────────────────────

    #[test]
    fn a_duration_is_always_extended_and_never_partial() {
        // iso8601_duration.adoc §Functions: is_extended "Returns True",
        // is_partial "Returns False".
        assert_eq!(dur("P1Y2M3W4DT5H6M7.5S").is_extended(), Some(true));
        assert_eq!(dur("P1D").is_extended(), Some(true));
        assert_eq!(dur("P1D").is_partial(), Some(false));
        assert_eq!(dur("nonsense").is_extended(), None);
    }

    #[test]
    fn decimal_sign_comma_is_reported() {
        assert!(dur("PT0,5S").is_decimal_sign_comma());
        assert!(dur("P1DT1M0,25S").is_decimal_sign_comma());
        assert!(!dur("PT0.5S").is_decimal_sign_comma());
        assert!(!dur("PT1S").is_decimal_sign_comma());
        assert!(!dur("nonsense").is_decimal_sign_comma());
    }

    #[test]
    fn as_string_returns_the_stored_value() {
        // The duration doc asks for "the duration string value" (not, as for the
        // date/time types, an extended re-spelling — a duration has no compact
        // form), so every component spelling survives.
        assert_eq!(dur("P1W3D").as_string(), "P1W3D");
        assert_eq!(dur("-P3M").as_string(), "-P3M");
        assert_eq!(dur("PT0,50S").as_string(), "PT0,50S");
        assert_eq!(dur("nonsense").as_string(), "nonsense");
    }

    // ── add / subtract (via the to_seconds reduction) ─────────────────────────

    #[test]
    fn add_and_subtract_reduce_via_seconds() {
        assert_eq!(value(dur("PT1H").add(&dur("PT30M"))), "PT1H30M");
        assert_eq!(value(dur("P1D").add(&dur("PT12H"))), "P1DT12H");
        assert_eq!(value(dur("PT1H").subtract(&dur("PT30M"))), "PT30M");
        // A negative result carries the openEHR negative-duration sign.
        assert_eq!(value(dur("PT1H").subtract(&dur("PT90M"))), "-PT30M");
        assert_eq!(value(dur("PT1H").subtract(&dur("PT1H"))), "PT0S");
        // A negative operand is honoured.
        assert_eq!(value(dur("PT1H").add(&dur("-PT30M"))), "PT30M");
    }

    #[test]
    fn add_uses_the_average_month_length() {
        // The class doc mandates the Average_days_in_month reduction: P1M is
        // 30 days and 10:04:48, which the canonical result form spells out.
        assert_eq!(value(dur("P1M").add(&dur("PT0S"))), "P30DT10H4M48S");
        // P1Y = 365 days and 05:45:36 (Average_days_in_year = 365.24).
        assert_eq!(value(dur("P1Y").add(&dur("PT0S"))), "P365DT5H45M36S");
    }

    #[test]
    fn add_and_subtract_keep_fractional_seconds() {
        assert_eq!(value(dur("PT0.5S").add(&dur("PT0.25S"))), "PT0.75S");
        assert_eq!(value(dur("PT0.5S").add(&dur("PT0.5S"))), "PT1S");
        assert_eq!(value(dur("PT1S").subtract(&dur("PT0.25S"))), "PT0.75S");
        assert_eq!(value(dur("PT0.25S").subtract(&dur("PT1S"))), "-PT0.75S");
        // A comma decimal sign parses and renders back with a period.
        assert_eq!(value(dur("PT0,5S").add(&dur("PT0,5S"))), "PT1S");
    }

    // ── multiply / divide / negative ─────────────────────────────────────────

    #[test]
    fn multiply_and_divide_scale_the_reduction() {
        assert_eq!(value(dur("PT1H").multiply(2.0)), "PT2H");
        assert_eq!(value(dur("P1D").multiply(0.5)), "PT12H");
        assert_eq!(value(dur("PT1H").divide(2.0)), "PT30M");
        assert_eq!(value(dur("PT1S").divide(4.0)), "PT0.25S");
        // Scaling by a negative factor flips the sign.
        assert_eq!(value(dur("PT1H").multiply(-1.0)), "-PT1H");
        assert_eq!(value(dur("-P1D").multiply(2.0)), "-P2D");
    }

    #[test]
    fn multiply_and_divide_round_trip() {
        let original = dur("P1DT2H3M4S");
        let doubled = original.multiply(2.0).unwrap();
        assert_eq!(doubled.value, "P2DT4H6M8S");
        assert_eq!(doubled.divide(2.0).unwrap().value, "P1DT2H3M4S");
    }

    #[test]
    fn divide_by_zero_and_a_non_finite_factor_are_uncomputable() {
        assert!(dur("PT1H").divide(0.0).is_none());
        assert!(dur("PT1H").multiply(f64::NAN).is_none());
        assert!(dur("PT1H").multiply(f64::INFINITY).is_none());
    }

    #[test]
    fn negative_flips_the_sign_and_keeps_the_components() {
        assert_eq!(value(dur("P1Y2M").negative()), "-P1Y2M");
        assert_eq!(value(dur("-P3M").negative()), "P3M");
        // Round-trip: the exact spelling comes back, designators and all.
        let original = dur("P1Y2M3W4DT5H6M7,50S");
        let flipped = original.negative().unwrap();
        assert_eq!(flipped.value, "-P1Y2M3W4DT5H6M7,50S");
        assert_eq!(flipped.negative().unwrap().value, original.value);
        assert!(dur("nonsense").negative().is_none());
    }

    // ── invariants ───────────────────────────────────────────────────────────

    /// A value that is not the `P[nnY][nnM][nnW][nnD][T[nnH][nnM][nnS]]`
    /// production is refused under the one rule this class can break.
    #[test]
    fn a_value_that_is_not_a_duration_reports_the_lexical_rule() {
        for bad in [
            "1D", "P", "PT", "", "nonsense", "P1Y1Y", "P1D1M", "P1YT", "-P", "PT1.S",
        ] {
            let v = dur(bad).invariants();
            assert_eq!(v.len(), 1, "{bad:?} should be refused, got {v:?}");
            assert_eq!(
                v[0].message,
                "Invariant Value_lexical_form_valid failed on type Iso8601_duration"
            );
        }
    }

    /// Every declared component invariant is structurally satisfied, including
    /// on a negative duration (the sign is on the value, not on a component).
    #[test]
    fn valid_durations_report_nothing() {
        for good in [
            "P1Y",
            "-P3M",
            "P1W",
            "P30D",
            "PT1M",
            "PT0S",
            "PT0,5S",
            "P1Y2M3W4DT5H6M7.5S",
        ] {
            assert!(
                dur(good).invariants().is_empty(),
                "{good:?} is a valid Iso8601_duration"
            );
        }
    }
}
