// @generated-from-template templates/openehr-base/foundation_types/time/iso8601_date_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written `Iso8601_date` spec behaviour.
//!
//! Covers the accessor functions (`is_partial`, `is_extended`, `month_unknown`,
//! `day_unknown`, `year`/`month`/`day`, `as_string`), the computational
//! functions (definite `add`/`subtract`/`diff` and nominal
//! `add_nominal`/`subtract_nominal`), and a `PartialOrd` implementing range
//! semantics over partial dates.
//!
//! Spec sources (vendored):
//! - `BASE/docs/UML/classes/org.openehr.base.foundation_types.iso8601_date.adoc`
//!   (§Functions: the accessors, `as_string`, the definite `add`/`subtract`/
//!   `diff` and the nominal `add_nominal`/`subtract_nominal` with their
//!   day-clamping rules; §Invariants).
//! - `BASE/docs/foundation_types/master06-time_types.adoc` (§Primitive Time
//!   Types: the accepted date forms; week dates excluded; §Computational
//!   Functions: the definite/nominal split and the average-length constants the
//!   definite forms use).
//!
//! Invariants — the FOUR entries the class table declares under §Invariants,
//! plus one rule of our own naming:
//! - `Month_valid` (`not month_unknown implies valid_month (month)`) and
//!   `Day_valid` (`not day_unknown implies valid_day (year, month, day)`) —
//!   checked, each under its own name.
//! - `Year_valid` (`valid_year (year)`, i.e. `year >= 0`) — structurally
//!   satisfied: the accepted forms admit only four zero-filled digits.
//! - `Partial_validity` (`month_unknown implies day_unknown`) — structurally
//!   satisfied: no accepted form writes a day without a month.
//! - `Value_lexical_form_valid` — OUR OWN name, because the class table
//!   declares none: a value that is not the `valid_iso8601_date` production at
//!   all has no components, so every declared invariant holds vacuously
//!   (the same rule `version_tree_id_impl.rs` names for identifiers).
//!
//! NOTE: no openEHR spec governs arithmetic on a PARTIAL date, the rounding of
//! a definite result, or date comparison (`Ordered` is abstract) — our own
//! design/extension. A partial or unparseable operand yields `None` rather than
//! an invented completion; a definite result returns the calendar date
//! CONTAINING the resulting instant, the only reading that keeps `add` and
//! `subtract` consistent; and `X < Y` holds only when every completion of `X`
//! precedes every completion of `Y`, with `Some(Equal)` only for equal raw
//! strings.
//!
//! A consumer needing a TOTAL order must choose its own completion policy; this
//! comparison never invents an order the value does not carry.

use std::cmp::Ordering;

use super::iso8601_date::Iso8601Date;
use super::iso8601_duration::Iso8601Duration;
use super::iso8601_parse::{
    EXACT_SECONDS_IN_DAY, ExactSeconds, ParsedDate, as_extended_date, civil_from_days,
    days_from_civil, days_in_month, parse_date, render_date_extended, render_duration, scan_date,
    shift_months,
};
use crate::validate::{InvariantViolation, Validate};

impl Iso8601Date {
    /// Parsed components, or `None` when `value` is not a valid ISO 8601 date.
    fn parsed(&self) -> Option<ParsedDate> {
        parse_date(&self.value)
    }

    /// The year part (`Iso8601_date.year`), or `None` when the value does not
    /// parse. Year is always present in a valid date.
    #[must_use]
    pub fn year(&self) -> Option<u32> {
        self.parsed().map(|p| p.year)
    }

    /// The month part (`Iso8601_date.month`), or `None` when month is unknown
    /// or the value does not parse. (The spec's `month()` returns 0 when
    /// `month_unknown`; we report honest absence as `None`.)
    #[must_use]
    pub fn month(&self) -> Option<u32> {
        self.parsed().and_then(|p| p.month)
    }

    /// The day part (`Iso8601_date.day`), or `None` when day is unknown or the
    /// value does not parse.
    #[must_use]
    pub fn day(&self) -> Option<u32> {
        self.parsed().and_then(|p| p.day)
    }

