// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written `Iso8601_date_time` spec behaviour.
//!
//! Covers the accessor functions, the computational functions (definite
//! `add`/`subtract`/`diff` and nominal `add_nominal`/`subtract_nominal`) and a
//! `PartialOrd` implementing range semantics over partial date/times with
//! timezone normalisation (which may roll the calendar date).
//!
//! Spec sources (vendored):
//! - `BASE/docs/UML/classes/org.openehr.base.foundation_types.iso8601_date_time.adoc`
//!   (§Functions: the accessors, `as_string`, `add`/`subtract`/`diff`,
//!   `add_nominal`/`subtract_nominal`; §Invariants).
//! - `BASE/docs/UML/classes/org.openehr.base.foundation_types.iso8601_date.adoc`
//!   (§Functions `add_nominal`: the day-clamping rules this class refers to).
//! - `BASE/docs/foundation_types/master06-time_types.adoc` (§Primitive Time
//!   Types: partial date/times may omit any part down to the month;
//!   `24:00:00` disallowed; §Computational Functions: the definite/nominal
//!   split).
//!
//! Invariants — the TWELVE rows the class table declares under §Invariants:
//! - `Month_valid`, `Day_valid` (shared with `iso8601_date_impl.rs`) and
//!   `Hour_valid`, `Minute_valid`, `Second_valid`, `Fractional_second_valid`
//!   (shared with `iso8601_time_impl.rs`) — checked, each under its own name.
//!   The first two are read in the GUARDED form the sibling `Iso8601_date`
//!   class spells them (`not month_unknown implies valid_month (month)`); the
//!   unguarded literal form would reject every partial this class's own
//!   Description permits.
//! - `Year_valid` — structurally satisfied (four zero-filled digits).
//! - `Partial_validity_minute` (`minute_unknown implies second_unknown`) —
//!   structurally satisfied: no accepted form writes a second without a minute.
//! - `Partial_validity_year`, `Partial_validity_month`, `Partial_validity_day`,
//!   `Partial_validity_hour` — refused, see the NOTEs below.
//! - `Value_lexical_form_valid` — OUR OWN name, because the class table
//!   declares no rule for a value that is not the production at all (the same
//!   rule `iso8601_date_impl.rs` names).
//!
//! NOTE: `Partial_validity_day`, `Partial_validity_hour` and
//! `Partial_validity_year` are not enforced, because `master06` §Primitive Time
//! Types states that partial variants of this type "can include missing hours,
//! days and months".
//!
//! NOTE: `add_nominal`/`subtract_nominal` are declared returning `Iso8601_date`
//! while `add`/`subtract` return `Iso8601_date_time`, which would discard the
//! time of day §Computational Functions says a nominal addition preserves; read
//! here as a copy-paste defect, so both return `Iso8601_date_time`.
//!
//! NOTE: no openEHR spec governs arithmetic on a PARTIAL date/time, timezone
//! handling under a shift, or ordering — our own design/extension. A partial or
//! unparseable operand yields `None`; the offset is carried through unchanged
//! (canonicalised to the extended form, `Z` for UTC) because a uniform offset
//! is invariant under a shift of the instant and openEHR has no zone-rule
//! database; a partial value denotes the interval of its completions on an
//! absolute-seconds axis, a zoned and an unzoned value are incomparable, and
//! `partial_cmp` returns `Some(Equal)` only for equal raw strings.

use std::cmp::Ordering;

use super::iso8601_date_impl::push_date_component_violations;
use super::iso8601_date_time::Iso8601DateTime;
use super::iso8601_duration::Iso8601Duration;
use super::iso8601_parse::{
    EXACT_SECONDS_IN_DAY, EXACT_SECONDS_IN_HOUR, EXACT_SECONDS_IN_MINUTE, ExactSeconds,
    ParsedDateTime, ParsedTime, as_extended_date_time, civil_from_days, date_time_completion_range,
    days_from_civil, hms_from_seconds_of_day, parse_date_time, range_before, render_date_extended,
    render_duration, render_time_extended, scan_date_time, shift_months,
};
use super::iso8601_time_impl::push_time_component_violations;
use crate::validate::{InvariantViolation, Validate};

impl Iso8601DateTime {
    /// Parsed components, or `None` when `value` is not a valid ISO 8601
    /// date/time.
    fn parsed(&self) -> Option<ParsedDateTime> {
        parse_date_time(&self.value)
    }

