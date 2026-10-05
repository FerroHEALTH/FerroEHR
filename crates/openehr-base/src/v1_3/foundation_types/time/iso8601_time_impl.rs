// @generated-from-template templates/openehr-base/foundation_types/time/iso8601_time_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written `Iso8601_time` spec behaviour.
//!
//! Covers the accessor functions (`is_partial`, `is_extended`,
//! `is_decimal_sign_comma`, `has_fractional_second`, `minute_unknown`,
//! `second_unknown`, `hour`/`minute`/`second`/`fractional_second`/`timezone`,
//! `as_string`), the computational functions (`add`/`subtract`/`diff`) and a
//! `PartialOrd` implementing range semantics over partial times with timezone
//! normalisation.
//!
//! Spec sources (vendored):
//! - `BASE/docs/UML/classes/org.openehr.base.foundation_types.iso8601_time.adoc`
//!   (§Functions: the accessors, `as_string`, `add`/`subtract`/`diff`;
//!   §Invariants).
//! - `BASE/docs/UML/classes/org.openehr.base.foundation_types.time_definitions.adoc`
//!   (§Functions `valid_second`, `valid_hour`).
//! - `BASE/docs/foundation_types/master06-time_types.adoc` (§Primitive Time
//!   Types: `24:00:00` disallowed anywhere; §Computational Functions: only the
//!   definite forms exist for a time — 'year' and 'month' cannot land on a clock
//!   face, so `Iso8601_time` declares no `add_nominal`).
//!
//! Invariants — the FIVE entries the class table declares under §Invariants,
//! plus one rule of our own naming:
//! - `Hour_valid`, `Minute_valid`, `Second_valid` and
//!   `Fractional_second_valid` — checked, each under its own name (shared with
//!   `Iso8601_date_time`, which re-declares all four verbatim).
//! - `Partial_validity` (`minute_unknown implies second_unknown`) —
//!   structurally satisfied: no accepted form writes a second without a minute.
//! - `Value_lexical_form_valid` — OUR OWN name, because the class table
//!   declares none: a value that is not the `valid_iso8601_time` production at
//!   all has no components, so every declared invariant holds vacuously.
//!
//! NOTE: `Hour_valid` is `valid_hour (hour, minute, second)`, whose
//! postcondition also admits `h = Hours_in_day and m = 0 and s = 0`; this class
//! and `master06` §Primitive Time Types both forbid `24:00:00` "anywhere", so
//! the invariant is enforced as `hour < 24`.
//!
//! NOTE: an `Iso8601_time` carries its timezone only as a lexeme, so an
//! out-of-range offset is reported as a lexical-form failure rather than under
//! one of `Iso8601_timezone`'s own invariants.
//!
//! NOTE: `Time_definitions.valid_iso8601_time` prose allows a `"60"` seconds
//! field while the machine-checkable `valid_second` post-condition is
//! `s < Seconds_in_minute` and `Second_valid` is stated in terms of it — an
//! internal contradiction, resolved toward the invariant, so a `:60` second
//! does not parse.
//!
//! NOTE: no openEHR spec governs arithmetic on a partial time, the 24-hour
//! overflow, or ordering — our own design/extension. A partial or unparseable
//! operand yields `None`; addition wraps modulo 24 h and a difference is the
//! signed clock distance in `(-24 h, +24 h)`, the only reading available
//! without inventing a date; a partial time denotes the interval of its
//! completions, a zoned and an unzoned time are incomparable, and `partial_cmp`
//! returns `Some(Equal)` only for equal raw strings.

use std::cmp::Ordering;

use super::iso8601_duration::Iso8601Duration;
use super::iso8601_parse::{
    EXACT_SECONDS_IN_DAY, EXACT_SECONDS_IN_HOUR, EXACT_SECONDS_IN_MINUTE, ExactSeconds, ParsedTime,
    as_extended_time, hms_from_seconds_of_day, parse_time, range_before, render_duration,
    render_time_extended, scan_time, time_completion_range,
};
use super::iso8601_time::Iso8601Time;
use crate::validate::{InvariantViolation, Validate};

