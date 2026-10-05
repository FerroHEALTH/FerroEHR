// @generated-from-template templates/openehr-base/foundation_types/time/time_definitions.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! `Time_Definitions` — the released validity functions, publicly.
//!
//! Spec: `docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.foundation_types.time_definitions.adoc`
//! §Functions. The class is a constant holder the emitter maps rather than
//! emits, so its constants already live in `iso8601_parse`; its eleven `valid_*`
//! functions had no home at all, which also made them invisible to the
//! unrealized-function ratchet (a mapped class is out of that projection's
//! scope by construction).
//!
//! This is the one public surface for "is this string a valid ISO-8601 X" in
//! the workspace. `openehr-rm` used to answer it with a second, hand-written
//! grammar; the two drifted twice, and both drifts shipped.

use crate::v1_2::foundation_types::time::iso8601_parse;

/// True if `y >= 0`.
///
/// Spec: `time_definitions.adoc` §Functions `valid_year` —
/// `Post: Result = y >= 0`.
#[must_use]
pub fn valid_year(y: i64) -> bool {
    y >= 0
}

/// True if `m >= 1 and m <= Months_in_year`.
///
/// Spec: `time_definitions.adoc` §Functions `valid_month`.
#[must_use]
pub fn valid_month(m: i64) -> bool {
    (1..=12).contains(&m)
}

/// True if `d >= 1 and d <= days_in_month (m, y)`.
///
/// Spec: `time_definitions.adoc` §Functions `valid_day`. The month and year are
/// checked first: `days_in_month` is undefined for a month outside `1..=12`, so
/// the post-condition cannot be evaluated without them.
#[must_use]
pub fn valid_day(y: i64, m: i64, d: i64) -> bool {
    let Some(length) = days_in_month(y, m) else {
        return false;
    };
    valid_year(y) && valid_month(m) && d >= 1 && d <= i64::from(length)
}

/// True if `(h >= 0 and h < Hours_in_day) or (h = Hours_in_day and m = 0 and
/// s = 0)`.
///
/// Spec: `time_definitions.adoc` §Functions `valid_hour`.
///
/// NOTE: the second clause admits `24:00:00`, which `iso8601_time.adoc`
/// §Description and `master06-time_types.adoc` forbid "anywhere" — a released
/// contradiction, resolved toward the prohibition.
#[must_use]
pub fn valid_hour(h: i64, m: i64, s: i64) -> bool {
    (0..24).contains(&h) && valid_minute(m) && valid_second(s)
}

/// True if `m >= 0 and m < Minutes_in_hour`.
///
/// Spec: `time_definitions.adoc` §Functions `valid_minute`.
#[must_use]
pub fn valid_minute(m: i64) -> bool {
    (0..60).contains(&m)
}

/// True if `s >= 0 and s < Seconds_in_minute`.
///
/// Spec: `time_definitions.adoc` §Functions `valid_second` —
/// `Post: Result = s >= 0 and s < Seconds_in_minute`. The `valid_iso8601_time`
/// Meaning column's "`ss` is `00` - `60`" on the same page is the contradiction;
/// this post-condition is the machine-checkable clause.
#[must_use]
pub fn valid_second(s: i64) -> bool {
    (0..60).contains(&s)
}

/// True if `fs >= 0.0 and fs < 1.0`.
///
/// Spec: `time_definitions.adoc` §Functions `valid_fractional_second`.
#[must_use]
pub fn valid_fractional_second(fs: f64) -> bool {
    fs.is_finite() && (0.0..1.0).contains(&fs)
}

/// True if `s` is a valid ISO-8601 date.
///
/// Spec: `time_definitions.adoc` §Functions `valid_iso8601_date`.
#[must_use]
pub fn valid_iso8601_date(s: &str) -> bool {
    iso8601_parse::parse_date(s).is_some()
}