    /// The year part (`Iso8601_date_time.year`), or `None` when the value does
    /// not parse.
    #[must_use]
    pub fn year(&self) -> Option<u32> {
        self.parsed().map(|p| p.date.year)
    }

    /// The month part, or `None` when unknown or the value does not parse.
    #[must_use]
    pub fn month(&self) -> Option<u32> {
        self.parsed().and_then(|p| p.date.month)
    }

    /// The day part, or `None` when unknown or the value does not parse.
    #[must_use]
    pub fn day(&self) -> Option<u32> {
        self.parsed().and_then(|p| p.date.day)
    }

    /// The hour part, or `None` when the time part is absent or the value does
    /// not parse.
    #[must_use]
    pub fn hour(&self) -> Option<u32> {
        self.parsed().and_then(|p| p.time.map(|t| t.hour))
    }

    /// The minute part, or `None` when unknown/absent or the value does not
    /// parse.
    #[must_use]
    pub fn minute(&self) -> Option<u32> {
        self.parsed().and_then(|p| p.time.and_then(|t| t.minute))
    }

    /// The integral seconds part, or `None` when unknown/absent or the value
    /// does not parse.
    #[must_use]
    pub fn second(&self) -> Option<u32> {
        self.parsed().and_then(|p| p.time.and_then(|t| t.second))
    }

    /// The fractional seconds part, or `None` when absent or the value does not
    /// parse.
    #[must_use]
    pub fn fractional_second(&self) -> Option<f64> {
        self.parsed()
            .and_then(|p| p.time.and_then(|t| t.fractional_second))
    }

    /// The timezone offset in signed minutes (`Z` → `Some(0)`), or `None` when
    /// unzoned/timeless or the value does not parse.
    #[must_use]
    pub fn timezone(&self) -> Option<i32> {
        self.parsed().and_then(|p| p.time.and_then(|t| t.timezone))
    }

    /// `Iso8601_date_time.month_unknown` (or the value does not parse).
    #[must_use]
    pub fn month_unknown(&self) -> bool {
        self.parsed().is_none_or(|p| p.date.month.is_none())
    }

    /// `Iso8601_date_time.day_unknown` (or the value does not parse).
    #[must_use]
    pub fn day_unknown(&self) -> bool {
        self.parsed().is_none_or(|p| p.date.day.is_none())
    }

    /// `Iso8601_date_time.minute_unknown`: the minute is absent (no time part,
    /// or a time of the form `hh`), or the value does not parse.
    #[must_use]
    pub fn minute_unknown(&self) -> bool {
        self.parsed()
            .is_none_or(|p| p.time.and_then(|t| t.minute).is_none())
    }

    /// `Iso8601_date_time.second_unknown`: the second is absent, or the value
    /// does not parse.
    #[must_use]
    pub fn second_unknown(&self) -> bool {
        self.parsed()
            .is_none_or(|p| p.time.and_then(|t| t.second).is_none())
    }

    /// `Iso8601_date_time.is_partial`: true when seconds or more is missing (a
    /// value that does not parse is treated as not complete).
    #[must_use]
    pub fn is_partial(&self) -> bool {
        self.parsed()
            .is_none_or(|p| p.time.and_then(|t| t.second).is_none())
    }

    /// `Iso8601_date_time.is_extended`: true when the value uses `'-'` and `':'`
    /// separators throughout (see the `is_extended` NOTE in
    /// `iso8601_parse.rs` for the forms that have no separator position). A value
    /// that does not parse is not extended.
    #[must_use]
    pub fn is_extended(&self) -> bool {
        self.parsed()
            .is_some_and(|p| p.date.extended && p.time.is_none_or(|t| t.extended))
    }

    /// `Iso8601_date_time.is_decimal_sign_comma`: true when the fractional second
    /// is written with `','` rather than `'.'`. False when there is no
    /// fractional part or the value does not parse.
    #[must_use]
    pub fn is_decimal_sign_comma(&self) -> bool {
        self.parsed()
            .is_some_and(|p| p.time.is_some_and(|t| t.decimal_sign_comma))
    }

