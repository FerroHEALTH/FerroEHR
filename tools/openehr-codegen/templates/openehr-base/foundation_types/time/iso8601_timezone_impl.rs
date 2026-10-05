// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written `Iso8601_timezone` spec behaviour.
//!
//! Covers the eight functions the class declares (`hour`, `minute`, `sign`,
//! `minute_unknown`, `is_partial`, `is_extended`, `is_gmt`, `as_string`).
//!
//! Spec sources (vendored):
//! - `BASE/docs/UML/classes/org.openehr.base.foundation_types.iso8601_timezone.adoc`
//!   (§Description: the accepted forms `Z | ±hh[mm]` and the `Z` ≡ `+0000`
//!   equivalence; §Functions: all eight; §Invariants).
//! - `BASE/docs/UML/classes/org.openehr.base.foundation_types.iso8601_type.adoc`
//!   (§Attributes `value`: "Representation of all descendants is a single
//!   String").
//!
//! NOTE: an accessor answers `None` for a value that is not the accepted
//! production at all, the reading the sibling
//! [`Iso8601Date`](super::iso8601_date::Iso8601Date) accessors already take: a
//! value with no components has no hour, minute or sign to report.
//!
//! NOTE: `Min_hour_valid`/`Max_hour_valid` also require `hour > 0`, which would
//! refuse `+00:00` while the same class defines `is_gmt` as "timezone `+0000`"
//! — a released-text contradiction, adjudicated in `iso8601_parse.rs` in favour
//! of `is_gmt`, so the offset bounds enforced here are
//! `hour <= Min_timezone_hour` (west) and `hour <= Max_timezone_hour` (east).
//!
//! NOTE: the class doc's §Functions entry for `minute` reads "Extract the hour
//! part of timezone, as an Integer, usually either 0 or 30" — the leading noun
//! contradicts the function's own name, its neighbours and its stated values,
//! so it is read as an editorial slip for "the minute part".

use super::iso8601_parse::{ParsedTimezone, insert_timezone_colon, scan_timezone};
use super::iso8601_timezone::Iso8601Timezone;

impl Iso8601Timezone {
    /// Parsed parts, or `None` when `value` is not a valid ISO 8601 timezone.
    fn parsed(&self) -> Option<ParsedTimezone> {
        scan_timezone(&self.value)
    }

    /// `Iso8601_timezone.hour`: "Extract the hour part of timezone, as an
    /// Integer in the range `00 - 14`" (class doc §Functions), or `None` when
    /// the value does not parse. `Z` reports `0`, its `+0000` equivalent.
    #[must_use]
    pub fn hour(&self) -> Option<u32> {
        self.parsed().map(|tz| tz.hour)
    }

    /// `Iso8601_timezone.minute`: the minute part, "usually either 0 or 30"
    /// (class doc §Functions), or `None` when the minute part is absent (the
    /// `±hh` form) or the value does not parse.
    #[must_use]
    pub fn minute(&self) -> Option<u32> {
        self.parsed().and_then(|tz| tz.minute)
    }

    /// `Iso8601_timezone.sign`: "Direction of timezone expresssed as +1 or -1"
    /// (class doc §Functions), or `None` when the value does not parse. `Z`
    /// reports `+1`, its `+0000` equivalent.
    #[must_use]
    pub fn sign(&self) -> Option<i32> {
        self.parsed().map(|tz| tz.sign)
    }

    /// `Iso8601_timezone.minute_unknown`: "Indicates whether minute part known"
    /// (class doc §Functions) — true for the `±hh` form (and for a value that
    /// does not parse, which knows nothing).
    #[must_use]
    pub fn minute_unknown(&self) -> bool {
        self.parsed().is_none_or(|tz| tz.minute.is_none())
    }

    /// `Iso8601_timezone.is_partial`: "True if this time zone is partial, i.e.
    /// if minutes is missing" (class doc §Functions).
    #[must_use]
    pub fn is_partial(&self) -> bool {
        self.minute_unknown()
    }

    /// `Iso8601_timezone.is_extended`: "True if this time-zone uses ':'
    /// separators" (class doc §Functions).
    ///
    /// A form with no separator POSITION (`Z`, `±hh`) counts as extended — the
    /// same reading `Iso8601_date.is_extended` takes for the separator-less
    /// `YYYY`; only the compact `±hhmm`, which has a position and omits it, is
    /// not. A value that does not parse is not extended.
    #[must_use]
    pub fn is_extended(&self) -> bool {
        self.parsed().is_some_and(|tz| tz.extended)
    }

    /// `Iso8601_timezone.is_gmt`: "True if timezone is UTC, i.e. `+0000`"
    /// (class doc §Functions) — a zero offset however it is written (`Z`,
    /// `+00:00`, `-0000`, `+00`), since the class description makes `Z` and
    /// `+0000` the same timezone.
    #[must_use]
    pub fn is_gmt(&self) -> bool {
        self.parsed().is_some_and(|tz| tz.offset_minutes() == 0)
    }