impl Iso8601Time {
    /// Parsed components, or `None` when `value` is not a valid ISO 8601 time.
    fn parsed(&self) -> Option<ParsedTime> {
        parse_time(&self.value)
    }

    /// The hour part (`Iso8601_time.hour`), or `None` when the value does not
    /// parse. Hour is always present in a valid time.
    #[must_use]
    pub fn hour(&self) -> Option<u32> {
        self.parsed().map(|p| p.hour)
    }

    /// The minute part (`Iso8601_time.minute`), or `None` when minute is
    /// unknown or the value does not parse.
    #[must_use]
    pub fn minute(&self) -> Option<u32> {
        self.parsed().and_then(|p| p.minute)
    }

    /// The integral seconds part (`Iso8601_time.second`), or `None` when second
    /// is unknown or the value does not parse.
    #[must_use]
    pub fn second(&self) -> Option<u32> {
        self.parsed().and_then(|p| p.second)
    }

    /// The fractional seconds part (`Iso8601_time.fractional_second`), or `None`
    /// when absent/not significant or the value does not parse.
    #[must_use]
    pub fn fractional_second(&self) -> Option<f64> {
        self.parsed().and_then(|p| p.fractional_second)
    }

    /// The timezone offset in signed minutes (`Iso8601_time.timezone` reduced to
    /// its offset; `Z` → `Some(0)`), or `None` when the value is unzoned or does
    /// not parse.
    #[must_use]
    pub fn timezone(&self) -> Option<i32> {
        self.parsed().and_then(|p| p.timezone)
    }

    /// `Iso8601_time.minute_unknown`: true when the value is of the form `hh`
    /// (or does not parse).
    #[must_use]
    pub fn minute_unknown(&self) -> bool {
        self.parsed().is_none_or(|p| p.minute.is_none())
    }

    /// `Iso8601_time.second_unknown`: true when the value omits the second (or
    /// does not parse).
    #[must_use]
    pub fn second_unknown(&self) -> bool {
        self.parsed().is_none_or(|p| p.second.is_none())
    }

    /// `Iso8601_time.is_partial`: true when seconds or more is missing (a value
    /// that does not parse is treated as not a complete time).
    #[must_use]
    pub fn is_partial(&self) -> bool {
        self.parsed().is_none_or(|p| p.second.is_none())
    }

    /// `Iso8601_time.is_extended`: true when the value uses `':'` separators
    /// (and, for forms with no separator position — `hh`, and a `Z`/`±hh`
    /// timezone — always; see the `is_extended` NOTE in `iso8601_parse.rs`). A
    /// value that does not parse is not extended.
    #[must_use]
    pub fn is_extended(&self) -> bool {
        self.parsed().is_some_and(|p| p.extended)
    }

    /// `Iso8601_time.is_decimal_sign_comma`: true when the fractional second is
    /// written with `','` rather than `'.'`. False when the value has no
    /// fractional part or does not parse.
    #[must_use]
    pub fn is_decimal_sign_comma(&self) -> bool {
        self.parsed().is_some_and(|p| p.decimal_sign_comma)
    }

    /// `Iso8601_time.has_fractional_second`: true when the fractional-second
    /// part is significant, i.e. the value writes one — "even if = 0.0", so
    /// `12:00:00.0` qualifies. False when the value does not parse.
    #[must_use]
    pub fn has_fractional_second(&self) -> bool {
        self.parsed().is_some_and(|p| p.fractional_second.is_some())
    }

    /// `Iso8601_time.as_string`: "Return string value in extended format" — a
    /// compact value is re-spelled with `':'` separators (its fractional
    /// precision, decimal sign and partial precision all preserved), an
    /// already-extended one is returned unchanged.
    ///
    /// NOTE: as for `Iso8601_date.as_string`, a value that is not a valid
    /// ISO 8601 time is returned verbatim — our own design/extension.
    #[must_use]
    pub fn as_string(&self) -> String {
        as_extended_time(&self.value).unwrap_or_else(|| self.value.clone())
    }