    /// `Iso8601_date_time.has_fractional_second`: true when the
    /// fractional-second part is significant, i.e. the value writes one — "even
    /// if = 0.0". False when the value does not parse.
    #[must_use]
    pub fn has_fractional_second(&self) -> bool {
        self.parsed()
            .is_some_and(|p| p.time.is_some_and(|t| t.fractional_second.is_some()))
    }

    /// `Iso8601_date_time.as_string`: "Return the string value in extended
    /// format" — a compact value is re-spelled with `'-'`/`':'` separators (its
    /// fractional precision, decimal sign and partial precision all preserved),
    /// an already-extended one is returned unchanged.
    ///
    /// NOTE: as for `Iso8601_date.as_string`, a value that is not a valid
    /// ISO 8601 date/time is returned verbatim — our own design/extension.
    #[must_use]
    pub fn as_string(&self) -> String {
        as_extended_date_time(&self.value).unwrap_or_else(|| self.value.clone())
    }

    /// `Iso8601_date_time.add` (alias `'+'`): DEFINITE addition of a duration —
    /// `a_diff` is reduced to an exact number of seconds with the
    /// `Time_definitions` average year/month lengths
    /// (`master06` §Computational Functions) and added to the local reading.
    ///
    /// `None` when either value does not parse, when this date/time is partial,
    /// or when the result leaves the representable `0000`–`9999` year range.
    #[must_use]
    pub fn add(&self, a_diff: &Iso8601Duration) -> Option<Self> {
        self.shifted(a_diff.parsed()?.to_definite_shift(false)?)
    }

    /// `Iso8601_date_time.subtract` (alias `'-'`): DEFINITE subtraction of a
    /// duration. See [`Iso8601DateTime::add`].
    ///
    /// `None` under the same conditions as [`Iso8601DateTime::add`].
    #[must_use]
    pub fn subtract(&self, a_diff: &Iso8601Duration) -> Option<Self> {
        self.shifted(a_diff.parsed()?.to_definite_shift(true)?)
    }

    /// `Iso8601_date_time.diff` (alias `'-'`): the difference
    /// `self - a_date_time` as an `Iso8601_duration` — negative (the openEHR
    /// negative-duration deviation) when `a_date_time` is the later instant. Two
    /// zoned values are compared on the UTC axis; a zoned and an unzoned value
    /// are not comparable at all (`None`), matching this module's ordering rule.
    ///
    /// `None` when either value does not parse or is partial, when exactly one
    /// side is zoned, or on arithmetic overflow.
    #[must_use]
    pub fn diff(&self, a_date_time: &Self) -> Option<Iso8601Duration> {
        let (a, b) = (self.parsed()?, a_date_time.parsed()?);
        if is_zoned(&a) != is_zoned(&b) {
            return None;
        }
        let total = utc_seconds(&a)?.checked_sub(utc_seconds(&b)?)?;
        Some(Iso8601Duration {
            value: render_duration(total)?,
        })
    }

    /// `Iso8601_date_time.add_nominal` (alias `'++'`): NOMINAL addition — years
    /// and months advance the calendar to the same day-of-month at the same time
    /// of day, clamped down when the target month is shorter (the rules of
    /// `Iso8601_date.add_nominal`), and the remaining components (weeks, days,
    /// hours, minutes, seconds) apply as an exact shift.
    ///
    /// `None` when either value does not parse, when this date/time is partial,
    /// or when the result leaves the representable `0000`–`9999` year range.
    #[must_use]
    pub fn add_nominal(&self, a_diff: &Iso8601Duration) -> Option<Self> {
        self.nominal_shift(a_diff, false)
    }

    /// `Iso8601_date_time.subtract_nominal` (alias `'--'`): NOMINAL subtraction,
    /// with the day-clamping semantics of [`Iso8601DateTime::add_nominal`].
    ///
    /// `None` under the same conditions as [`Iso8601DateTime::add_nominal`].
    #[must_use]
    pub fn subtract_nominal(&self, a_diff: &Iso8601Duration) -> Option<Self> {
        self.nominal_shift(a_diff, true)
    }

    /// The date/time reached by shifting this one's local instant by `shift`
    /// seconds, re-rendered in extended form with the timezone carried through.
    fn shifted(&self, shift: ExactSeconds) -> Option<Self> {
        let p = self.parsed()?;
        let total = local_seconds(&p)?.checked_add(shift)?;
        render_absolute(total, p.time.and_then(|t| t.timezone)).map(|value| Self { value })
    }

