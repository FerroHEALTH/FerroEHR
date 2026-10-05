// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written ISO 8601 parsing, arithmetic and completion-range machinery
//! shared by the four `Iso8601_*` `*_impl.rs` siblings.
//!
//! The generated `Iso8601_*` types hold their value as a single `String`
//! (BMM: `Iso8601_type.value`). Ordering, the spec-declared accessor
//! functions, duration reduction, and the computational functions
//! (`add`/`subtract`/`diff`/`add_nominal`/`subtract_nominal`) all need the
//! value decomposed into typed components, so this module parses the
//! documented lexical forms into plain component structs, computes on them,
//! and renders results back to a string. A malformed string parses to `None`,
//! which every caller turns into an undecidable (`None`) comparison or an
//! uncomputable (`None`) result — the generated types admit any `String`, so
//! parsing can always fail and must never panic.
//!
//! Spec sources (vendored):
//! - `BASE/docs/UML/classes/org.openehr.base.foundation_types.time_definitions.adoc`
//!   (§Constants: the average-length constants used verbatim below; §Functions:
//!   `valid_iso8601_date`/`_time`/`_date_time`/`_duration`, `valid_second`,
//!   `valid_hour`, `valid_year`).
//! - `BASE/docs/foundation_types/master06-time_types.adoc` (§Overview,
//!   §Primitive Time Types: the accepted forms and openEHR's deviations —
//!   week dates `YYYY-Www` excluded, `24:00:00` disallowed anywhere, the `W`
//!   duration designator mixable, negative durations, 4-digit years only;
//!   §Computational Functions: the definite/nominal split).
//!
//! NOTE: the openEHR specs define THAT these types are `Ordered` (via the
//! `Ordered` ancestry of `Iso8601_type`) but give NO comparison algorithm —
//! `Ordered.less_than` is abstract and none of the four classes effect it, and
//! there is no `magnitude`. Partial-value ordering, timezone normalisation, and
//! duration comparison are therefore spec-silent: the algorithms here are our
//! own design/extension, grounded only in the range/completion semantics of
//! partial ISO 8601 values (see each `*_impl.rs`).
//!
//! NOTE: `Interval.Limits_comparable`
//! (`org.openehr.base.foundation_types.interval.adoc`) is stated in terms of
//! `lower.strictly_comparable_to(upper)`, but `strictly_comparable_to` is
//! referenced nowhere-else and defined by no class in the vendored BASE spec
//! (a spec defect). Our operational comparability predicate for the temporal
//! value spaces is exactly `PartialOrd::partial_cmp(..).is_some()` — two values
//! are comparable when their completion intervals are order-separated.

use std::cmp::Ordering;
// `write!` into a `String` is infallible; the `let _ =` discards of its
// `Result` below are the standard `std::fmt` idiom, not dropped guards.
use std::fmt::Write;

// ── Time_Definitions constants (verbatim; §Constants) ────────────────────────
// The BMM `Time_Definitions` class is not emitted as a generated type (it
// carries only constants + validity functions), so its constants are mirrored
// here verbatim from
// `org.openehr.base.foundation_types.time_definitions.adoc` §Constants.

/// `Time_Definitions.Seconds_in_minute` = 60.
pub(crate) const SECONDS_IN_MINUTE: f64 = 60.0;
/// `Time_Definitions.Minutes_in_hour` = 60.
pub(crate) const MINUTES_IN_HOUR: f64 = 60.0;
/// `Time_Definitions.Hours_in_day` = 24.
pub(crate) const HOURS_IN_DAY: f64 = 24.0;
/// `Time_Definitions.Days_in_week` = 7.
pub(crate) const DAYS_IN_WEEK: f64 = 7.0;
/// `Time_Definitions.Average_days_in_month` = 30.42.
pub(crate) const AVERAGE_DAYS_IN_MONTH: f64 = 30.42;
/// `Time_Definitions.Average_days_in_year` = 365.24.
pub(crate) const AVERAGE_DAYS_IN_YEAR: f64 = 365.24;
/// `Time_Definitions.Min_timezone_hour` = 12 — "minimum hour value of a
/// timezone according to ISO 8601 (note that the -ve sign is supplied in the
/// `ISO8601_TIMEZONE` class)".
pub(crate) const MIN_TIMEZONE_HOUR: u32 = 12;
/// `Time_Definitions.Max_timezone_hour` = 14.
pub(crate) const MAX_TIMEZONE_HOUR: u32 = 14;

/// Seconds in one clock hour (`Minutes_in_hour * Seconds_in_minute`).
pub(crate) const SECONDS_IN_HOUR: f64 = MINUTES_IN_HOUR * SECONDS_IN_MINUTE;
/// Seconds in one clock day (`Hours_in_day * Minutes_in_hour * Seconds_in_minute`).
pub(crate) const SECONDS_IN_DAY: f64 = HOURS_IN_DAY * SECONDS_IN_HOUR;

// ── Integer-second forms of the same constants ───────────────────────────────
// No openEHR spec governs the arithmetic's internal representation — our own
// design. Both average lengths are a whole number of seconds (`365.24 × 86400 =
// 31_556_736`, `30.42 × 86400 = 2_628_288`, pinned by the
// `average_constants_are_whole_seconds` test below), so the `i64`-seconds plus
// separate fractional part stays faithful to the spec constants.
// NOTE: the definite computational functions (§Computational Functions) reduce
// a duration to seconds; doing that on `f64` would corrupt sub-second
// precision, so the arithmetic runs on exact whole seconds ([`ExactSeconds`]).

/// `Time_Definitions.Seconds_in_minute` as exact seconds.
pub(crate) const EXACT_SECONDS_IN_MINUTE: i64 = 60;
/// Seconds in one clock hour, exact.
pub(crate) const EXACT_SECONDS_IN_HOUR: i64 = 60 * EXACT_SECONDS_IN_MINUTE;
/// Seconds in one clock day, exact.
pub(crate) const EXACT_SECONDS_IN_DAY: i64 = 24 * EXACT_SECONDS_IN_HOUR;
/// Seconds in `Time_Definitions.Days_in_week` days, exact.
pub(crate) const EXACT_SECONDS_IN_WEEK: i64 = 7 * EXACT_SECONDS_IN_DAY;
/// `Time_Definitions.Average_days_in_month` (30.42) in exact seconds.
pub(crate) const EXACT_SECONDS_IN_AVERAGE_MONTH: i64 = 2_628_288;
/// `Time_Definitions.Average_days_in_year` (365.24) in exact seconds.
pub(crate) const EXACT_SECONDS_IN_AVERAGE_YEAR: i64 = 31_556_736;

/// The inclusive year range the openEHR time types can represent: `valid_year`
/// requires `y >= 0` (`time_definitions.adoc` §Functions) and `master06`
/// §Primitive Time Types excludes 'expanded' (>4-digit) years, so an arithmetic
/// result outside `0000`..=`9999` is not representable.
const REPRESENTABLE_YEARS: std::ops::RangeInclusive<i64> = 0..=9999;

// ── Parsed component structs ─────────────────────────────────────────────────

/// A parsed ISO 8601 date. `year` is always present (the openEHR types have no
/// sensible value without it); `month`/`day` are absent for the partial forms
/// `YYYY` and `YYYY-MM`. `extended` records the lexical form
/// (`Iso8601_date.is_extended`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParsedDate {
    pub(crate) year: u32,
    pub(crate) month: Option<u32>,
    pub(crate) day: Option<u32>,
    pub(crate) extended: bool,
}