    /// `Iso8601_timezone.as_string`: "Return timezone string in extended
    /// format" (class doc §Functions) — the compact `±hhmm` is re-spelled
    /// `±hh:mm`; `Z` and `±hh` have no separator position and are returned
    /// unchanged.
    ///
    /// NOTE: the spec does not say what a value that is not a valid timezone
    /// returns. It is returned verbatim, since `Iso8601_type.value` is the only
    /// representation there is — our own design/extension, the same answer
    /// `Iso8601_date.as_string` gives.
    #[must_use]
    pub fn as_string(&self) -> String {
        if self.parsed().is_none() {
            return self.value.clone();
        }
        if self.value.len() == 5
            && let Some(extended) = insert_timezone_colon(&self.value)
        {
            return extended;
        }
        self.value.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tz(v: &str) -> Iso8601Timezone {
        Iso8601Timezone {
            value: v.to_owned(),
        }
    }

    /// `Z` is `+0000` (class doc §Description), so every part it publishes is
    /// the part of `+00:00`.
    #[test]
    fn the_z_literal_decomposes_as_plus_zero() {
        let zulu = tz("Z");
        assert_eq!(zulu.hour(), Some(0));
        assert_eq!(zulu.minute(), Some(0));
        assert_eq!(zulu.sign(), Some(1));
        assert!(!zulu.minute_unknown());
        assert!(!zulu.is_partial());
        assert!(zulu.is_extended());
        assert!(zulu.is_gmt());
        assert_eq!(zulu.as_string(), "Z");
    }

    #[test]
    fn components_read_through_every_accepted_form() {
        for (written, sign, hour, minute) in [
            ("+01:00", 1, 1, Some(0)),
            ("+0100", 1, 1, Some(0)),
            ("-05:30", -1, 5, Some(30)),
            ("-0530", -1, 5, Some(30)),
            ("+05", 1, 5, None),
            ("-05", -1, 5, None),
        ] {
            let value = tz(written);
            assert_eq!(value.sign(), Some(sign), "sign of {written:?}");
            assert_eq!(value.hour(), Some(hour), "hour of {written:?}");
            assert_eq!(value.minute(), minute, "minute of {written:?}");
        }
    }

    /// `is_partial`/`minute_unknown` turn on exactly one thing: whether the
    /// minute part is written.
    #[test]
    fn the_hour_only_form_is_the_partial_one() {
        assert!(tz("+05").is_partial());
        assert!(tz("+05").minute_unknown());
        assert!(!tz("+05:00").is_partial());
        assert!(!tz("+0500").is_partial());
        assert!(!tz("Z").is_partial());
        // A value that does not parse knows no minute.
        assert!(tz("not-a-timezone").is_partial());
        assert!(tz("").minute_unknown());
    }

    #[test]
    fn only_the_compact_hhmm_form_is_not_extended() {
        assert!(tz("+01:00").is_extended());
        assert!(tz("Z").is_extended());
        assert!(tz("+01").is_extended());
        assert!(!tz("+0100").is_extended());
        assert!(!tz("-0530").is_extended());
        assert!(!tz("nonsense").is_extended());
    }

    /// `is_gmt` is a zero OFFSET, not the `Z` spelling.
    #[test]
    fn every_zero_offset_spelling_is_gmt() {
        for zero in ["Z", "+00:00", "+0000", "-0000", "+00", "-00:00"] {
            assert!(tz(zero).is_gmt(), "{zero:?} is a zero offset");
        }
        for offset in ["+01:00", "-00:30", "+0001"] {
            assert!(!tz(offset).is_gmt(), "{offset:?} is not UTC");
        }
        assert!(!tz("garbage").is_gmt());
    }

    #[test]
    fn as_string_returns_the_extended_form() {
        assert_eq!(tz("+0100").as_string(), "+01:00");
        assert_eq!(tz("-0530").as_string(), "-05:30");
        assert_eq!(tz("+01:00").as_string(), "+01:00");
        assert_eq!(tz("+05").as_string(), "+05");
        assert_eq!(tz("Z").as_string(), "Z");
        // Not a valid timezone: verbatim.
        assert_eq!(tz("+99:00").as_string(), "+99:00");
        assert_eq!(tz("").as_string(), "");
    }

    /// The offset bounds are asymmetric by the class's own two invariants:
    /// `Max_timezone_hour` (14) east, `Min_timezone_hour` (12) west.
    #[test]
    fn the_offset_bounds_are_asymmetric() {
        assert_eq!(tz("+14:00").hour(), Some(14));
        assert_eq!(tz("+15:00").hour(), None);
        assert_eq!(tz("-12:00").hour(), Some(12));
        assert_eq!(tz("-13:00").hour(), None);
        // Minute_valid: valid_minute (m) = m < Minutes_in_hour.
        assert_eq!(tz("+01:59").minute(), Some(59));
        assert_eq!(tz("+01:60").minute(), None);
    }

    #[test]
    fn malformed_values_report_no_components() {
        for bad in ["", "01:00", "+1:00", "+010", "+01:0", "z", "GMT", "+"] {
            let value = tz(bad);
            assert_eq!(value.hour(), None, "{bad:?} has no hour");
            assert_eq!(value.sign(), None, "{bad:?} has no sign");
            assert_eq!(value.minute(), None, "{bad:?} has no minute");
        }
    }
}