    /// Shared body of the nominal `add_nominal`/`subtract_nominal`: the
    /// year/month part shifts the calendar with day-clamping, then the
    /// sub-month remainder applies as an exact shift of the local instant.
    fn nominal_shift(&self, a_diff: &Iso8601Duration, subtract: bool) -> Option<Self> {
        let (months, remainder) = a_diff.parsed()?.to_nominal_parts(subtract)?;
        let p = self.parsed()?;
        let time = p.time?;
        let (year, month, day) = shift_months(p.date.year, p.date.month?, p.date.day?, months)?;
        let clamped = local_seconds_at(year, month, day, &time)?;
        render_absolute(clamped.checked_add(remainder)?, time.timezone).map(|value| Self { value })
    }
}

/// A complete date/time's local instant on the absolute-seconds axis (days since
/// 1970-01-01 × 86400 + time of day), or `None` when any component the
/// arithmetic needs is missing.
fn local_seconds(dt: &ParsedDateTime) -> Option<ExactSeconds> {
    let time = dt.time?;
    local_seconds_at(dt.date.year, dt.date.month?, dt.date.day?, &time)
}

/// The local instant of `(year, month, day)` at the time of day `time` (whose
/// minute and second must be present).
fn local_seconds_at(year: u32, month: u32, day: u32, time: &ParsedTime) -> Option<ExactSeconds> {
    let whole = days_from_civil(year, month, day)
        .checked_mul(EXACT_SECONDS_IN_DAY)?
        .checked_add(i64::from(time.hour) * EXACT_SECONDS_IN_HOUR)?
        .checked_add(i64::from(time.minute?) * EXACT_SECONDS_IN_MINUTE)?
        .checked_add(i64::from(time.second?))?;
    ExactSeconds::new(whole, time.fractional_second.unwrap_or(0.0))
}

/// A complete date/time's instant on the UTC axis when zoned (the same uniform
/// shift the ordering uses), or its local instant when unzoned.
fn utc_seconds(dt: &ParsedDateTime) -> Option<ExactSeconds> {
    let offset = i64::from(dt.time.and_then(|t| t.timezone).unwrap_or(0))
        .checked_mul(EXACT_SECONDS_IN_MINUTE)?;
    local_seconds(dt)?.checked_sub(ExactSeconds::new(offset, 0.0)?)
}

/// Render an absolute-seconds instant as an extended-form date/time value with
/// `timezone` carried through. `None` when the instant falls outside the
/// representable year range.
fn render_absolute(instant: ExactSeconds, timezone: Option<i32>) -> Option<String> {
    let instant = instant.rounded_to_nanos()?;
    let (year, month, day) = civil_from_days(instant.whole.div_euclid(EXACT_SECONDS_IN_DAY))?;
    let (hour, minute, second) =
        hms_from_seconds_of_day(instant.whole.rem_euclid(EXACT_SECONDS_IN_DAY))?;
    Some(format!(
        "{}T{}",
        render_date_extended(year, Some(month), Some(day)),
        render_time_extended(hour, minute, second, instant.frac, timezone)
    ))
}

/// True when the value carries a timezone (a time part with an offset).
fn is_zoned(dt: &ParsedDateTime) -> bool {
    dt.time.and_then(|t| t.timezone).is_some()
}

/// Range-semantics comparison of two parsed date/times. `None` unless both are
/// zoned or both unzoned. Never returns `Some(Equal)` — equal strings are
/// handled before parsing.
fn cmp_date_time(a: &ParsedDateTime, b: &ParsedDateTime) -> Option<Ordering> {
    if is_zoned(a) != is_zoned(b) {
        return None;
    }
    let ra = date_time_completion_range(a);
    let rb = date_time_completion_range(b);
    if range_before(&ra, &rb) {
        Some(Ordering::Less)
    } else if range_before(&rb, &ra) {
        Some(Ordering::Greater)
    } else {
        None
    }
}

impl PartialOrd for Iso8601DateTime {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        if self.value == other.value {
            return Some(Ordering::Equal); // consistent with the derived PartialEq
        }
        cmp_date_time(&self.parsed()?, &other.parsed()?)
    }
}