/// A parsed ISO 8601 time. `hour` is always present; `minute`/`second` are
/// absent for the partial forms `hh` and `hh:mm`. `fractional_second` is only
/// meaningful when `second` is present. `timezone` is the offset in signed
/// minutes (`Z` → `Some(0)`, absent → `None`). `extended` and
/// `decimal_sign_comma` record the lexical form (`Iso8601_time.is_extended`,
/// `is_decimal_sign_comma`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ParsedTime {
    pub(crate) hour: u32,
    pub(crate) minute: Option<u32>,
    pub(crate) second: Option<u32>,
    pub(crate) fractional_second: Option<f64>,
    pub(crate) timezone: Option<i32>,
    pub(crate) extended: bool,
    pub(crate) decimal_sign_comma: bool,
}

/// A parsed ISO 8601 timezone designator (`Iso8601_timezone`): `sign` is `+1`
/// or `-1`, `hour` is the offset hours, `minute` is absent for the partial
/// `±hh` form, and `extended` records whether the value writes its `':'`
/// separator (`Iso8601_timezone.is_extended`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParsedTimezone {
    pub(crate) sign: i32,
    pub(crate) hour: u32,
    pub(crate) minute: Option<u32>,
    pub(crate) extended: bool,
}

impl ParsedTimezone {
    /// The offset as signed minutes east of UTC — the form
    /// [`ParsedTime::timezone`] carries. A partial `±hh` counts its absent
    /// minute part as zero.
    pub(crate) fn offset_minutes(self) -> i32 {
        #[expect(
            clippy::as_conversions,
            clippy::cast_possible_wrap,
            reason = "hour <= 14 and minute <= 59 by the checks in `scan_timezone` — far inside i32"
        )]
        let magnitude = self.hour as i32 * 60 + self.minute.unwrap_or(0) as i32;
        self.sign * magnitude
    }
}

/// A parsed ISO 8601 date/time: a date (possibly partial) with an optional
/// time part (present exactly when the source carried a `T` separator).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ParsedDateTime {
    pub(crate) date: ParsedDate,
    pub(crate) time: Option<ParsedTime>,
}

/// A parsed ISO 8601 duration. Integer designator counts plus fractional
/// seconds, a sign flag (openEHR allows a leading `-` and mixing `W` with
/// other designators) and the decimal-sign lexeme
/// (`Iso8601_duration.is_decimal_sign_comma`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ParsedDuration {
    pub(crate) negative: bool,
    pub(crate) decimal_sign_comma: bool,
    pub(crate) years: u64,
    pub(crate) months: u64,
    pub(crate) weeks: u64,
    pub(crate) days: u64,
    pub(crate) hours: u64,
    pub(crate) minutes: u64,
    pub(crate) seconds: u64,
    pub(crate) fractional_seconds: f64,
}

/// The four decomposed time-of-day fields: `(hour, minute, second,
/// fractional_second)`, with the trailing components absent for a partial time.
type TimeParts = (u32, Option<u32>, Option<u32>, Option<f64>);

// ── Small lexical helpers (no panicking indexing/slicing) ─────────────────────

/// Parse `tok` as a `width`-digit zero-filled unsigned integer, rejecting any
/// token whose length differs or that contains a non-digit (so `"1"` is not a
/// valid two-digit month and `"+02"`/whitespace are rejected).
fn parse_fixed(tok: &str, width: usize) -> Option<u32> {
    if tok.len() != width || !tok.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    tok.parse::<u32>().ok()
}

/// True for a Gregorian leap year (`master06`/`Time_definitions.days_in_month`
/// calendar semantics).
pub(crate) fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

/// Calendar days in `month` (1..=12) of `year`, or `None` for an out-of-range
/// month (`Time_definitions.valid_day` uses `days_in_month`).
pub(crate) fn days_in_month(year: u32, month: u32) -> Option<u32> {
    let d = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => return None,
    };
    Some(d)
}