    /// `Iso8601_time.add` (alias `'+'`): DEFINITE addition of a duration —
    /// `a_diff` is reduced to an exact number of seconds
    /// (`master06` §Computational Functions) and added to the clock reading,
    /// wrapping modulo 24 h. The timezone offset is carried through unchanged.
    ///
    /// `None` when either value does not parse, when this time is partial, or on
    /// arithmetic overflow.
    #[must_use]
    pub fn add(&self, a_diff: &Iso8601Duration) -> Option<Self> {
        self.shifted(a_diff.parsed()?.to_definite_shift(false)?)
    }

    /// `Iso8601_time.subtract` (alias `'-'`): DEFINITE subtraction of a
    /// duration. See [`Iso8601Time::add`].
    ///
    /// `None` under the same conditions as [`Iso8601Time::add`].
    #[must_use]
    pub fn subtract(&self, a_diff: &Iso8601Duration) -> Option<Self> {
        self.shifted(a_diff.parsed()?.to_definite_shift(true)?)
    }

    /// `Iso8601_time.diff` (alias `'-'`): the difference `self - a_time` as an
    /// `Iso8601_duration` — negative (the openEHR negative-duration deviation)
    /// when `a_time` is the later clock reading. Two zoned times are compared on
    /// the UTC axis; a zoned and an unzoned time are not comparable at all
    /// (`None`), matching this module's ordering rule.
    ///
    /// `None` when either value does not parse or is partial, when exactly one
    /// side is zoned, or on arithmetic overflow.
    #[must_use]
    pub fn diff(&self, a_time: &Self) -> Option<Iso8601Duration> {
        let (a, b) = (self.parsed()?, a_time.parsed()?);
        if a.timezone.is_some() != b.timezone.is_some() {
            return None;
        }
        let total = utc_seconds_of_day(&a)?.checked_sub(utc_seconds_of_day(&b)?)?;
        Some(Iso8601Duration {
            value: render_duration(total)?,
        })
    }

    /// The clock reading shifted by `shift` seconds, wrapped into a single day
    /// and re-rendered in extended form with the original timezone.
    fn shifted(&self, shift: ExactSeconds) -> Option<Self> {
        let p = self.parsed()?;
        let total = local_seconds_of_day(&p)?
            .checked_add(shift)?
            .rounded_to_nanos()?;
        let (hour, minute, second) =
            hms_from_seconds_of_day(total.whole.rem_euclid(EXACT_SECONDS_IN_DAY))?;
        Some(Self {
            value: render_time_extended(hour, minute, second, total.frac, p.timezone),
        })
    }
}

/// A complete time's seconds since local midnight, or `None` when the time is
/// partial (arithmetic needs a complete value).
fn local_seconds_of_day(t: &ParsedTime) -> Option<ExactSeconds> {
    let whole = i64::from(t.hour) * EXACT_SECONDS_IN_HOUR
        + i64::from(t.minute?) * EXACT_SECONDS_IN_MINUTE
        + i64::from(t.second?);
    ExactSeconds::new(whole, t.fractional_second.unwrap_or(0.0))
}

/// A complete time's seconds since midnight on the UTC axis when zoned (the same
/// uniform shift the ordering uses), or since local midnight when unzoned. May
/// fall outside `[0, 86400)`, which is correct for a difference.
fn utc_seconds_of_day(t: &ParsedTime) -> Option<ExactSeconds> {
    let local = local_seconds_of_day(t)?;
    let offset = i64::from(t.timezone.unwrap_or(0)).checked_mul(EXACT_SECONDS_IN_MINUTE)?;
    local.checked_sub(ExactSeconds::new(offset, 0.0)?)
}

/// Range-semantics comparison of two parsed times. `None` unless both are
/// zoned or both unzoned (a local time cannot be ordered against an absolute
/// one). Never returns `Some(Equal)` — equal strings are handled before parsing.
fn cmp_time(a: &ParsedTime, b: &ParsedTime) -> Option<Ordering> {
    match (a.timezone, b.timezone) {
        (Some(_), None) | (None, Some(_)) => return None,
        _ => {}
    }
    let ra = time_completion_range(a);
    let rb = time_completion_range(b);
    if range_before(&ra, &rb) {
        Some(Ordering::Less)
    } else if range_before(&rb, &ra) {
        Some(Ordering::Greater)
    } else {
        None
    }
}