impl Validate for Iso8601DateTime {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        let Some(dt) = scan_date_time(&self.value) else {
            out.push(InvariantViolation::here(
                "Invariant Value_lexical_form_valid failed on type Iso8601_date_time",
            ));
            return;
        };
        push_date_component_violations(&dt.date, "Iso8601_date_time", out);
        if let Some(time) = dt.time {
            push_time_component_violations(&time, "Iso8601_date_time", out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(v: &str) -> Iso8601DateTime {
        Iso8601DateTime {
            value: v.to_owned(),
        }
    }

    fn dur(v: &str) -> Iso8601Duration {
        Iso8601Duration {
            value: v.to_owned(),
        }
    }

    /// The value of a computed date/time, or `"None"`.
    fn value(d: Option<Iso8601DateTime>) -> String {
        d.map_or_else(|| "None".to_owned(), |d| d.value)
    }

    /// The value of a computed duration, or `"None"`.
    fn duration_value(d: Option<Iso8601Duration>) -> String {
        d.map_or_else(|| "None".to_owned(), |d| d.value)
    }

    // ── full-vs-full ordering ────────────────────────────────────────────────

    #[test]
    fn full_date_times_order_by_instant() {
        assert_eq!(
            dt("2020-06-15T09:00:00").partial_cmp(&dt("2020-06-15T17:30:00")),
            Some(Ordering::Less)
        );
        assert_eq!(
            dt("2020-06-15T00:00:00").partial_cmp(&dt("2020-06-14T23:59:59")),
            Some(Ordering::Greater)
        );
        assert_eq!(
            dt("2020-06-15T12:00:00").partial_cmp(&dt("2020-06-15T12:00:00")),
            Some(Ordering::Equal)
        );
    }

    #[test]
    fn fractional_seconds_break_ties() {
        assert_eq!(
            dt("2020-06-15T12:00:00.25").partial_cmp(&dt("2020-06-15T12:00:00.75")),
            Some(Ordering::Less)
        );
    }

    // ── partial range semantics ──────────────────────────────────────────────

    #[test]
    fn partial_before_separated_full() {
        // 2020-06 spans June, entirely before July 1st.
        assert_eq!(
            dt("2020-06").partial_cmp(&dt("2020-07-01T00:00:00")),
            Some(Ordering::Less)
        );
        // The year 2019 is entirely before any 2020 instant.
        assert_eq!(
            dt("2019").partial_cmp(&dt("2020-01-01T00:00:00")),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn overlapping_partials_are_incomparable() {
        assert_eq!(dt("2020-06").partial_cmp(&dt("2020-06-15T12:00:00")), None);
        assert_eq!(
            dt("2020-06-15T12").partial_cmp(&dt("2020-06-15T12:30:00")),
            None
        );
    }

    #[test]
    fn missing_time_spans_the_day() {
        // The bare date 2020-06-15 spans the whole day, overlapping noon.
        assert_eq!(
            dt("2020-06-15").partial_cmp(&dt("2020-06-15T12:00:00")),
            None
        );
        // ... and is entirely before the next day.
        assert_eq!(
            dt("2020-06-15").partial_cmp(&dt("2020-06-16T00:00:00")),
            Some(Ordering::Less)
        );
    }

    // ── timezone matrix (crossing day / month / year boundaries) ─────────────

    #[test]
    fn both_zoned_normalise_to_utc() {
        // 2020-06-15T12:00Z vs 2020-06-15T13:00+02:00 (= 11:00 UTC).
        assert_eq!(
            dt("2020-06-15T12:00:00Z").partial_cmp(&dt("2020-06-15T13:00:00+02:00")),
            Some(Ordering::Greater)
        );
    }

    #[test]
    fn timezone_shift_rolls_across_midnight() {
        // 2020-06-15T23:00-02:00 = 2020-06-16T01:00 UTC, later than 2020-06-16T00:30Z.
        assert_eq!(
            dt("2020-06-15T23:00:00-02:00").partial_cmp(&dt("2020-06-16T00:30:00Z")),
            Some(Ordering::Greater)
        );
    }

    #[test]
    fn timezone_shift_rolls_across_year_boundary() {
        // 2019-12-31T23:00-02:00 = 2020-01-01T01:00 UTC, later than 2020-01-01T00:00Z.
        assert_eq!(
            dt("2019-12-31T23:00:00-02:00").partial_cmp(&dt("2020-01-01T00:00:00Z")),
            Some(Ordering::Greater)
        );
    }

    #[test]
    fn zoned_vs_unzoned_is_incomparable() {
        assert_eq!(
            dt("2020-06-15T12:00:00Z").partial_cmp(&dt("2020-06-15T12:00:00")),
            None
        );
    }

    // ── mixed / malformed ────────────────────────────────────────────────────

    #[test]
    fn compact_vs_extended_same_instant_is_incomparable() {
        assert_eq!(
            dt("20200615T120000").partial_cmp(&dt("2020-06-15T12:00:00")),
            None
        );
    }

    #[test]
    fn malformed_values_are_incomparable() {
        assert_eq!(dt("garbage").partial_cmp(&dt("2020-06-15T12:00:00")), None);
        assert_eq!(
            dt("2020-06-15T24:00:00").partial_cmp(&dt("2020-06-16T00:00:00")),
            None
        );
    }

    // ── accessors ────────────────────────────────────────────────────────────

    #[test]
    fn accessors_report_components_and_unknowns() {
        let full = dt("2020-06-15T13:45:30.5+02:00");
        assert_eq!(full.year(), Some(2020));
        assert_eq!(full.month(), Some(6));
        assert_eq!(full.day(), Some(15));
        assert_eq!(full.hour(), Some(13));
        assert_eq!(full.minute(), Some(45));
        assert_eq!(full.second(), Some(30));
        assert_eq!(full.fractional_second(), Some(0.5));
        assert_eq!(full.timezone(), Some(120));
        assert!(!full.is_partial());

        let date_only = dt("2020-06-15");
        assert_eq!(date_only.hour(), None);
        assert!(date_only.minute_unknown());
        assert!(date_only.second_unknown());
        assert!(date_only.is_partial());
        assert_eq!(date_only.timezone(), None);
    }

    // ── lexical predicates / as_string ───────────────────────────────────────

    #[test]
    fn extended_and_compact_forms_are_distinguished() {
        assert!(dt("2020-06-15T12:00:00").is_extended());
        assert!(dt("2020-06-15T12:00:00Z").is_extended());
        assert!(dt("2020-06-15").is_extended());
        assert!(dt("2020-06").is_extended());
        assert!(!dt("20200615T120000").is_extended());
        assert!(!dt("2020-06-15T120000").is_extended()); // mixed spelling
        assert!(!dt("20200615T12:00:00").is_extended());
        assert!(!dt("2020-06-15T12:00:00+0200").is_extended()); // compact timezone
        assert!(!dt("garbage").is_extended());
    }

    #[test]
    fn decimal_sign_and_fractional_second_are_reported() {
        assert!(dt("2020-06-15T12:00:00,5").is_decimal_sign_comma());
        assert!(!dt("2020-06-15T12:00:00.5").is_decimal_sign_comma());
        assert!(!dt("2020-06-15T12:00:00").is_decimal_sign_comma());
        assert!(!dt("2020-06-15").is_decimal_sign_comma());

        assert!(dt("2020-06-15T12:00:00.0").has_fractional_second());
        assert!(dt("2020-06-15T12:00:00,25Z").has_fractional_second());
        assert!(!dt("2020-06-15T12:00:00").has_fractional_second());
        assert!(!dt("2020-06-15").has_fractional_second());
        assert!(!dt("garbage").has_fractional_second());
    }

    #[test]
    fn as_string_returns_the_extended_form() {
        assert_eq!(dt("20200615T120000").as_string(), "2020-06-15T12:00:00");
        assert_eq!(dt("20200615T1200").as_string(), "2020-06-15T12:00");
        assert_eq!(dt("202006").as_string(), "2020-06");
        assert_eq!(dt("2020-06-15T12:00:00").as_string(), "2020-06-15T12:00:00");
        // Precision, decimal sign and timezone survive the re-spelling.
        assert_eq!(
            dt("20200615T120000,500+0200").as_string(),
            "2020-06-15T12:00:00,500+02:00"
        );
        // An invalid date/time has no extended form: verbatim.
        assert_eq!(dt("2020-06-15T24:00:00").as_string(), "2020-06-15T24:00:00");
    }

    // ── definite arithmetic ──────────────────────────────────────────────────

    #[test]
    fn definite_arithmetic_rolls_the_calendar() {
        assert_eq!(
            value(dt("2020-06-15T23:30:00").add(&dur("PT1H"))),
            "2020-06-16T00:30:00"
        );
        assert_eq!(
            value(dt("2020-06-15T00:30:00").subtract(&dur("PT1H"))),
            "2020-06-14T23:30:00"
        );
        assert_eq!(
            value(dt("2019-12-31T23:59:59").add(&dur("PT1S"))),
            "2020-01-01T00:00:00"
        );
        assert_eq!(
            value(dt("2020-02-28T12:00:00").add(&dur("P1D"))),
            "2020-02-29T12:00:00"
        );
    }

    #[test]
    fn definite_month_and_year_use_the_average_lengths() {
        // Average_days_in_month = 30.42 ⇒ 30 days and 10:04:48.
        assert_eq!(
            value(dt("2020-01-15T00:00:00").add(&dur("P1M"))),
            "2020-02-14T10:04:48"
        );
        // Average_days_in_year = 365.24 ⇒ 365 days and 05:45:36.
        assert_eq!(
            value(dt("2019-03-01T00:00:00").add(&dur("P1Y"))),
            "2020-02-29T05:45:36"
        );
    }

    #[test]
    fn definite_arithmetic_keeps_fractions_and_the_timezone() {
        assert_eq!(
            value(dt("2020-06-15T12:00:00.5").add(&dur("PT0.25S"))),
            "2020-06-15T12:00:00.75"
        );
        assert_eq!(
            value(dt("2020-06-15T12:00:00+02:00").add(&dur("PT30M"))),
            "2020-06-15T12:30:00+02:00"
        );
        // A compact value's timezone is canonicalised to the extended spelling.
        assert_eq!(
            value(dt("20200615T120000+0200").add(&dur("PT30M"))),
            "2020-06-15T12:30:00+02:00"
        );
        assert_eq!(
            value(dt("2020-06-15T12:00:00Z").subtract(&dur("PT1H"))),
            "2020-06-15T11:00:00Z"
        );
    }

    // ── nominal arithmetic ───────────────────────────────────────────────────

    #[test]
    fn nominal_year_on_leap_day_clamps_and_keeps_the_time() {
        // Iso8601_date.add_nominal's rule, with the time of day preserved
        // (§Computational Functions: "the same time on the next or previous day").
        assert_eq!(
            value(dt("2020-02-29T12:00:00Z").add_nominal(&dur("P1Y"))),
            "2021-02-28T12:00:00Z"
        );
        assert_eq!(
            value(dt("2020-02-29T08:15:30.5").subtract_nominal(&dur("P1Y"))),
            "2019-02-28T08:15:30.5"
        );
    }

    #[test]
    fn nominal_month_clamps_into_a_shorter_month() {
        assert_eq!(
            value(dt("2020-01-31T08:15:00").add_nominal(&dur("P1M"))),
            "2020-02-29T08:15:00"
        );
        assert_eq!(
            value(dt("2021-01-31T08:15:00").add_nominal(&dur("P1M"))),
            "2021-02-28T08:15:00"
        );
        assert_eq!(
            value(dt("2020-03-31T08:15:00").subtract_nominal(&dur("P1M"))),
            "2020-02-29T08:15:00"
        );
    }

    #[test]
    fn nominal_time_components_shift_exactly() {
        // P1DT2H nominally: the next day, two hours later.
        assert_eq!(
            value(dt("2020-06-15T22:00:00").add_nominal(&dur("P1DT2H"))),
            "2020-06-17T00:00:00"
        );
        assert_eq!(
            value(dt("2020-01-31T12:00:00").add_nominal(&dur("P1M1DT30M"))),
            "2020-03-01T12:30:00"
        );
    }

    #[test]
    fn definite_and_nominal_diverge_for_the_same_operands() {
        assert_eq!(
            value(dt("2020-01-15T00:00:00").add(&dur("P1M"))),
            "2020-02-14T10:04:48"
        );
        assert_eq!(
            value(dt("2020-01-15T00:00:00").add_nominal(&dur("P1M"))),
            "2020-02-15T00:00:00"
        );
    }

    // ── diff ─────────────────────────────────────────────────────────────────

    #[test]
    fn diff_reports_the_signed_interval() {
        assert_eq!(
            duration_value(dt("2020-06-15T12:00:00").diff(&dt("2020-06-14T12:00:00"))),
            "P1D"
        );
        assert_eq!(
            duration_value(dt("2020-06-14T12:00:00").diff(&dt("2020-06-15T12:00:00"))),
            "-P1D"
        );
        assert_eq!(
            duration_value(dt("2020-06-15T12:00:00").diff(&dt("2020-06-15T12:00:00"))),
            "PT0S"
        );
        assert_eq!(
            duration_value(dt("2020-06-15T12:00:01.5").diff(&dt("2020-06-15T12:00:00"))),
            "PT1.5S"
        );
        assert_eq!(
            duration_value(dt("2020-03-01T00:00:00").diff(&dt("2020-02-01T00:00:00"))),
            "P29D"
        );
    }

    #[test]
    fn diff_normalises_two_zoned_values_and_refuses_a_mixed_pair() {
        // 2020-06-15T12:00Z vs 2020-06-15T13:00+02:00 (= 11:00 UTC).
        assert_eq!(
            duration_value(dt("2020-06-15T12:00:00Z").diff(&dt("2020-06-15T13:00:00+02:00"))),
            "PT1H"
        );
        assert!(
            dt("2020-06-15T12:00:00Z")
                .diff(&dt("2020-06-15T12:00:00"))
                .is_none()
        );
    }

    // ── partial / malformed operands ─────────────────────────────────────────

    #[test]
    fn partial_and_malformed_values_have_no_arithmetic() {
        assert!(dt("2020-06-15").add(&dur("PT1H")).is_none()); // no time part
        assert!(dt("2020-06-15T12:00").add(&dur("PT1H")).is_none()); // no second
        assert!(dt("2020-06").add_nominal(&dur("P1M")).is_none());
        assert!(
            dt("2020-06-15T12:00")
                .diff(&dt("2020-06-15T12:00:00"))
                .is_none()
        );
        assert!(dt("garbage").add(&dur("PT1H")).is_none());
        assert!(dt("2020-06-15T12:00:00").add(&dur("1H")).is_none());
    }

    // ── invariants ───────────────────────────────────────────────────────────

    /// Every invalid value names the `iso8601_date_time.adoc` §Invariants entry
    /// it breaks, on this class rather than on the date/time class the rule is
    /// shared with.
    #[test]
    fn invalid_date_times_name_the_invariant_they_break() {
        for (bad, invariant) in [
            ("2020-13-01T10:00", "Month_valid"),
            ("2021-02-29T10:00:00", "Day_valid"),
            ("2020-06-15T24:00:00", "Hour_valid"),
            ("2020-06-15T12:60", "Minute_valid"),
            ("2020-06-15T12:00:60", "Second_valid"),
            ("2020-06-15T12:00.5", "Fractional_second_valid"),
            ("2020-06-15T", "Value_lexical_form_valid"),
            ("garbage", "Value_lexical_form_valid"),
            ("2020-06-15T12:00:00+15:00", "Value_lexical_form_valid"),
        ] {
            let v = dt(bad).invariants();
            let expected = format!("Invariant {invariant} failed on type Iso8601_date_time");
            assert!(
                v.iter().any(|m| m.message == expected),
                "{bad:?} should report {invariant}, got {v:?}"
            );
        }
    }

    /// The partials `master06` §Primitive Time Types permits — "missing hours,
    /// days and months" — validate clean, which is exactly what the refused
    /// `Partial_validity_day`/`_hour` clauses would have rejected.
    #[test]
    fn valid_date_times_including_every_partial_form_report_nothing() {
        for good in [
            "2020",
            "2020-06",
            "2020-06-15",
            "2020-06-15T10",
            "2020-06-15T10:30",
            "2020-06-15T10:30:00",
            "2020-06-15T10:30:00.25Z",
            "20200615T103000+0200",
            "2020-02-29T00:00:00",
        ] {
            assert!(
                dt(good).invariants().is_empty(),
                "{good:?} is a valid Iso8601_date_time"
            );
        }
    }

    #[test]
    fn results_outside_the_representable_years_are_none() {
        assert!(dt("9999-12-31T23:00:00").add(&dur("PT2H")).is_none());
        assert!(dt("0000-01-01T00:00:00").subtract(&dur("PT1S")).is_none());
        assert!(dt("9999-12-31T00:00:00").add_nominal(&dur("P1M")).is_none());
    }
}