/// Days since 1970-01-01 for a proleptic-Gregorian `(year, month, day)`
/// (Howard Hinnant's `days_from_civil`, a standard branch-free algorithm). Used
/// to place date/times on a common absolute-seconds axis, both for timezone
/// normalisation and for the definite computational functions. Inputs are
/// validated dates, so the result is well-defined.
#[expect(
    clippy::integer_division,
    reason = "days_from_civil is defined in terms of truncating integer division (era/leap-cycle counting); the discarded remainders are the algorithm"
)]
pub(crate) fn days_from_civil(year: u32, month: u32, day: u32) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let m = i64::from(month);
    let d = i64::from(day);
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = if m > 2 { m - 3 } else { m + 9 }; // Mar=0 … Feb=11
    let doy = (153 * mp + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

// ── Date parsing ──────────────────────────────────────────────────────────────

/// Parse an ISO 8601 date in extended (`YYYY-MM-DD`, `YYYY-MM`, `YYYY`) or
/// compact (`YYYYMMDD`, `YYYYMM`, `YYYY`) form. Week dates (`YYYY-Www`) and
/// expanded (>4-digit / signed) years are rejected (`master06` §Primitive Time
/// Types); component combinations must be calendar-valid
/// (`Time_definitions.valid_iso8601_date`).
pub(crate) fn parse_date(s: &str) -> Option<ParsedDate> {
    let d = scan_date(s)?;
    validate_date(d.year, d.month, d.day)?;
    Some(d)
}

/// Decompose a date LEXICALLY, without the calendar-validity checks: the same
/// accepted spellings as [`parse_date`], but `2021-02-29` still yields its
/// three components.
///
/// The invariant reporting in `iso8601_date_impl.rs` needs the components of an
/// INVALID value to name the rule that value breaks (`Month_valid` versus
/// `Day_valid`); a form that is not the production at all (a week date, an
/// expanded year) has no components to report and is refused here.
pub(crate) fn scan_date(s: &str) -> Option<ParsedDate> {
    if s.is_empty() || s.bytes().any(|b| b == b'W' || b == b'w') {
        return None;
    }
    let (year, month, day) = if s.contains('-') {
        let mut parts = s.split('-');
        let year = parse_fixed(parts.next()?, 4)?;
        let month = match parts.next() {
            Some(m) => Some(parse_fixed(m, 2)?),
            None => None,
        };
        let day = match parts.next() {
            Some(d) => Some(parse_fixed(d, 2)?),
            None => None,
        };
        if parts.next().is_some() {
            return None;
        }
        (year, month, day)
    } else {
        match s.len() {
            4 => (parse_fixed(s.get(0..4)?, 4)?, None, None),
            6 => (
                parse_fixed(s.get(0..4)?, 4)?,
                Some(parse_fixed(s.get(4..6)?, 2)?),
                None,
            ),
            8 => (
                parse_fixed(s.get(0..4)?, 4)?,
                Some(parse_fixed(s.get(4..6)?, 2)?),
                Some(parse_fixed(s.get(6..8)?, 2)?),
            ),
            _ => return None,
        }
    };
    // `Iso8601_date.is_extended`: "True if this date uses '-' separators".
    // NOTE: the spec does not say what a form with NO separator position
    // (`YYYY`) reports; we report it extended, keeping `as_string() == value`
    // exactly when `is_extended` — our own design, no spec governs it.
    let extended = s.contains('-') || s.len() == 4;
    Some(ParsedDate {
        year,
        month,
        day,
        extended,
    })
}

/// Enforce the `Iso8601_date` validity invariants: a day requires a month
/// (`Partial_validity`), and present components must be calendar-valid
/// (`Month_valid`/`Day_valid`).
fn validate_date(year: u32, month: Option<u32>, day: Option<u32>) -> Option<()> {
    if let Some(m) = month {
        if !(1..=12).contains(&m) {
            return None;
        }
        if let Some(d) = day {
            let max = days_in_month(year, m)?;
            if d < 1 || d > max {
                return None;
            }
        }
    } else if day.is_some() {
        return None; // day present but month unknown — invalid partial
    }
    Some(())
}

// ── Time parsing ──────────────────────────────────────────────────────────────

/// Parse an ISO 8601 time (`hh[:mm[:ss[(.|,)fff]]][Z|±hh[:mm]]` extended or the
/// compact form). Rejects `24:00:00` anywhere and a `60` (leap) second
/// (`master06`; `Time_definitions.valid_second` Post `s < Seconds_in_minute`),
/// see the leap-second NOTE in `iso8601_time_impl.rs`.
pub(crate) fn parse_time(s: &str) -> Option<ParsedTime> {
    let t = scan_time(s)?;
    validate_time(t.hour, t.minute, t.second, t.fractional_second)?;
    Some(t)
}

/// Decompose a time LEXICALLY, without the component-validity checks: the same
/// accepted spellings as [`parse_time`], but `24:00:00` and `12:00:60` still
/// yield their components, and a fractional part is kept even where no second
/// carries it (`12:00.5`).
///
/// The invariant reporting in `iso8601_time_impl.rs` needs those components to
/// name the rule the value breaks. The timezone lexeme is still range-checked
/// by [`parse_timezone`]: its bounds are `Iso8601_timezone`'s own invariants,
/// which this class does not declare.
pub(crate) fn scan_time(s: &str) -> Option<ParsedTime> {
    let (main, tz) = split_timezone_lexeme(s)?;
    let timezone = if tz.is_empty() {
        None
    } else {
        Some(parse_timezone(tz)?)
    };
    let (hour, minute, second, fractional_second) = scan_time_main(main)?;
    // `Iso8601_time.is_extended`: "True if this time uses '-', ':' separators".
    // A value counts as extended when every separator position it actually has
    // is written with a separator (so `hh`, `Z` and `±hh`, which have none, do
    // not disqualify it) — see the `scan_date` is_extended NOTE.
    let extended = body_is_extended(split_fraction_lexeme(main).0) && timezone_is_extended(tz);
    Some(ParsedTime {
        hour,
        minute,
        second,
        fractional_second,
        timezone,
        extended,
        // After a successful parse the only comma the value can carry is the
        // decimal sign (`Iso8601_time.is_decimal_sign_comma`).
        decimal_sign_comma: s.contains(','),
    })
}

/// Split a time string into its time-of-day part and its timezone lexeme (`""`
/// when unzoned): the timezone starts at the first `Z`, `+` or `-`.
pub(crate) fn split_timezone_lexeme(s: &str) -> Option<(&str, &str)> {
    match s.bytes().position(|b| b == b'Z' || b == b'+' || b == b'-') {
        Some(i) => Some((s.get(0..i)?, s.get(i..)?)),
        None => Some((s, "")),
    }
}

/// True when a time-of-day body (fraction already split off) is in extended
/// form: it either writes its `':'` separators or has no separator position
/// (the `hh` partial).
fn body_is_extended(body: &str) -> bool {
    body.contains(':') || body.len() <= 2
}

/// True when a timezone lexeme is in extended form: `""` (unzoned), `Z` and
/// `±hh` have no separator position; `±hh:mm` writes it, `±hhmm` does not.
pub(crate) fn timezone_is_extended(tz: &str) -> bool {
    tz.contains(':') || tz.len() <= 3
}

/// Decompose the time-of-day part (no timezone) in extended or compact form,
/// lexically only — see [`scan_time`].
fn scan_time_main(main: &str) -> Option<TimeParts> {
    // Separate an optional fractional-seconds tail introduced by '.' or ','.
    let (body, frac) = split_fraction(main);
    let (hour, minute, second) = if body.contains(':') {
        let mut parts = body.split(':');
        let hour = parse_fixed(parts.next()?, 2)?;
        let minute = match parts.next() {
            Some(m) => Some(parse_fixed(m, 2)?),
            None => None,
        };
        let second = match parts.next() {
            Some(sec) => Some(parse_fixed(sec, 2)?),
            None => None,
        };
        if parts.next().is_some() {
            return None;
        }
        (hour, minute, second)
    } else {
        match body.len() {
            2 => (parse_fixed(body.get(0..2)?, 2)?, None, None),
            4 => (
                parse_fixed(body.get(0..2)?, 2)?,
                Some(parse_fixed(body.get(2..4)?, 2)?),
                None,
            ),
            6 => (
                parse_fixed(body.get(0..2)?, 2)?,
                Some(parse_fixed(body.get(2..4)?, 2)?),
                Some(parse_fixed(body.get(4..6)?, 2)?),
            ),
            _ => return None,
        }
    };
    Some((hour, minute, second, frac))
}

/// Split a trailing `(.|,)digits` fractional part off the time body, returning
/// the body and the parsed fraction (a value in `[0.0, 1.0)`), or `(main, None)`
/// when absent. A malformed fraction makes the whole time unparseable.
fn split_fraction(main: &str) -> (&str, Option<f64>) {
    if let Some(i) = main.bytes().position(|b| b == b'.' || b == b',') {
        match (main.get(0..i), main.get(i + 1..)) {
            (Some(body), Some(digits))
                if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) =>
            {
                let frac = format!("0.{digits}").parse::<f64>().ok();
                (body, frac.or(Some(f64::NAN)))
            }
            // Malformed fraction: signal it as NaN so validate_time rejects it.
            _ => (main, Some(f64::NAN)),
        }
    } else {
        (main, None)
    }
}

/// Split a trailing `(.|,)digits` fractional part off the time body, returning
/// the body and the fractional lexeme VERBATIM (including its decimal sign, or
/// `""` when absent). Used for re-rendering in extended form, which must never
/// change a value's precision or decimal sign — unlike [`split_fraction`], this
/// does not validate the lexeme (the caller has already parsed the value).
fn split_fraction_lexeme(main: &str) -> (&str, &str) {
    match main.bytes().position(|b| b == b'.' || b == b',') {
        Some(i) => match (main.get(0..i), main.get(i..)) {
            (Some(body), Some(frac)) => (body, frac),
            _ => (main, ""),
        },
        None => (main, ""),
    }
}

/// Enforce the time validity invariants: `24:00:00` (and any hour 24) is
/// disallowed anywhere (`master06`), a partial time needs minute-before-second
/// (`Partial_validity`), and minute/second are range-checked. A `60` second is
/// rejected (`valid_second` Post `s < Seconds_in_minute`).
fn validate_time(
    hour: u32,
    minute: Option<u32>,
    second: Option<u32>,
    fractional_second: Option<f64>,
) -> Option<TimeParts> {
    if hour >= 24 {
        return None;
    }
    if let Some(m) = minute {
        if m >= 60 {
            return None;
        }
    } else if second.is_some() {
        return None; // second present but minute unknown — invalid partial
    }
    if let Some(sec) = second
        && sec >= 60
    {
        return None; // leap second `:60` rejected per valid_second
    }
    // `Fractional_second_valid`: a fraction is significant only on a present
    // second, and lies in `[0.0, 1.0)` (`valid_fractional_second`).
    if let Some(f) = fractional_second
        && (second.is_none() || !(f.is_finite() && (0.0..1.0).contains(&f)))
    {
        return None; // includes the NaN split_fraction uses to flag malformed input
    }
    Some((hour, minute, second, fractional_second))
}

/// Parse a timezone designator: `Z` → 0, `±hh[:mm]` / `±hh[mm]` → signed
/// minutes.
///
/// The hour bound is ASYMMETRIC, per `iso8601_timezone.adoc` §Invariants:
/// `Max_hour_valid` allows `sign = 1` up to `Max_timezone_hour` (14) while
/// `Min_hour_valid` allows `sign = -1` only to `Min_timezone_hour` (12) — the
/// real span of civil offsets either side of the dateline. `mm` is `00`..=`59`.
///
/// NOTE: both invariants also require `hour > 0`, which would refuse `+00:00`
/// while the same class defines `is_gmt` as "timezone `+0000`" — a released-text
/// contradiction, so that clause is not enforced.
pub(crate) fn parse_timezone(tz: &str) -> Option<i32> {
    Some(scan_timezone(tz)?.offset_minutes())
}