    /// `Iso8601_date.month_unknown`: true when the value is of the form `YYYY`
    /// (or does not parse).
    #[must_use]
    pub fn month_unknown(&self) -> bool {
        self.parsed().is_none_or(|p| p.month.is_none())
    }

    /// `Iso8601_date.day_unknown`: true when the value omits the day (or does
    /// not parse).
    #[must_use]
    pub fn day_unknown(&self) -> bool {
        self.parsed().is_none_or(|p| p.day.is_none())
    }

    /// `Iso8601_date.is_partial`: true when days or more is missing (a value
    /// that does not parse is treated as not a complete date).
    #[must_use]
    pub fn is_partial(&self) -> bool {
        self.parsed().is_none_or(|p| p.day.is_none())
    }

    /// `Iso8601_date.is_extended`: true when the value uses `'-'` separators
    /// (and, for the separator-less `YYYY` form, always — see the `is_extended`
    /// NOTE in `iso8601_parse.rs`). A value that does not parse is not extended.
    #[must_use]
    pub fn is_extended(&self) -> bool {
        self.parsed().is_some_and(|p| p.extended)
    }

    /// `Iso8601_date.as_string`: "Return string value in extended format" — a
    /// compact value is re-spelled with `'-'` separators, an already-extended
    /// one is returned unchanged.
    ///
    /// NOTE: the spec does not say what a value that is not a valid ISO 8601
    /// date returns. It is returned verbatim, since `Iso8601_type.value` is the
    /// only representation there is — our own design/extension.
    #[must_use]
    pub fn as_string(&self) -> String {
        as_extended_date(&self.value).unwrap_or_else(|| self.value.clone())
    }

    /// `Iso8601_date.add` (alias `'+'`): DEFINITE addition of a duration —
    /// `a_diff` is reduced to an exact number of seconds with the
    /// `Time_definitions` average year/month lengths
    /// (`master06` §Computational Functions) and the result is the calendar date
    /// containing the shifted instant.
    ///
    /// `None` when either value does not parse, when this date is partial, or
    /// when the result leaves the representable `0000`–`9999` year range.
    #[must_use]
    pub fn add(&self, a_diff: &Iso8601Duration) -> Option<Self> {
        self.definite_shift(a_diff, false)
    }

    /// `Iso8601_date.subtract` (alias `'-'`): DEFINITE subtraction of a
    /// duration. See [`Iso8601Date::add`].
    ///
    /// `None` under the same conditions as [`Iso8601Date::add`].
    #[must_use]
    pub fn subtract(&self, a_diff: &Iso8601Duration) -> Option<Self> {
        self.definite_shift(a_diff, true)
    }

    /// `Iso8601_date.diff` (alias `'-'`): the difference `self - a_date` as an
    /// `Iso8601_duration` of whole days — negative (the openEHR
    /// negative-duration deviation, `master06` §Primitive Time Types) when
    /// `a_date` is the later date.
    ///
    /// `None` when either value does not parse or is partial.
    #[must_use]
    pub fn diff(&self, a_date: &Self) -> Option<Iso8601Duration> {
        let days = self.day_index()?.checked_sub(a_date.day_index()?)?;
        let seconds = days.checked_mul(EXACT_SECONDS_IN_DAY)?;
        Some(Iso8601Duration {
            value: render_duration(ExactSeconds::new(seconds, 0.0)?)?,
        })
    }

    /// `Iso8601_date.add_nominal` (alias `'++'`): NOMINAL addition — years and
    /// months advance the calendar to the same day-of-month, clamped down when
    /// the target month is shorter (29 Feb `++ P1Y` → 28 Feb next year; 31 Jan
    /// `++ P1M` → 28 Feb, or 29 Feb in a leap year), and the remaining
    /// components (weeks, days and any time part) apply as an exact calendar
    /// shift.
    ///
    /// `None` when either value does not parse, when this date is partial, or
    /// when the result leaves the representable `0000`–`9999` year range.
    #[must_use]
    pub fn add_nominal(&self, a_diff: &Iso8601Duration) -> Option<Self> {
        self.nominal_shift(a_diff, false)
    }