impl PartialOrd for Iso8601Time {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        if self.value == other.value {
            return Some(Ordering::Equal); // consistent with the derived PartialEq
        }
        cmp_time(&self.parsed()?, &other.parsed()?)
    }
}

/// The uniform violation for one named invariant, on the class reporting it —
/// `Iso8601_time`, or `Iso8601_date_time` where it re-declares the same rule.
fn failed(invariant: &str, type_name: &str) -> InvariantViolation {
    InvariantViolation::here(format!("Invariant {invariant} failed on type {type_name}"))
}

/// Appends the time-component invariant violations of a scanned value.
///
/// The rules are `Hour_valid`, `Minute_valid`, `Second_valid` and
/// `Fractional_second_valid`, reported on `type_name`. `Iso8601_date_time`
/// declares the same four under the same names with the same expressions, so
/// both classes report through this one body.
pub(crate) fn push_time_component_violations(
    t: &ParsedTime,
    type_name: &str,
    out: &mut Vec<InvariantViolation>,
) {
    // Hour_valid — `24:00:00` is forbidden anywhere (module NOTE).
    if t.hour >= 24 {
        out.push(failed("Hour_valid", type_name));
    }
    // Minute_valid / Second_valid: `valid_minute (m)` is `m < Minutes_in_hour`
    // and `valid_second (s)` is `s < Seconds_in_minute`, each on a present
    // component (the implication is vacuous on an absent one).
    if t.minute.is_some_and(|m| m >= 60) {
        out.push(failed("Minute_valid", type_name));
    }
    if t.second.is_some_and(|s| s >= 60) {
        out.push(failed("Second_valid", type_name));
    }
    // Fractional_second_valid: significant only alongside a present second, and
    // `valid_fractional_second (fs)` is `fs >= 0.0 and fs < 1.0`.
    if let Some(f) = t.fractional_second
        && (t.second.is_none() || !(0.0..1.0).contains(&f))
    {
        out.push(failed("Fractional_second_valid", type_name));
    }
}