/// Decompose a timezone designator into the parts `Iso8601_timezone` publishes.
///
/// `Z` decomposes as `+00:00`, the equivalence the class description states
/// ("`Z` is a literal meaning UTC (modern replacement for GMT), i.e. timezone
/// `+0000`",
/// `BASE/docs/UML/classes/org.openehr.base.foundation_types.iso8601_timezone.adoc`
/// §Description). The `±hh` form leaves `minute` absent — the partial case
/// `Iso8601_timezone.is_partial` names.
pub(crate) fn scan_timezone(tz: &str) -> Option<ParsedTimezone> {
    if tz == "Z" {
        return Some(ParsedTimezone {
            sign: 1,
            hour: 0,
            minute: Some(0),
            extended: true,
        });
    }
    let sign = match tz.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let rest = tz.get(1..)?;
    let (hh, mm) = if let Some((h, m)) = rest.split_once(':') {
        (parse_fixed(h, 2)?, Some(parse_fixed(m, 2)?))
    } else {
        match rest.len() {
            2 => (parse_fixed(rest.get(0..2)?, 2)?, None),
            4 => (
                parse_fixed(rest.get(0..2)?, 2)?,
                Some(parse_fixed(rest.get(2..4)?, 2)?),
            ),
            _ => return None,
        }
    };
    let max_hour = if sign < 0 {
        MIN_TIMEZONE_HOUR
    } else {
        MAX_TIMEZONE_HOUR
    };
    if hh > max_hour || mm.is_some_and(|m| m > 59) {
        return None;
    }
    Some(ParsedTimezone {
        sign,
        hour: hh,
        minute: mm,
        extended: timezone_is_extended(tz),
    })
}

// ── Date/time parsing ──────────────────────────────────────────────────────────

/// Parse an ISO 8601 date/time: a date, optionally followed by `T` and a time.
pub(crate) fn parse_date_time(s: &str) -> Option<ParsedDateTime> {
    if let Some((d, t)) = s.split_once('T') {
        let date = parse_date(d)?;
        let time = parse_time(t)?;
        Some(ParsedDateTime {
            date,
            time: Some(time),
        })
    } else {
        Some(ParsedDateTime {
            date: parse_date(s)?,
            time: None,
        })
    }
}

/// Decompose a date/time LEXICALLY, without the component-validity checks — see
/// [`scan_date`] and [`scan_time`], which it composes on the same `T` split as
/// [`parse_date_time`].
pub(crate) fn scan_date_time(s: &str) -> Option<ParsedDateTime> {
    if let Some((d, t)) = s.split_once('T') {
        Some(ParsedDateTime {
            date: scan_date(d)?,
            time: Some(scan_time(t)?),
        })
    } else {
        Some(ParsedDateTime {
            date: scan_date(s)?,
            time: None,
        })
    }
}

// ── Duration parsing ────────────────────────────────────────────────────────────

/// Parse an ISO 8601 duration `[-]P[nY][nM][nW][nD][T[nH][nM][nS(.f)]]`
/// (`Time_definitions.valid_iso8601_duration`, with the openEHR deviations: a
/// leading `-` and `W` mixable with other designators). Requires at least one
/// component; only the seconds field may carry a fraction.
/// Store one duration component on `d` and return its slot in the production
/// `P[nnY][nnM][nnW][nnD][T[nnH][nnM][nnS]]`, or `None` for a designator that
/// production does not admit in this position.
///
/// The slot is what makes the production ORDERED and each designator
/// at-most-once: the caller requires it to advance strictly. Without that,
/// `P1D1M` parsed out of order and `P2Y1Y` parsed with the second `Y`
/// OVERWRITING the first — one duration silently becoming a different one,
/// which every `to_seconds`/`add_nominal` downstream then computed from.
fn apply_duration_component(
    d: &mut ParsedDuration,
    in_time: bool,
    designator: char,
    intval: u64,
    fracval: f64,
) -> Option<i8> {
    match (in_time, designator) {
        (false, 'Y') => {
            d.years = intval;
            Some(0)
        }
        (false, 'M') => {
            d.months = intval;
            Some(1)
        }
        (false, 'W') => {
            d.weeks = intval;
            Some(2)
        }
        (false, 'D') => {
            d.days = intval;
            Some(3)
        }
        (true, 'H') => {
            d.hours = intval;
            Some(4)
        }
        (true, 'M') => {
            d.minutes = intval;
            Some(5)
        }
        (true, 'S') => {
            d.seconds = intval;
            d.fractional_seconds = fracval;
            Some(6)
        }
        _ => None,
    }
}

/// One duration component's number: its integer part, its fractional part,
/// and whether a fraction was written at all.
///
/// A decimal sign with no digits after it is refused (`P1.D`).
fn read_duration_number(
    it: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> Option<(u64, f64, bool)> {
    let mut num = String::new();
    while let Some(c) = it.peek() {
        if !c.is_ascii_digit() {
            break;
        }
        num.push(*c);
        it.next();
    }
    let has_frac = matches!(it.peek(), Some('.' | ','));
    let mut frac = String::new();
    if has_frac {
        it.next();
        while let Some(c) = it.peek() {
            if !c.is_ascii_digit() {
                break;
            }
            frac.push(*c);
            it.next();
        }
    }
    let intval = num.parse::<u64>().ok()?;
    if !has_frac {
        return Some((intval, 0.0, false));
    }
    if frac.is_empty() {
        return None;
    }
    let fracval = format!("0.{frac}").parse::<f64>().ok()?;
    Some((intval, fracval, true))
}

pub(crate) fn parse_duration(s: &str) -> Option<ParsedDuration> {
    let mut it = s.chars().peekable();
    let negative = match it.peek() {
        Some('-') => {
            it.next();
            true
        }
        _ => false,
    };
    if it.next()? != 'P' {
        return None;
    }
    let mut d = ParsedDuration {
        negative,
        // After a successful parse the only comma a duration can carry is the
        // decimal sign (`Iso8601_duration.is_decimal_sign_comma`).
        decimal_sign_comma: s.contains(','),
        years: 0,
        months: 0,
        weeks: 0,
        days: 0,
        hours: 0,
        minutes: 0,
        seconds: 0,
        fractional_seconds: 0.0,
    };
    let mut scan = DurationScan::default();
    loop {
        match it.peek() {
            None => break,
            Some('T') => {
                if scan.in_time {
                    return None;
                }
                scan.in_time = true;
                it.next();
            }
            Some(c) if c.is_ascii_digit() => {
                read_duration_component(&mut it, &mut d, &mut scan)?;
            }
            _ => return None,
        }
    }
    if !scan.seen_any {
        return None; // `P`/`PT` with no components is not a duration
    }
    if scan.in_time && !scan.seen_time_component {
        return None; // `P1YT` — the production has no bare trailing `T`
    }
    Some(d)
}

/// The scan state threaded through a duration's component loop.
struct DurationScan {
    /// Whether the `T` designator separating date from time was consumed.
    in_time: bool,
    /// Whether any component at all was read.
    seen_any: bool,
    /// Whether a component followed the `T` separator.
    seen_time_component: bool,
    /// The slot of the last designator consumed, in the order the production
    /// fixes; `-1` because `Y` is slot 0.
    last_slot: i8,
}

impl Default for DurationScan {
    fn default() -> Self {
        Self {
            in_time: false,
            seen_any: false,
            seen_time_component: false,
            last_slot: -1,
        }
    }
}

/// Reads one `<number><designator>` duration component and folds it into `d`.
///
/// `None` on any violation of the production: a designator out of the fixed
/// slot order (which also rejects a repeated one), or a fraction on any field
/// but the seconds.
fn read_duration_component(
    it: &mut std::iter::Peekable<std::str::Chars<'_>>,
    d: &mut ParsedDuration,
    scan: &mut DurationScan,
) -> Option<()> {
    let (intval, fracval, has_frac) = read_duration_number(it)?;
    let designator = it.next()?;
    let slot = apply_duration_component(d, scan.in_time, designator, intval, fracval)?;
    if slot <= scan.last_slot {
        return None;
    }
    scan.last_slot = slot;
    if scan.in_time {
        scan.seen_time_component = true;
    }
    if has_frac && !(scan.in_time && designator == 'S') {
        return None; // only the seconds field carries a fraction
    }
    scan.seen_any = true;
    Some(())
}

impl ParsedDuration {
    /// Total seconds equivalent, sign applied
    /// (`Iso8601_duration.to_seconds`): non-definite years/months reduce via
    /// the `Time_definitions` average-length constants.
    #[expect(
        clippy::as_conversions,
        clippy::cast_precision_loss,
        reason = "ISO 8601 duration counts are small integers; f64 represents them exactly"
    )]
    pub(crate) fn to_seconds(self) -> f64 {
        let magnitude = self.years as f64 * AVERAGE_DAYS_IN_YEAR * SECONDS_IN_DAY
            + self.months as f64 * AVERAGE_DAYS_IN_MONTH * SECONDS_IN_DAY
            + self.weeks as f64 * DAYS_IN_WEEK * SECONDS_IN_DAY
            + self.days as f64 * SECONDS_IN_DAY
            + self.hours as f64 * SECONDS_IN_HOUR
            + self.minutes as f64 * SECONDS_IN_MINUTE
            + self.seconds as f64
            + self.fractional_seconds;
        if self.negative { -magnitude } else { magnitude }
    }
}