    /// `Iso8601_date.subtract_nominal` (alias `'--'`): NOMINAL subtraction, with
    /// the day-clamping semantics of [`Iso8601Date::add_nominal`].
    ///
    /// `None` under the same conditions as [`Iso8601Date::add_nominal`].
    #[must_use]
    pub fn subtract_nominal(&self, a_diff: &Iso8601Duration) -> Option<Self> {
        self.nominal_shift(a_diff, true)
    }

    /// The date's day count since 1970-01-01, or `None` when the value does not
    /// parse or is partial (arithmetic needs a complete value).
    fn day_index(&self) -> Option<i64> {
        let p = self.parsed()?;
        Some(days_from_civil(p.year, p.month?, p.day?))
    }

    /// The date reached by shifting this one's midnight instant by `shift`
    /// seconds, truncated to the containing calendar day. A fractional part is
    /// irrelevant to that truncation: `shift.frac` is in `[0.0, 1.0)` and the
    /// whole-second total is an integer, so adding it can never cross a day
    /// boundary.
    fn shifted(&self, shift: ExactSeconds) -> Option<Self> {
        let base = self.day_index()?.checked_mul(EXACT_SECONDS_IN_DAY)?;
        let total = base.checked_add(shift.whole)?;
        let (year, month, day) = civil_from_days(total.div_euclid(EXACT_SECONDS_IN_DAY))?;
        Some(Self {
            value: render_date_extended(year, Some(month), Some(day)),
        })
    }

    /// Shared body of the definite `add`/`subtract`.
    fn definite_shift(&self, a_diff: &Iso8601Duration, subtract: bool) -> Option<Self> {
        self.shifted(a_diff.parsed()?.to_definite_shift(subtract)?)
    }

    /// Shared body of the nominal `add_nominal`/`subtract_nominal`: the
    /// year/month part shifts the calendar with day-clamping, then the
    /// sub-month remainder applies as an exact shift.
    fn nominal_shift(&self, a_diff: &Iso8601Duration, subtract: bool) -> Option<Self> {
        let (months, remainder) = a_diff.parsed()?.to_nominal_parts(subtract)?;
        let p = self.parsed()?;
        let (year, month, day) = shift_months(p.year, p.month?, p.day?, months)?;
        Self {
            value: render_date_extended(year, Some(month), Some(day)),
        }
        .shifted(remainder)
    }
}

/// Range-semantics comparison of two parsed dates on their shared prefix. Never
/// returns `Some(Equal)`: an equal string is handled before parsing, so equal
/// components here (with differing strings) are incomparable (`None`).
fn cmp_date(a: &ParsedDate, b: &ParsedDate) -> Option<Ordering> {
    match a.year.cmp(&b.year) {
        Ordering::Equal => {}
        ord => return Some(ord),
    }
    match (a.month, b.month) {
        (Some(am), Some(bm)) => match am.cmp(&bm) {
            Ordering::Equal => {}
            ord => return Some(ord),
        },
        // One side's month is unknown while the years match: its completions
        // span the whole year and overlap the other's — undecidable.
        _ => return None,
    }
    match (a.day, b.day) {
        (Some(ad), Some(bd)) => match ad.cmp(&bd) {
            Ordering::Equal => None, // equal components, differing strings ⇒ incomparable
            ord => Some(ord),
        },
        _ => None,
    }
}

impl PartialOrd for Iso8601Date {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        if self.value == other.value {
            return Some(Ordering::Equal); // consistent with the derived PartialEq
        }
        cmp_date(&self.parsed()?, &other.parsed()?)
    }
}

/// The uniform violation for one named invariant, on the class reporting it —
/// `Iso8601_date`, or `Iso8601_date_time` where it re-declares the same rule.
fn failed(invariant: &str, type_name: &str) -> InvariantViolation {
    InvariantViolation::here(format!("Invariant {invariant} failed on type {type_name}"))
}

