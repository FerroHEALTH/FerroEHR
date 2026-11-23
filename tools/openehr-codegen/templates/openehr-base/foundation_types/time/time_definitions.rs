// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The `Time_Definitions` validity functions
//! (`org.openehr.base.foundation_types.time_definitions.adoc` §Functions), the
//! one public answer to "is this a valid ISO 8601 X".

use crate::v1_3::foundation_types::time::iso8601_parse;

/// `valid_year`: `y >= 0`.
#[must_use]
pub fn valid_year(y: i64) -> bool {
    y >= 0
}

/// `valid_month`: `m >= 1 and m <= Months_in_year`.
#[must_use]
pub fn valid_month(m: i64) -> bool {
    (1..=12).contains(&m)
}

/// `valid_day`: `d >= 1 and d <= days_in_month (m, y)`, false for an invalid
/// month or year.
#[must_use]
pub fn valid_day(y: i64, m: i64, d: i64) -> bool {
    let Some(length) = days_in_month(y, m) else {
        return false;
    };
    valid_year(y) && valid_month(m) && d >= 1 && d <= i64::from(length)
}

/// `valid_hour`, without the `24:00:00` clause.
///
/// NOTE: `iso8601_time.adoc` §Description and `master06-time_types.adoc`
/// forbid `24:00:00` "anywhere".
#[must_use]
pub fn valid_hour(h: i64, m: i64, s: i64) -> bool {
    (0..24).contains(&h) && valid_minute(m) && valid_second(s)
}

/// `valid_minute`: `m >= 0 and m < Minutes_in_hour`.
#[must_use]
pub fn valid_minute(m: i64) -> bool {
    (0..60).contains(&m)
}

/// `valid_second`: `s >= 0 and s < Seconds_in_minute`.
#[must_use]
pub fn valid_second(s: i64) -> bool {
    (0..60).contains(&s)
}

/// `valid_fractional_second`: `fs >= 0.0 and fs < 1.0`.
#[must_use]
pub fn valid_fractional_second(fs: f64) -> bool {
    fs.is_finite() && (0.0..1.0).contains(&fs)
}

/// `valid_iso8601_date`.
#[must_use]
pub fn valid_iso8601_date(s: &str) -> bool {
    iso8601_parse::parse_date(s).is_some()
}

/// `valid_iso8601_time`.
#[must_use]
pub fn valid_iso8601_time(s: &str) -> bool {
    iso8601_parse::parse_time(s).is_some()
}

/// `valid_iso8601_date_time`.
#[must_use]
pub fn valid_iso8601_date_time(s: &str) -> bool {
    iso8601_parse::parse_date_time(s).is_some()
}

/// `valid_iso8601_duration`.
#[must_use]
pub fn valid_iso8601_duration(s: &str) -> bool {
    iso8601_parse::parse_duration(s).is_some()
}

/// `days_in_month`, or `None` when the pair names no month.
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

    /// The four string predicates agree with the crate's readers.
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