// ── Completion-range comparison (range semantics over partials) ──────────────────

/// The set of instants a (possibly partial) time value denotes, as a half-open
/// second interval `[lo, hi)` when `upper_open` (a partial spans a range) or a
/// degenerate point `[lo, lo]` when not (a fully-specified instant). `lo`/`hi`
/// share whatever axis the builder chose (seconds-of-day for `Iso8601_time`,
/// absolute seconds for `Iso8601_date_time`); comparison only ever happens
/// between ranges on the same axis.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CompletionRange {
    pub(crate) lo: f64,
    pub(crate) hi: f64,
    pub(crate) upper_open: bool,
}

/// True if every instant of `x` precedes every instant of `y` — the
/// range-semantics `<` (our own design; see the module NOTE). The lower bound
/// is always inclusive; `x`'s upper bound is exclusive exactly when `x` is a
/// partial (open) range, so two ranges meeting at a single boundary value are
/// separated only if the boundary is not actually attained by `x`.
pub(crate) fn range_before(x: &CompletionRange, y: &CompletionRange) -> bool {
    match x.hi.partial_cmp(&y.lo) {
        Some(Ordering::Less) => true,
        Some(Ordering::Equal) => x.upper_open,
        _ => false,
    }
}

/// The completion range of a time-of-day, on the seconds-of-day axis, shifted
/// to UTC when the value is zoned (a uniform shift preserves ordering; a bare
/// zoned time may land outside `[0, 86400)`, which is fine as a comparison
/// key). See `iso8601_time_impl.rs` for the timezone-compatibility rule.
pub(crate) fn time_completion_range(t: &ParsedTime) -> CompletionRange {
    let base = f64::from(t.hour) * SECONDS_IN_HOUR;
    let (lo, hi, upper_open) = match (t.minute, t.second) {
        (Some(m), Some(s)) => {
            let point = base
                + f64::from(m) * SECONDS_IN_MINUTE
                + f64::from(s)
                + t.fractional_second.unwrap_or(0.0);
            (point, point, false)
        }
        (Some(m), None) => {
            let start = base + f64::from(m) * SECONDS_IN_MINUTE;
            (start, start + SECONDS_IN_MINUTE, true)
        }
        _ => (base, base + SECONDS_IN_HOUR, true),
    };
    let shift = t
        .timezone
        .map_or(0.0, |off| -f64::from(off) * SECONDS_IN_MINUTE);
    CompletionRange {
        lo: lo + shift,
        hi: hi + shift,
        upper_open,
    }
}

/// The completion range of a date/time on the absolute-seconds axis (days from
/// the civil epoch × 86400 + time-of-day), shifted to UTC when zoned. The
/// coarsest unknown component sets the range width — an unknown month spans the
/// year, an unknown day spans the month, a missing time spans the day — so a
/// timezone shift that crosses midnight rolls the date automatically (it is
/// plain arithmetic on the absolute axis). The definite-calendar day counts use
/// real month lengths + leap years.
///
/// NOTE: representing a zoned date/time on a single absolute-seconds axis so a
/// timezone offset can roll the calendar date is our own design/extension — no
/// openEHR spec governs partial-temporal ordering or timezone normalisation.
#[expect(
    clippy::cast_precision_loss,
    reason = "day counts and clock components are bounded by the representable calendar; f64 represents them exactly"
)]
pub(crate) fn date_time_completion_range(dt: &ParsedDateTime) -> CompletionRange {
    let d = &dt.date;
    let (lo, hi, upper_open) = match (d.month, d.day) {
        (None, _) => (
            year_start_seconds(d.year),
            year_start_seconds(d.year + 1),
            true,
        ),
        (Some(m), None) => {
            let (ny, nm) = if m == 12 {
                (d.year + 1, 1)
            } else {
                (d.year, m + 1)
            };
            (
                month_start_seconds(d.year, m),
                month_start_seconds(ny, nm),
                true,
            )
        }
        (Some(m), Some(day)) => {
            #[expect(
                clippy::as_conversions,
                reason = "the day count is bounded by the representable calendar; f64 represents it exactly"
            )]
            let day_start = days_from_civil(d.year, m, day) as f64 * SECONDS_IN_DAY;
            match &dt.time {
                None => (day_start, day_start + SECONDS_IN_DAY, true),
                Some(t) => intraday_range(day_start, t),
            }
        }
    };
    let shift = dt
        .time
        .and_then(|t| t.timezone)
        .map_or(0.0, |off| -f64::from(off) * SECONDS_IN_MINUTE);
    CompletionRange {
        lo: lo + shift,
        hi: hi + shift,
        upper_open,
    }
}

/// Absolute seconds at `year`-01-01 00:00:00.
#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "the day count is bounded by the representable calendar; f64 represents it exactly"
)]
fn year_start_seconds(year: u32) -> f64 {
    days_from_civil(year, 1, 1) as f64 * SECONDS_IN_DAY
}

/// Absolute seconds at `year`-`month`-01 00:00:00.
#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "the day count is bounded by the representable calendar; f64 represents it exactly"
)]
fn month_start_seconds(year: u32, month: u32) -> f64 {
    days_from_civil(year, month, 1) as f64 * SECONDS_IN_DAY
}

/// The intraday completion range (relative to `day_start`) for a known-day
/// date/time's time part, mirroring [`time_completion_range`]'s partial widths.
fn intraday_range(day_start: f64, t: &ParsedTime) -> (f64, f64, bool) {
    let base = day_start + f64::from(t.hour) * SECONDS_IN_HOUR;
    match (t.minute, t.second) {
        (Some(m), Some(s)) => {
            let point = base
                + f64::from(m) * SECONDS_IN_MINUTE
                + f64::from(s)
                + t.fractional_second.unwrap_or(0.0);
            (point, point, false)
        }
        (Some(m), None) => {
            let start = base + f64::from(m) * SECONDS_IN_MINUTE;
            (start, start + SECONDS_IN_MINUTE, true)
        }
        _ => (base, base + SECONDS_IN_HOUR, true),
    }
}