/// Appends the date-component invariant violations of a scanned value.
///
/// The rules are `Month_valid` and `Day_valid`, reported on `type_name` —
/// `Iso8601_date_time` re-declares both.
///
/// `Day_valid` is checked only under an in-range month: its rule is
/// `valid_day (y, m, d) = d >= 1 and d <= days_in_month (m, y)`
/// (`time_definitions.adoc` §Functions), which is undefined where `valid_month`
/// already fails.
pub(crate) fn push_date_component_violations(
    d: &ParsedDate,
    type_name: &str,
    out: &mut Vec<InvariantViolation>,
) {
    let month_valid = d.month.is_none_or(|m| (1..=12).contains(&m));
    if !month_valid {
        out.push(failed("Month_valid", type_name));
    }
    if month_valid
        && let (Some(month), Some(day)) = (d.month, d.day)
        && !days_in_month(d.year, month).is_some_and(|last| (1..=last).contains(&day))
    {
        out.push(failed("Day_valid", type_name));
    }
}

impl Validate for Iso8601Date {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        let Some(d) = scan_date(&self.value) else {
            out.push(failed("Value_lexical_form_valid", "Iso8601_date"));
            return;
        };
        push_date_component_violations(&d, "Iso8601_date", out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(v: &str) -> Iso8601Date {
        Iso8601Date {
            value: v.to_owned(),
        }
    }

    fn dur(v: &str) -> Iso8601Duration {
        Iso8601Duration {
            value: v.to_owned(),
        }
    }

    /// The value of a computed date, or `"None"` — keeps the arithmetic
    /// assertions below readable as string comparisons.
    fn value(d: Option<Iso8601Date>) -> String {
        d.map_or_else(|| "None".to_owned(), |d| d.value)
    }

    /// The value of a computed duration, or `"None"`.
    fn duration_value(d: Option<Iso8601Duration>) -> String {
        d.map_or_else(|| "None".to_owned(), |d| d.value)
    }

    // ── full-vs-full ordering ────────────────────────────────────────────────

    #[test]
    fn full_dates_order_component_wise() {
        assert_eq!(
            date("2020-01-01").partial_cmp(&date("2020-06-15")),
            Some(Ordering::Less)
        );
        assert_eq!(
            date("2020-12-31").partial_cmp(&date("2020-06-15")),
            Some(Ordering::Greater)
        );
        assert_eq!(
            date("2019-12-31").partial_cmp(&date("2020-01-01")),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn equal_strings_are_equal() {
        assert_eq!(
            date("2020-06-15").partial_cmp(&date("2020-06-15")),
            Some(Ordering::Equal)
        );
        assert_eq!(
            date("2020-06").partial_cmp(&date("2020-06")),
            Some(Ordering::Equal)
        );
        assert_eq!(
            date("2020").partial_cmp(&date("2020")),
            Some(Ordering::Equal)
        );
    }

    // ── partial range semantics ──────────────────────────────────────────────

    #[test]
    fn partial_year_before_full_date_when_separated() {
        // 2019 spans all of 2019, entirely before 2020-06-15.
        assert_eq!(
            date("2019").partial_cmp(&date("2020-06-15")),
            Some(Ordering::Less)
        );
        assert_eq!(
            date("2021").partial_cmp(&date("2020-06-15")),
            Some(Ordering::Greater)
        );
    }

    #[test]
    fn overlapping_partials_are_incomparable() {
        // 2020 spans all of 2020, overlapping 2020-06-15 ⇒ undecidable.
        assert_eq!(date("2020").partial_cmp(&date("2020-06-15")), None);
        // 2020-06 spans June 2020, overlapping 2020-06-15.
        assert_eq!(date("2020-06").partial_cmp(&date("2020-06-15")), None);
    }

    #[test]
    fn equal_precision_partials_order() {
        assert_eq!(
            date("2020-06").partial_cmp(&date("2020-07")),
            Some(Ordering::Less)
        );
        assert_eq!(
            date("2020-08").partial_cmp(&date("2020-07")),
            Some(Ordering::Greater)
        );
        assert_eq!(
            date("2019").partial_cmp(&date("2020")),
            Some(Ordering::Less)
        );
    }

    // ── mixed compact / extended ─────────────────────────────────────────────

    #[test]
    fn compact_vs_extended_same_instant_is_incomparable() {
        // Same components, different strings ⇒ None (decision 4).
        assert_eq!(date("20200615").partial_cmp(&date("2020-06-15")), None);
    }

    #[test]
    fn compact_dates_order_among_themselves() {
        assert_eq!(
            date("20200615").partial_cmp(&date("20200616")),
            Some(Ordering::Less)
        );
        assert_eq!(
            date("20201231").partial_cmp(&date("20200101")),
            Some(Ordering::Greater)
        );
    }

    // ── malformed / excluded forms ───────────────────────────────────────────

    #[test]
    fn malformed_values_are_incomparable() {
        assert_eq!(date("not-a-date").partial_cmp(&date("2020")), None);
        assert_eq!(date("2020-13").partial_cmp(&date("2020-06")), None); // month 13 invalid
        assert_eq!(date("2020-02-30").partial_cmp(&date("2020-02-28")), None); // Feb 30 invalid
    }

    #[test]
    fn week_dates_are_rejected() {
        assert_eq!(date("2020-W01").partial_cmp(&date("2020-06-15")), None);
        assert!(date("2020-W01").year().is_none());
    }

    #[test]
    fn leap_day_valid_only_in_leap_years() {
        assert_eq!(
            date("2020-02-29").partial_cmp(&date("2020-03-01")),
            Some(Ordering::Less)
        );
        assert_eq!(date("2021-02-29").partial_cmp(&date("2021-03-01")), None); // 2021 not a leap year
    }

    // ── accessors ────────────────────────────────────────────────────────────

    #[test]
    fn accessors_report_components_and_unknowns() {
        let full = date("2020-06-15");
        assert_eq!(full.year(), Some(2020));
        assert_eq!(full.month(), Some(6));
        assert_eq!(full.day(), Some(15));
        assert!(!full.month_unknown());
        assert!(!full.day_unknown());
        assert!(!full.is_partial());

        let year_only = date("2020");
        assert_eq!(year_only.year(), Some(2020));
        assert_eq!(year_only.month(), None);
        assert!(year_only.month_unknown());
        assert!(year_only.day_unknown());
        assert!(year_only.is_partial());

        let year_month = date("2020-06");
        assert_eq!(year_month.month(), Some(6));
        assert!(!year_month.month_unknown());
        assert!(year_month.day_unknown());
        assert!(year_month.is_partial());
    }

    // ── is_extended / as_string ──────────────────────────────────────────────

    #[test]
    fn extended_and_compact_forms_are_distinguished() {
        assert!(date("2020-06-15").is_extended());
        assert!(date("2020-06").is_extended());
        assert!(date("2020").is_extended()); // no separator position at all
        assert!(!date("20200615").is_extended());
        assert!(!date("202006").is_extended());
        assert!(!date("not-a-date").is_extended());
    }

    #[test]
    fn as_string_returns_the_extended_form() {
        assert_eq!(date("20200615").as_string(), "2020-06-15");
        assert_eq!(date("202006").as_string(), "2020-06");
        assert_eq!(date("2020-06-15").as_string(), "2020-06-15");
        assert_eq!(date("2020").as_string(), "2020");
        // A value that is not a valid date has no extended form: verbatim.
        assert_eq!(date("2020-13-01").as_string(), "2020-13-01");
    }

    // ── nominal arithmetic (the class doc's own examples) ─────────────────────

    #[test]
    fn nominal_year_on_leap_day_clamps_to_28_february() {
        // iso8601_date.adoc §Functions add_nominal: "with the exception of the
        // date 29 February in a leap year, to which the addition of a nominal
        // year will result in 28 February of the following year".
        assert_eq!(
            value(date("2020-02-29").add_nominal(&dur("P1Y"))),
            "2021-02-28"
        );
        // A non-leap-day date keeps its day-of-month.
        assert_eq!(
            value(date("2020-06-15").add_nominal(&dur("P1Y"))),
            "2021-06-15"
        );
        // ... and the same clamp applies downwards.
        assert_eq!(
            value(date("2020-02-29").subtract_nominal(&dur("P1Y"))),
            "2019-02-28"
        );
    }

    #[test]
    fn nominal_month_clamps_into_a_shorter_month() {
        // add_nominal: "in the case of adding a month to the date 31 Jan, the
        // result will be 28 Feb in a non-leap year (i.e. three less) and 29 Feb
        // in a leap year (i.e. two less)".
        assert_eq!(
            value(date("2020-01-31").add_nominal(&dur("P1M"))),
            "2020-02-29"
        );
        assert_eq!(
            value(date("2021-01-31").add_nominal(&dur("P1M"))),
            "2021-02-28"
        );
        // "one or two days less where the following month is shorter" — 31 May
        // to 30 June.
        assert_eq!(
            value(date("2020-05-31").add_nominal(&dur("P1M"))),
            "2020-06-30"
        );
        // "the same day in the following month, if it exists".
        assert_eq!(
            value(date("2020-01-15").add_nominal(&dur("P1M"))),
            "2020-02-15"
        );
        assert_eq!(
            value(date("2020-03-31").subtract_nominal(&dur("P1M"))),
            "2020-02-29"
        );
    }

    #[test]
    fn nominal_year_and_month_combine_before_the_day_shift() {
        // P1Y1M from 31 Dec: 13 months to 31 Jan (exists), then no day part.
        assert_eq!(
            value(date("2019-12-31").add_nominal(&dur("P1Y1M"))),
            "2021-01-31"
        );
        // Weeks and days apply as exact calendar shifts after the clamp.
        assert_eq!(
            value(date("2020-01-31").add_nominal(&dur("P1M1D"))),
            "2020-03-01"
        );
        assert_eq!(
            value(date("2020-06-15").add_nominal(&dur("P2W"))),
            "2020-06-29"
        );
    }

    #[test]
    fn nominal_arithmetic_honours_a_negative_duration() {
        // The openEHR negative-duration deviation: '-P1M' added is a month back.
        assert_eq!(
            value(date("2020-03-31").add_nominal(&dur("-P1M"))),
            "2020-02-29"
        );
        assert_eq!(
            value(date("2020-02-29").subtract_nominal(&dur("-P1Y"))),
            "2021-02-28"
        );
    }

    // ── definite arithmetic (average year/month lengths) ─────────────────────

    #[test]
    fn definite_arithmetic_uses_the_average_lengths() {
        // Average_days_in_month = 30.42, so P1M is 30 days and 10:04:48 — the
        // resulting instant still falls on 14 February.
        assert_eq!(value(date("2020-01-15").add(&dur("P1M"))), "2020-02-14");
        // Average_days_in_year = 365.24.
        assert_eq!(value(date("2019-03-01").add(&dur("P1Y"))), "2020-02-29");
        // Definite day/week/time components are exact.
        assert_eq!(value(date("2020-02-28").add(&dur("P1D"))), "2020-02-29");
        assert_eq!(
            value(date("2020-03-01").subtract(&dur("P1D"))),
            "2020-02-29"
        );
        assert_eq!(value(date("2020-06-15").add(&dur("PT23H"))), "2020-06-15");
        assert_eq!(value(date("2020-06-15").add(&dur("PT24H"))), "2020-06-16");
        assert_eq!(
            value(date("2020-06-15").subtract(&dur("PT1S"))),
            "2020-06-14"
        );
    }

    #[test]
    fn definite_and_nominal_diverge_for_the_same_operands() {
        // §Computational Functions: the definite form treats P1M/P1Y as the
        // average length, the nominal form as the calendar step.
        assert_eq!(value(date("2020-01-15").add(&dur("P1M"))), "2020-02-14");
        assert_eq!(
            value(date("2020-01-15").add_nominal(&dur("P1M"))),
            "2020-02-15"
        );
        assert_eq!(value(date("2019-03-01").add(&dur("P1Y"))), "2020-02-29");
        assert_eq!(
            value(date("2019-03-01").add_nominal(&dur("P1Y"))),
            "2020-03-01"
        );
    }

    // ── diff ─────────────────────────────────────────────────────────────────

    #[test]
    fn diff_reports_whole_days_in_both_directions() {
        assert_eq!(
            duration_value(date("2020-06-15").diff(&date("2020-06-01"))),
            "P14D"
        );
        // The earlier operand first ⇒ the negative-duration deviation.
        assert_eq!(
            duration_value(date("2020-06-01").diff(&date("2020-06-15"))),
            "-P14D"
        );
        assert_eq!(
            duration_value(date("2020-06-15").diff(&date("2020-06-15"))),
            "PT0S"
        );
        // Across a leap February.
        assert_eq!(
            duration_value(date("2020-03-01").diff(&date("2020-02-01"))),
            "P29D"
        );
        assert_eq!(
            duration_value(date("2021-03-01").diff(&date("2021-02-01"))),
            "P28D"
        );
    }

    // ── partial / unrepresentable operands ───────────────────────────────────

    #[test]
    fn partial_and_malformed_values_have_no_arithmetic() {
        assert!(date("2020-06").add(&dur("P1D")).is_none());
        assert!(date("2020").add_nominal(&dur("P1M")).is_none());
        assert!(date("2020-06").subtract(&dur("P1D")).is_none());
        assert!(date("2020-06").diff(&date("2020-06-15")).is_none());
        assert!(date("2020-06-15").diff(&date("2020")).is_none());
        assert!(date("not-a-date").add(&dur("P1D")).is_none());
        // A malformed duration is equally uncomputable.
        assert!(date("2020-06-15").add(&dur("1D")).is_none());
    }

    // ── invariants ───────────────────────────────────────────────────────────

    /// Every invalid value names the `iso8601_date.adoc` §Invariants entry it
    /// breaks — the point of the per-invariant realization: a report says WHICH
    /// rule failed, not merely that the string was rejected.
    #[test]
    fn invalid_dates_name_the_invariant_they_break() {
        for (bad, invariant) in [
            // Month_valid: valid_month (m) = m >= 1 and m <= Months_in_year.
            ("2020-13-01", "Month_valid"),
            ("2020-00-15", "Month_valid"),
            ("202013", "Month_valid"),
            // Day_valid: d >= 1 and d <= days_in_month (m, y).
            ("2021-02-29", "Day_valid"), // 2021 is not a leap year
            ("2020-02-30", "Day_valid"),
            ("2020-04-31", "Day_valid"),
            ("2020-06-00", "Day_valid"),
            ("20210229", "Day_valid"),
            // Our own lexical rule: not the valid_iso8601_date production.
            ("2020-W01", "Value_lexical_form_valid"),
            ("not-a-date", "Value_lexical_form_valid"),
            ("2020-6-15", "Value_lexical_form_valid"),
            ("+2020-06-15", "Value_lexical_form_valid"),
            ("", "Value_lexical_form_valid"),
        ] {
            let v = date(bad).invariants();
            let expected = format!("Invariant {invariant} failed on type Iso8601_date");
            assert!(
                v.iter().any(|m| m.message == expected),
                "{bad:?} should report {invariant}, got {v:?}"
            );
        }
    }

    #[test]
    fn valid_dates_including_every_partial_form_report_nothing() {
        for good in [
            "2020",
            "2020-06",
            "2020-06-15",
            "202006",
            "20200615",
            "2020-02-29",
            "0000-01-01",
            "9999-12-31",
        ] {
            assert!(
                date(good).invariants().is_empty(),
                "{good:?} is a valid Iso8601_date"
            );
        }
    }

    /// `valid_day` is defined in terms of `days_in_month (m, y)`, so a month
    /// `valid_month` already rejects leaves `Day_valid` undecided.
    #[test]
    fn an_out_of_range_month_reports_month_valid_alone() {
        let v = date("2020-13-01").invariants();
        assert_eq!(v.len(), 1);
        assert_eq!(
            v[0].message,
            "Invariant Month_valid failed on type Iso8601_date"
        );
    }

    #[test]
    fn results_outside_the_representable_years_are_none() {
        // valid_year (y >= 0) + master06's 4-digit-year restriction.
        assert!(date("9999-12-31").add(&dur("P1D")).is_none());
        assert!(date("0000-01-01").subtract(&dur("P1D")).is_none());
        assert!(date("9999-12-31").add_nominal(&dur("P1M")).is_none());
    }
}