impl Validate for Iso8601Time {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        let Some(t) = scan_time(&self.value) else {
            out.push(failed("Value_lexical_form_valid", "Iso8601_time"));
            return;
        };
        push_time_component_violations(&t, "Iso8601_time", out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn time(v: &str) -> Iso8601Time {
        Iso8601Time {
            value: v.to_owned(),
        }
    }

    fn dur(v: &str) -> Iso8601Duration {
        Iso8601Duration {
            value: v.to_owned(),
        }
    }

    /// The value of a computed time, or `"None"`.
    fn value(t: Option<Iso8601Time>) -> String {
        t.map_or_else(|| "None".to_owned(), |t| t.value)
    }

    /// The value of a computed duration, or `"None"`.
    fn duration_value(d: Option<Iso8601Duration>) -> String {
        d.map_or_else(|| "None".to_owned(), |d| d.value)
    }

    // ── full-vs-full ordering (incl. fractional seconds) ─────────────────────

    #[test]
    fn full_times_order_by_instant() {
        assert_eq!(
            time("09:00:00").partial_cmp(&time("17:30:00")),
            Some(Ordering::Less)
        );
        assert_eq!(
            time("23:59:59").partial_cmp(&time("00:00:01")),
            Some(Ordering::Greater)
        );
        assert_eq!(
            time("12:00:00").partial_cmp(&time("12:00:00")),
            Some(Ordering::Equal)
        );
    }

    #[test]
    fn fractional_seconds_break_ties() {
        assert_eq!(
            time("12:00:00.250").partial_cmp(&time("12:00:00.750")),
            Some(Ordering::Less)
        );
        assert_eq!(
            time("12:00:00.9").partial_cmp(&time("12:00:01")),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn same_fraction_written_differently_is_incomparable() {
        // Equal magnitude, different strings ⇒ None (decision 4).
        assert_eq!(time("12:00:00.5").partial_cmp(&time("12:00:00.50")), None);
        // ':00' seconds is a precise instant, distinct string from a fractional form.
        assert_eq!(time("12:00:00").partial_cmp(&time("12:00:00.000")), None);
    }

    // ── partial range semantics ──────────────────────────────────────────────

    #[test]
    fn partial_hour_before_later_full_time() {
        // 09 spans 09:00:00..10:00:00, entirely before 10:30.
        assert_eq!(
            time("09").partial_cmp(&time("10:30:00")),
            Some(Ordering::Less)
        );
        assert_eq!(
            time("11").partial_cmp(&time("10:30:00")),
            Some(Ordering::Greater)
        );
    }

    #[test]
    fn overlapping_partial_times_are_incomparable() {
        // 09 spans the whole hour, overlapping 09:30.
        assert_eq!(time("09").partial_cmp(&time("09:30")), None);
        assert_eq!(time("09:30").partial_cmp(&time("09:30:15")), None);
    }

    #[test]
    fn equal_precision_partials_order() {
        assert_eq!(
            time("09:15").partial_cmp(&time("09:16")),
            Some(Ordering::Less)
        );
        assert_eq!(time("10").partial_cmp(&time("09")), Some(Ordering::Greater));
    }

    // ── timezone matrix ──────────────────────────────────────────────────────

    #[test]
    fn both_zoned_normalise_to_utc() {
        // 12:00Z vs 13:00+02:00 (= 11:00 UTC) ⇒ 12:00Z is later.
        assert_eq!(
            time("12:00:00Z").partial_cmp(&time("13:00:00+02:00")),
            Some(Ordering::Greater)
        );
        // +02:00 vs -05:00 across the local clock.
        assert_eq!(
            time("10:00:00+02:00").partial_cmp(&time("10:00:00-05:00")),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn compact_and_extended_timezone_forms_parse() {
        assert_eq!(
            time("120000Z").partial_cmp(&time("130000+0200")),
            Some(Ordering::Greater)
        );
    }

    #[test]
    fn zoned_vs_unzoned_is_incomparable() {
        assert_eq!(time("12:00:00Z").partial_cmp(&time("12:00:00")), None);
        assert_eq!(time("12:00:00").partial_cmp(&time("12:00:00+02:00")), None);
    }

    // ── malformed / excluded forms ───────────────────────────────────────────

    #[test]
    fn leap_second_is_rejected() {
        assert_eq!(time("12:00:60").partial_cmp(&time("12:01:00")), None);
        assert!(time("12:00:60").second().is_none());
    }

    #[test]
    fn twenty_four_hundred_is_rejected() {
        assert_eq!(time("24:00:00").partial_cmp(&time("00:00:00")), None);
        assert!(time("24:00:00").hour().is_none());
    }

    #[test]
    fn malformed_values_are_incomparable() {
        assert_eq!(time("nonsense").partial_cmp(&time("12:00")), None);
        assert_eq!(time("12:60").partial_cmp(&time("12:00")), None); // minute 60 invalid
    }

    // ── accessors ────────────────────────────────────────────────────────────

    #[test]
    fn accessors_report_components_and_unknowns() {
        let full = time("13:45:30.5Z");
        assert_eq!(full.hour(), Some(13));
        assert_eq!(full.minute(), Some(45));
        assert_eq!(full.second(), Some(30));
        assert_eq!(full.fractional_second(), Some(0.5));
        assert_eq!(full.timezone(), Some(0));
        assert!(!full.minute_unknown());
        assert!(!full.second_unknown());
        assert!(!full.is_partial());

        let hour_only = time("13");
        assert_eq!(hour_only.hour(), Some(13));
        assert_eq!(hour_only.minute(), None);
        assert!(hour_only.minute_unknown());
        assert!(hour_only.second_unknown());
        assert!(hour_only.is_partial());

        assert_eq!(time("09:30:00+02:30").timezone(), Some(150));
        assert_eq!(time("09:30:00-05:00").timezone(), Some(-300));
    }

    // ── lexical predicates / as_string ───────────────────────────────────────

    #[test]
    fn extended_and_compact_forms_are_distinguished() {
        assert!(time("12:00:00").is_extended());
        assert!(time("12:00").is_extended());
        assert!(time("12").is_extended()); // no separator position at all
        assert!(time("12:00:00Z").is_extended());
        assert!(time("12:00:00+02").is_extended());
        assert!(time("12:00:00+02:00").is_extended());
        assert!(!time("120000").is_extended());
        assert!(!time("1200").is_extended());
        assert!(!time("12:00:00+0200").is_extended()); // compact timezone
        assert!(!time("nonsense").is_extended());
    }

    #[test]
    fn decimal_sign_and_fractional_second_are_reported() {
        assert!(time("12:00:00,5").is_decimal_sign_comma());
        assert!(!time("12:00:00.5").is_decimal_sign_comma());
        assert!(!time("12:00:00").is_decimal_sign_comma());
        assert!(!time("nonsense").is_decimal_sign_comma());

        // "True if the fractional_second part is significant (i.e. even if = 0.0)".
        assert!(time("12:00:00.0").has_fractional_second());
        assert!(time("12:00:00,25").has_fractional_second());
        assert!(!time("12:00:00").has_fractional_second());
        assert!(!time("12:00").has_fractional_second());
        assert!(!time("nonsense").has_fractional_second());
    }

    #[test]
    fn as_string_returns_the_extended_form() {
        assert_eq!(time("120000").as_string(), "12:00:00");
        assert_eq!(time("1200").as_string(), "12:00");
        assert_eq!(time("12").as_string(), "12");
        assert_eq!(time("12:00:00").as_string(), "12:00:00");
        // Precision, decimal sign and the timezone survive the re-spelling.
        assert_eq!(time("120000,50").as_string(), "12:00:00,50");
        assert_eq!(time("120000.500Z").as_string(), "12:00:00.500Z");
        assert_eq!(time("120000+0200").as_string(), "12:00:00+02:00");
        assert_eq!(time("120000+02").as_string(), "12:00:00+02");
        // An invalid time has no extended form: verbatim.
        assert_eq!(time("24:00:00").as_string(), "24:00:00");
    }

    // ── arithmetic (wraps on the clock face) ─────────────────────────────────

    #[test]
    fn add_wraps_past_midnight() {
        assert_eq!(value(time("23:30:00").add(&dur("PT1H"))), "00:30:00");
        assert_eq!(value(time("23:59:59").add(&dur("PT1S"))), "00:00:00");
        // A whole day of duration returns the same clock reading.
        assert_eq!(value(time("09:15:30").add(&dur("P1D"))), "09:15:30");
        // Average_days_in_month = 30.42 ⇒ 30 days plus 10:04:48 of clock time.
        assert_eq!(value(time("09:15:30").add(&dur("P1M"))), "19:20:18");
    }

    #[test]
    fn subtract_wraps_before_midnight() {
        assert_eq!(value(time("00:30:00").subtract(&dur("PT1H"))), "23:30:00");
        assert_eq!(value(time("00:00:00").subtract(&dur("PT1S"))), "23:59:59");
        // A negative duration added is the same as a positive one subtracted.
        assert_eq!(value(time("00:30:00").add(&dur("-PT1H"))), "23:30:00");
    }

    #[test]
    fn arithmetic_keeps_fractional_seconds_exact() {
        assert_eq!(value(time("12:00:00.5").add(&dur("PT1S"))), "12:00:01.5");
        assert_eq!(value(time("12:00:00.5").add(&dur("PT0.5S"))), "12:00:01");
        assert_eq!(
            value(time("12:00:01").subtract(&dur("PT0.25S"))),
            "12:00:00.75"
        );
        assert_eq!(
            value(time("00:00:00").subtract(&dur("PT0.5S"))),
            "23:59:59.5"
        );
    }

    #[test]
    fn arithmetic_carries_the_timezone_through() {
        assert_eq!(
            value(time("12:00:00+02:00").add(&dur("PT30M"))),
            "12:30:00+02:00"
        );
        assert_eq!(value(time("12:00:00Z").add(&dur("PT30M"))), "12:30:00Z");
        // A compact timezone is canonicalised to the extended spelling.
        assert_eq!(
            value(time("120000+0200").add(&dur("PT30M"))),
            "12:30:00+02:00"
        );
        assert_eq!(
            value(time("12:00:00-05:30").add(&dur("PT1H"))),
            "13:00:00-05:30"
        );
    }

    // ── diff ─────────────────────────────────────────────────────────────────

    #[test]
    fn diff_reports_the_signed_clock_distance() {
        assert_eq!(
            duration_value(time("12:00:00").diff(&time("09:30:00"))),
            "PT2H30M"
        );
        assert_eq!(
            duration_value(time("09:30:00").diff(&time("12:00:00"))),
            "-PT2H30M"
        );
        assert_eq!(
            duration_value(time("12:00:00").diff(&time("12:00:00"))),
            "PT0S"
        );
        assert_eq!(
            duration_value(time("12:00:01.25").diff(&time("12:00:00"))),
            "PT1.25S"
        );
        assert_eq!(
            duration_value(time("12:00:00").diff(&time("12:00:01.25"))),
            "-PT1.25S"
        );
    }

    #[test]
    fn diff_normalises_two_zoned_times_and_refuses_a_mixed_pair() {
        // 12:00Z vs 13:00+02:00 (= 11:00 UTC) ⇒ one hour later.
        assert_eq!(
            duration_value(time("12:00:00Z").diff(&time("13:00:00+02:00"))),
            "PT1H"
        );
        assert!(time("12:00:00Z").diff(&time("12:00:00")).is_none());
        assert!(time("12:00:00").diff(&time("12:00:00+02:00")).is_none());
    }

    // ── partial / malformed operands ─────────────────────────────────────────

    // ── invariants ───────────────────────────────────────────────────────────

    /// Every invalid value names the `iso8601_time.adoc` §Invariants entry it
    /// breaks, rather than merely failing to parse.
    #[test]
    fn invalid_times_name_the_invariant_they_break() {
        for (bad, invariant) in [
            // Hour_valid — including the `24:00:00` openEHR deviation.
            ("24:00:00", "Hour_valid"),
            ("25:30", "Hour_valid"),
            ("990000", "Hour_valid"),
            // Minute_valid / Second_valid.
            ("12:60:00", "Minute_valid"),
            ("12:00:60", "Second_valid"),
            // Fractional_second_valid: a fraction with no second to carry it.
            ("12:00.5", "Fractional_second_valid"),
            // Our own lexical rule: not the valid_iso8601_time production.
            ("nonsense", "Value_lexical_form_valid"),
            ("12:00:00+15:00", "Value_lexical_form_valid"),
            ("12:0:00", "Value_lexical_form_valid"),
            ("", "Value_lexical_form_valid"),
        ] {
            let v = time(bad).invariants();
            let expected = format!("Invariant {invariant} failed on type Iso8601_time");
            assert!(
                v.iter().any(|m| m.message == expected),
                "{bad:?} should report {invariant}, got {v:?}"
            );
        }
    }

    #[test]
    fn valid_times_including_every_partial_form_report_nothing() {
        for good in [
            "12",
            "12:00",
            "12:00:00",
            "1200",
            "120000",
            "12:00:00.5",
            "12:00:00,25",
            "23:59:59",
            "00:00:00Z",
            "12:00:00+02:00",
            "120000-0500",
        ] {
            assert!(
                time(good).invariants().is_empty(),
                "{good:?} is a valid Iso8601_time"
            );
        }
    }

    #[test]
    fn partial_and_malformed_values_have_no_arithmetic() {
        assert!(time("12:00").add(&dur("PT1S")).is_none());
        assert!(time("12").subtract(&dur("PT1S")).is_none());
        assert!(time("12:00").diff(&time("12:00:00")).is_none());
        assert!(time("12:00:00").diff(&time("12")).is_none());
        assert!(time("24:00:00").add(&dur("PT1S")).is_none());
        assert!(time("12:00:00").add(&dur("1H")).is_none());
    }
}