// ── Exact second quantities (the arithmetic axis) ────────────────────────────

/// An exact second quantity: an integer second count plus a fractional part in
/// `[0.0, 1.0)`, denoting `whole + frac`. The representation is a *floor* one —
/// a negative quantity has `whole` one below its truncation and a positive
/// remainder (`-0.5` is `whole = -1, frac = 0.5`) — which is what makes
/// `div_euclid`/`rem_euclid` decomposition into date + time-of-day correct in
/// both directions without special-casing the sign.
///
/// NOTE: no openEHR spec governs the internal representation of the
/// computational functions — our own design/extension, chosen because `f64`
/// alone loses sub-second precision at absolute-time magnitudes (see the
/// integer-constant NOTE above).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ExactSeconds {
    pub(crate) whole: i64,
    pub(crate) frac: f64,
}

impl ExactSeconds {
    /// Normalising constructor: rejects a non-finite or negative `frac` and
    /// carries a `frac` of `1.0` or more (which is all that summing two
    /// normalised fractions can produce) into `whole`. `None` on overflow.
    pub(crate) fn new(whole: i64, frac: f64) -> Option<Self> {
        if !frac.is_finite() || !(0.0..2.0).contains(&frac) {
            return None;
        }
        let (whole, frac) = if frac >= 1.0 {
            (whole.checked_add(1)?, frac - 1.0)
        } else {
            (whole, frac)
        };
        Some(Self { whole, frac })
    }

    /// Sum of two exact quantities, `None` on overflow.
    pub(crate) fn checked_add(self, rhs: Self) -> Option<Self> {
        Self::new(self.whole.checked_add(rhs.whole)?, self.frac + rhs.frac)
    }

    /// Difference of two exact quantities, `None` on overflow.
    pub(crate) fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.checked_add(rhs.negated()?)
    }

    /// The additive inverse, `None` on overflow. Re-normalises the floor
    /// representation: `-(w + f) = (-w - 1) + (1 - f)` for a non-zero fraction.
    pub(crate) fn negated(self) -> Option<Self> {
        if self.frac > 0.0 {
            Some(Self {
                whole: self.whole.checked_neg()?.checked_sub(1)?,
                frac: 1.0 - self.frac,
            })
        } else {
            Some(Self {
                whole: self.whole.checked_neg()?,
                frac: 0.0,
            })
        }
    }

    /// The quantity as an `f64` total (for `multiply`/`divide`, whose factor is
    /// a spec `Real`).
    #[expect(
        clippy::as_conversions,
        clippy::cast_precision_loss,
        reason = "second counts over the representable calendar stay far inside 2^53, where f64 is exact on integers"
    )]
    pub(crate) fn as_f64(self) -> f64 {
        self.whole as f64 + self.frac
    }

    /// An exact quantity from an `f64` total, `None` when the value is
    /// non-finite or so large that the `f64` no longer carries whole seconds
    /// (beyond 2^53 s ≈ 285 million years, far outside the representable
    /// 0000–9999 calendar).
    pub(crate) fn from_f64(total: f64) -> Option<Self> {
        const EXACT_INTEGER_LIMIT: f64 = 9_007_199_254_740_992.0; // 2^53
        if !total.is_finite() || total.abs() >= EXACT_INTEGER_LIMIT {
            return None;
        }
        let floor = total.floor();
        #[expect(
            clippy::as_conversions,
            clippy::cast_possible_truncation,
            reason = "guarded immediately above: |floor| < 2^53, which is inside i64::MAX"
        )]
        let whole = floor as i64;
        Self::new(whole, total - floor)
    }

    /// The quantity with its fraction rounded to nanosecond precision, carrying
    /// a fraction that rounds up to a whole second into `whole`.
    ///
    /// NOTE: the openEHR specs bound a fractional second only by
    /// `valid_fractional_second` (`0.0 <= fs < 1.0`) and say nothing about the
    /// precision of a computed result, so the rendering precision is our own
    /// design/extension: nanoseconds, which covers every precision ISO 8601
    /// data carries in practice and keeps `f64` round-off out of the output.
    pub(crate) fn rounded_to_nanos(self) -> Option<Self> {
        let nanos = (self.frac * NANOS_PER_SECOND).round();
        if nanos >= NANOS_PER_SECOND {
            Some(Self {
                whole: self.whole.checked_add(1)?,
                frac: 0.0,
            })
        } else if nanos <= 0.0 {
            Some(Self {
                whole: self.whole,
                frac: 0.0,
            })
        } else {
            Some(Self {
                whole: self.whole,
                frac: nanos / NANOS_PER_SECOND,
            })
        }
    }
}

/// Nanoseconds in a second — the precision computed fractional seconds render
/// at (see [`ExactSeconds::rounded_to_nanos`]).
const NANOS_PER_SECOND: f64 = 1_000_000_000.0;

impl ParsedDuration {
    /// The duration as an exact signed second quantity — the definite
    /// (`§Computational Functions`) reduction `Iso8601_duration.to_seconds`
    /// performs, with years/months at their `Time_definitions` average lengths,
    /// computed in whole seconds so no precision is lost. `None` on overflow.
    pub(crate) fn to_exact_seconds(self) -> Option<ExactSeconds> {
        let mut whole: i64 = 0;
        for (count, unit) in [
            (self.years, EXACT_SECONDS_IN_AVERAGE_YEAR),
            (self.months, EXACT_SECONDS_IN_AVERAGE_MONTH),
            (self.weeks, EXACT_SECONDS_IN_WEEK),
            (self.days, EXACT_SECONDS_IN_DAY),
            (self.hours, EXACT_SECONDS_IN_HOUR),
            (self.minutes, EXACT_SECONDS_IN_MINUTE),
            (self.seconds, 1),
        ] {
            let part = i64::try_from(count).ok()?.checked_mul(unit)?;
            whole = whole.checked_add(part)?;
        }
        let magnitude = ExactSeconds::new(whole, self.fractional_seconds)?;
        if self.negative {
            magnitude.negated()
        } else {
            Some(magnitude)
        }
    }

    /// The NOMINAL split for calendrical arithmetic (`Iso8601_date.add_nominal`):
    /// a signed month count (`years × Months_in_year + months`) applied with
    /// day-clamping, plus the exact-second remainder (weeks, days and the time
    /// components) applied as a plain calendar shift. Both carry the duration's
    /// own sign, flipped when `subtract`. `None` on overflow.
    pub(crate) fn to_nominal_parts(self, subtract: bool) -> Option<(i64, ExactSeconds)> {
        let months = i64::try_from(self.years)
            .ok()?
            .checked_mul(12)?
            .checked_add(i64::try_from(self.months).ok()?)?;
        let mut remainder = Self {
            years: 0,
            months: 0,
            ..self
        }
        .to_exact_seconds()?;
        let mut months = if self.negative { -months } else { months };
        if subtract {
            months = months.checked_neg()?;
            remainder = remainder.negated()?;
        }
        Some((months, remainder))
    }

    /// The DEFINITE shift this duration applies (`add`), or its inverse
    /// (`subtract`). `None` on overflow.
    pub(crate) fn to_definite_shift(self, subtract: bool) -> Option<ExactSeconds> {
        let shift = self.to_exact_seconds()?;
        if subtract {
            shift.negated()
        } else {
            Some(shift)
        }
    }
}

// ── Calendar arithmetic (nominal rules) ──────────────────────────────────────