/// True if `s` is a valid ISO-8601 time.
///
/// Spec: `time_definitions.adoc` §Functions `valid_iso8601_time`.
#[must_use]
pub fn valid_iso8601_time(s: &str) -> bool {
    iso8601_parse::parse_time(s).is_some()
}

/// True if `s` is a valid ISO-8601 date-time.
///
/// Spec: `time_definitions.adoc` §Functions `valid_iso8601_date_time`.
#[must_use]
pub fn valid_iso8601_date_time(s: &str) -> bool {
    iso8601_parse::parse_date_time(s).is_some()
}

/// True if `s` is a valid ISO-8601 duration.
///
/// Spec: `time_definitions.adoc` §Functions `valid_iso8601_duration`.
#[must_use]
pub fn valid_iso8601_duration(s: &str) -> bool {
    iso8601_parse::parse_duration(s).is_some()
}

/// The number of days in month `m` of year `y`, or `None` when the pair names
/// no month of the calendar.
///
/// Spec: `time_definitions.adoc` §Functions `days_in_month`.
#[must_use]
pub fn days_in_month(y: i64, m: i64) -> Option<u32> {
    let (Ok(year), Ok(month)) = (u32::try_from(y), u32::try_from(m)) else {
        return None;
    };
    iso8601_parse::days_in_month(year, month)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each numeric predicate is its own post-condition, at the boundary.
    #[test]
    fn the_numeric_predicates_are_their_post_conditions() {
        assert!(valid_year(0) && valid_year(2024));
        assert!(!valid_year(-1));

        assert!(valid_month(1) && valid_month(12));
        assert!(!valid_month(0) && !valid_month(13));

        assert!(valid_minute(0) && valid_minute(59));
        assert!(!valid_minute(60) && !valid_minute(-1));

        assert!(valid_second(0) && valid_second(59));
        assert!(
            !valid_second(60),
            "the post-condition is `s < Seconds_in_minute`"
        );

        assert!(valid_fractional_second(0.0) && valid_fractional_second(0.999));
        assert!(!valid_fractional_second(1.0) && !valid_fractional_second(-0.1));
        assert!(!valid_fractional_second(f64::NAN));
    }

    /// `valid_day` is defined via `days_in_month (m, y)`, so it answers the
    /// leap year and refuses a month it cannot evaluate.
    #[test]
    fn valid_day_follows_the_month_length() {
        assert!(valid_day(2024, 2, 29), "2024 is a leap year");
        assert!(!valid_day(2023, 2, 29));
        assert!(valid_day(2024, 1, 31) && !valid_day(2024, 4, 31));
        assert!(!valid_day(2024, 0, 1), "no month to measure");
        assert!(!valid_day(2024, 1, 0));
    }

    /// `24:00:00` is refused: the released `valid_hour` post-condition admits
    /// it while the `Iso8601_time` class forbids it anywhere.
    #[test]
    fn the_twenty_fourth_hour_is_refused() {
        assert!(valid_hour(23, 59, 59));
        assert!(!valid_hour(24, 0, 0));
        assert!(!valid_hour(-1, 0, 0));
    }

    /// The four string predicates are the one public answer to "is this a valid
    /// ISO-8601 X" — the same readers every accessor and every arithmetic
    /// function in this crate already uses.
    #[test]
    fn the_string_predicates_agree_with_the_readers() {
        assert!(valid_iso8601_date("2024-02-29") && !valid_iso8601_date("2023-02-29"));
        assert!(valid_iso8601_time("10:30:59") && !valid_iso8601_time("10:30:60"));
        assert!(
            valid_iso8601_date_time("2024-02-29T10:30:00")
                && !valid_iso8601_date_time("2024-02-30T10:30:00")
        );
        assert!(valid_iso8601_duration("P1Y2M3DT4H5M6S"));
        assert!(
            !valid_iso8601_duration("P1Y1Y"),
            "a repeated designator is not a duration"
        );
    }
}