/// The proleptic-Gregorian `(year, month, day)` for a day count since
/// 1970-01-01 (Howard Hinnant's `civil_from_days`, the inverse of
/// [`days_from_civil`]). `None` when the result falls outside the representable
/// 0000–9999 year range.
#[expect(
    clippy::integer_division,
    reason = "civil_from_days is defined in terms of truncating integer division (era/leap-cycle counting); the discarded remainders are the algorithm"
)]
pub(crate) fn civil_from_days(days: i64) -> Option<(u32, u32, u32)> {
    // Bound the input to the era arithmetic's safe domain before computing: the
    // representable calendar spans roughly -719_468..=2_932_896 days.
    if !(-4_000_000..=4_000_000).contains(&days) {
        return None;
    }
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // Mar=0 … Feb=11
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    if !REPRESENTABLE_YEARS.contains(&year) {
        return None;
    }
    Some((
        u32::try_from(year).ok()?,
        u32::try_from(m).ok()?,
        u32::try_from(d).ok()?,
    ))
}

/// Shift a calendar date by a signed number of months under the NOMINAL rules
/// of `Iso8601_date.add_nominal`: the same day in the target month, clamped
/// down when that month is shorter (31 Jan `+ P1M` → 28/29 Feb, 29 Feb
/// `+ P1Y` → 28 Feb). `None` when the result leaves the representable year
/// range.
pub(crate) fn shift_months(
    year: u32,
    month: u32,
    day: u32,
    months: i64,
) -> Option<(u32, u32, u32)> {
    let total = i64::from(year)
        .checked_mul(12)?
        .checked_add(i64::from(month) - 1)?
        .checked_add(months)?;
    let shifted_year = total.div_euclid(12);
    if !REPRESENTABLE_YEARS.contains(&shifted_year) {
        return None;
    }
    let shifted_year = u32::try_from(shifted_year).ok()?;
    let shifted_month = u32::try_from(total.rem_euclid(12) + 1).ok()?;
    let last = days_in_month(shifted_year, shifted_month)?;
    Some((shifted_year, shifted_month, day.min(last)))
}

/// Decompose a seconds-of-day count into `(hour, minute, second)`. `None` when
/// the count is outside `[0, 86400)`.
#[expect(
    clippy::integer_division,
    reason = "whole hours/minutes are exactly the truncated quotients; the remainders are taken separately by the `%` terms"
)]
pub(crate) fn hms_from_seconds_of_day(seconds: i64) -> Option<(u32, u32, u32)> {
    if !(0..EXACT_SECONDS_IN_DAY).contains(&seconds) {
        return None;
    }
    Some((
        u32::try_from(seconds / EXACT_SECONDS_IN_HOUR).ok()?,
        u32::try_from((seconds % EXACT_SECONDS_IN_HOUR) / EXACT_SECONDS_IN_MINUTE).ok()?,
        u32::try_from(seconds % EXACT_SECONDS_IN_MINUTE).ok()?,
    ))
}

// ── Rendering (extended form) ────────────────────────────────────────────────
// NOTE: every computed result is rendered in the EXTENDED form, `master06`
// §Primitive Time Types calling it "strongly recommended"; the spec prescribes
// no output spelling otherwise, so the remaining choices are our own design.

/// Render a date in extended form, omitting the components the value does not
/// have (`YYYY-MM-DD`, `YYYY-MM`, `YYYY`).
pub(crate) fn render_date_extended(year: u32, month: Option<u32>, day: Option<u32>) -> String {
    match (month, day) {
        (Some(m), Some(d)) => format!("{year:04}-{m:02}-{d:02}"),
        (Some(m), None) => format!("{year:04}-{m:02}"),
        _ => format!("{year:04}"),
    }
}

/// Render a complete time of day in extended form
/// (`hh:mm:ss[.fff][Z|±hh:mm]`). `frac` must already be rounded
/// ([`ExactSeconds::rounded_to_nanos`]).
pub(crate) fn render_time_extended(
    hour: u32,
    minute: u32,
    second: u32,
    frac: f64,
    timezone: Option<i32>,
) -> String {
    format!(
        "{hour:02}:{minute:02}:{second:02}{}{}",
        fraction_lexeme_of(frac),
        render_timezone_extended(timezone)
    )
}

/// Render a timezone offset in signed minutes as an extended-form designator:
/// `Z` for UTC, `±hh:mm` otherwise, `""` when unzoned.
#[expect(
    clippy::integer_division,
    reason = "whole offset hours are the truncated quotient; the leftover minutes are taken by the paired `%`"
)]
fn render_timezone_extended(timezone: Option<i32>) -> String {
    match timezone {
        None => String::new(),
        Some(0) => "Z".to_owned(),
        Some(offset) => {
            let sign = if offset < 0 { '-' } else { '+' };
            let magnitude = offset.abs();
            let (hh, mm) = (magnitude / 60, magnitude % 60);
            format!("{sign}{hh:02}:{mm:02}")
        }
    }
}

/// The fractional-second lexeme for an already-rounded fraction: `""` when
/// zero, otherwise `'.'` followed by up to nine digits with trailing zeros
/// trimmed. The decimal sign is the period, the form `master06` §Primitive
/// Time Types recommends for written data.
fn fraction_lexeme_of(frac: f64) -> String {
    let nanos = (frac * NANOS_PER_SECOND).round();
    if nanos <= 0.0 {
        return String::new();
    }
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the value is a nanosecond count in 0..1e9 by construction — non-negative and inside u32"
    )]
    let nanos = (nanos as u64).min(999_999_999);
    format!(".{}", format!("{nanos:09}").trim_end_matches('0'))
}

/// Render a signed second quantity as an ISO 8601 duration in the canonical
/// form `[-]P[nD][T[nH][nM][n[.f]S]]`, `PT0S` for zero. `None` on overflow.
///
/// NOTE: only the DEFINITE designators (days and below) are emitted. A computed
/// result has passed through the `to_seconds` reduction the class doc mandates
/// for `add`/`subtract`, which collapses years and months into their
/// `Time_definitions` average lengths — re-deriving a `Y`/`M` component from
/// that scalar would invent calendar structure the result no longer has. The
/// leading `-` is the openEHR negative-duration deviation (`master06`
/// §Primitive Time Types). No openEHR spec prescribes the output spelling —
/// our own design/extension.
#[expect(
    clippy::integer_division,
    reason = "whole days/hours/minutes are the truncated quotients; each leftover is taken by the paired `%`"
)]
pub(crate) fn render_duration(total: ExactSeconds) -> Option<String> {
    let total = total.rounded_to_nanos()?;
    // Split into sign and magnitude, undoing the floor representation.
    let (negative, whole, frac) = if total.whole >= 0 {
        (false, i128::from(total.whole), total.frac)
    } else if total.frac > 0.0 {
        (true, -(i128::from(total.whole) + 1), 1.0 - total.frac)
    } else {
        (true, -i128::from(total.whole), 0.0)
    };
    let seconds_in_day = i128::from(EXACT_SECONDS_IN_DAY);
    let days = whole / seconds_in_day;
    let rest = whole % seconds_in_day;
    let hours = rest / i128::from(EXACT_SECONDS_IN_HOUR);
    let minutes = (rest % i128::from(EXACT_SECONDS_IN_HOUR)) / i128::from(EXACT_SECONDS_IN_MINUTE);
    let seconds = rest % i128::from(EXACT_SECONDS_IN_MINUTE);
    let fraction = fraction_lexeme_of(frac);

    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.push('P');
    if days > 0 {
        let _ = write!(out, "{days}D");
    }
    if hours > 0 || minutes > 0 || seconds > 0 || !fraction.is_empty() {
        out.push('T');
        if hours > 0 {
            let _ = write!(out, "{hours}H");
        }
        if minutes > 0 {
            let _ = write!(out, "{minutes}M");
        }
        if seconds > 0 || !fraction.is_empty() {
            let _ = write!(out, "{seconds}{fraction}S");
        }
    }
    if out.ends_with('P') {
        out.push_str("T0S"); // the zero duration
    }
    Some(out)
}

// ── Extended-form re-spelling of a stored value (`as_string`) ────────────────

/// The extended-form spelling of a stored date value (`Iso8601_date.as_string`:
/// "Return string value in extended format"), or `None` when the value is not a
/// valid date.
pub(crate) fn as_extended_date(s: &str) -> Option<String> {
    let d = parse_date(s)?;
    Some(render_date_extended(d.year, d.month, d.day))
}

/// The extended-form spelling of a stored time value (`Iso8601_time.as_string`),
/// or `None` when the value is not a valid time. Re-spelled LEXICALLY — only
/// separators are inserted — so a partial time, the fractional-second precision
/// and its decimal sign all survive verbatim.
pub(crate) fn as_extended_time(s: &str) -> Option<String> {
    parse_time(s)?; // validity gate
    let (main, tz) = split_timezone_lexeme(s)?;
    let (body, fraction) = split_fraction_lexeme(main);
    let body = if body.contains(':') {
        body.to_owned()
    } else {
        insert_separators(body, ':')?
    };
    let tz = if tz.len() == 5 {
        insert_timezone_colon(tz)? // `±hhmm` → `±hh:mm`
    } else {
        tz.to_owned()
    };
    Some(format!("{body}{fraction}{tz}"))
}

/// The extended-form spelling of a stored date/time value
/// (`Iso8601_date_time.as_string`), or `None` when the value is not a valid
/// date/time.
pub(crate) fn as_extended_date_time(s: &str) -> Option<String> {
    parse_date_time(s)?; // validity gate
    match s.split_once('T') {
        Some((date, time)) => Some(format!(
            "{}T{}",
            as_extended_date(date)?,
            as_extended_time(time)?
        )),
        None => as_extended_date(s),
    }
}

/// Insert `separator` between every pair of characters of a compact
/// fixed-width-pair lexeme (`hhmmss` → `hh:mm:ss`). `None` when the length is
/// not a multiple of two.
fn insert_separators(compact: &str, separator: char) -> Option<String> {
    let mut out = String::new();
    let mut at = 0;
    while let Some(pair) = compact.get(at..at + 2) {
        if at > 0 {
            out.push(separator);
        }
        out.push_str(pair);
        at += 2;
    }
    if at == compact.len() { Some(out) } else { None }
}

/// Re-spell a compact `±hhmm` timezone designator as extended `±hh:mm`.
pub(crate) fn insert_timezone_colon(tz: &str) -> Option<String> {
    Some(format!("{}:{}", tz.get(0..3)?, tz.get(3..5)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn average_constants_are_whole_seconds() {
        // The integer constants the arithmetic runs on ARE the spec's average
        // lengths in seconds — pinned so a future edit cannot drift them.
        let year = AVERAGE_DAYS_IN_YEAR * SECONDS_IN_DAY;
        let month = AVERAGE_DAYS_IN_MONTH * SECONDS_IN_DAY;
        assert!((year - 31_556_736.0).abs() < 1e-6);
        assert!((month - 2_628_288.0).abs() < 1e-6);
        assert_eq!(EXACT_SECONDS_IN_AVERAGE_YEAR, 31_556_736);
        assert_eq!(EXACT_SECONDS_IN_AVERAGE_MONTH, 2_628_288);
    }

    /// The lenient scanners accept EXACTLY what the strict parsers accept, plus
    /// the values whose only defect is a class invariant — which is what lets
    /// the `*_impl.rs` validators name the rule a bad value breaks.
    #[test]
    fn the_scanners_extend_the_parsers_only_where_an_invariant_decides() {
        for good in ["2020", "2020-06", "2020-06-15", "20200615", "202006"] {
            assert_eq!(scan_date(good), parse_date(good), "{good:?}");
        }
        for good in [
            "12",
            "12:00",
            "12:00:00",
            "120000",
            "12:00:00.5",
            "120000,25Z",
        ] {
            assert_eq!(scan_time(good), parse_time(good), "{good:?}");
        }
        for bad in ["2020-W01", "not-a-date", "2020-6-15", ""] {
            assert!(scan_date(bad).is_none(), "{bad:?} is not the production");
        }
        // Calendar- and component-invalid values still decompose.
        assert!(parse_date("2021-02-29").is_none() && scan_date("2021-02-29").is_some());
        assert!(parse_date("2020-13-01").is_none() && scan_date("2020-13-01").is_some());
        assert!(parse_time("24:00:00").is_none() && scan_time("24:00:00").is_some());
        assert!(parse_time("12:00:60").is_none() && scan_time("12:00:60").is_some());
        // A fraction with no second to carry it: refused by the parser (the
        // `Fractional_second_valid` clause), kept by the scanner.
        assert!(parse_time("12:00.5").is_none());
        assert_eq!(
            scan_time("12:00.5").and_then(|t| t.fractional_second),
            Some(0.5)
        );
    }

    #[test]
    fn civil_day_conversion_round_trips() {
        for (y, m, d) in [
            (1970, 1, 1),
            (2020, 2, 29),
            (2021, 12, 31),
            (0, 1, 1),
            (9999, 12, 31),
        ] {
            assert_eq!(civil_from_days(days_from_civil(y, m, d)), Some((y, m, d)));
        }
    }

    #[test]
    fn out_of_range_days_are_unrepresentable() {
        assert_eq!(civil_from_days(days_from_civil(9999, 12, 31) + 1), None);
        assert_eq!(civil_from_days(days_from_civil(0, 1, 1) - 1), None);
    }

    #[test]
    fn exact_seconds_negation_uses_floor_representation() {
        let half = ExactSeconds::new(0, 0.5).unwrap();
        let neg = half.negated().unwrap();
        assert_eq!(neg.whole, -1);
        assert!((neg.frac - 0.5).abs() < 1e-12);
        assert_eq!(neg.negated(), Some(half));
    }

    #[test]
    fn zero_and_signed_durations_render_canonically() {
        assert_eq!(
            render_duration(ExactSeconds::new(0, 0.0).unwrap()).as_deref(),
            Some("PT0S")
        );
        assert_eq!(
            render_duration(ExactSeconds::new(90_061, 0.5).unwrap()).as_deref(),
            Some("P1DT1H1M1.5S")
        );
        assert_eq!(
            render_duration(ExactSeconds::new(-1, 0.5).unwrap()).as_deref(),
            Some("-PT0.5S")
        );
    }

    /// `P[nnY][nnM][nnW][nnD][T[nnH][nnM][nnS]]` is ORDERED and each designator
    /// appears at most once. A repeat used to parse and OVERWRITE — `P2Y1Y`
    /// became `P1Y`, a different duration, with no error anywhere downstream.
    #[test]
    fn a_repeated_or_misordered_designator_is_not_a_duration() {
        // The silent-loss case: the first count vanished.
        assert!(parse_duration("P2Y1Y").is_none());
        assert!(parse_duration("P1Y1Y").is_none());
        assert!(parse_duration("PT1H2H").is_none());

        // Out of order.
        assert!(parse_duration("P1D1M").is_none());
        assert!(parse_duration("P1D1Y").is_none());
        assert!(parse_duration("PT1S1H").is_none());
        assert!(parse_duration("P1D1W").is_none());

        // A bare trailing `T` carries no time component.
        assert!(parse_duration("P1YT").is_none());
        assert!(parse_duration("PT").is_none());

        // The whole production, in order, still parses — including the openEHR
        // `W`-mixed-with-others deviation in its own slot.
        let full = parse_duration("P1Y2M3W4DT5H6M7.5S").expect("the full production");
        assert_eq!(
            (full.years, full.months, full.weeks, full.days),
            (1, 2, 3, 4)
        );
        assert_eq!((full.hours, full.minutes, full.seconds), (5, 6, 7));
        assert!(parse_duration("P1M").is_some(), "months alone");
        assert!(parse_duration("PT1M").is_some(), "minutes alone");
        assert!(parse_duration("P1W").is_some());
    }
}
