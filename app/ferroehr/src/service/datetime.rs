// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shared decoder for the ITS-REST datetime request parameters,
//! `version_at_time` and the contribution `time_range` bounds, used by every
//! at-time read of the EHR and DEMOGRAPHIC groups.
//!
//! ITS-REST `docs/overview/Resources.md` §"Datetime format" fixes the wire
//! grammar: query parameters and path segments that are dates, datetimes or
//! times "MUST always use the _extended_ ISO 8601 format", general form
//! `YYYY-MM-DDThh:mm:ss.sss[Z|±hh:mm]`, and "Timezone SHOULD be only supplied
//! when needed, otherwise the local timezone is assumed". The offset is
//! therefore optional and an offset-less datetime names a civil datetime the
//! server resolves in its own local timezone. `jiff::Timestamp`'s grammar
//! requires an offset, hence the two-step decode below: the offset-carrying form
//! is the unchanged fast path and only a value it rejects is re-read as civil.
//!
//! NOTE: the local timezone is the server process's system timezone
//! ([`jiff::tz::TimeZone::system`]), the spec sentence assuming one ambient
//! locale for the service and giving a client no way to name another; a
//! CDR-private notion of "local" would make one request mean two instants on
//! the same host, so there is no configuration knob.
//!
//! NOTE: a date-only value (`2016-06-23`) is rejected with the same `400` as
//! garbage, the parameter being "A given time in the extended ISO 8601 format"
//! (`specifications/parameters/query/version_at_time.yaml`) while jiff's civil
//! parser would silently default the time to midnight, so the decoder requires
//! the ISO `T` designator.
//!
//! NOTE: a civil datetime falling inside a DST fold or gap resolves by jiff's
//! `Disambiguation::Compatible` default, documented on
//! [`jiff::civil::DateTime::to_zoned`] as selecting the earlier time in a fold
//! and the later time in a gap; no openEHR spec governs the ambiguity of an
//! offset-less local datetime — our own design, adopting the pinned crate's
//! documented default.
//!
//! Basic-format ISO 8601 (`20160623T134216Z`) is rejected by an explicit guard:
//! the spec's MUST is the extended format, while jiff's `Timestamp` grammar is
//! Temporal's and tolerates the basic date form.

use crate::service::status::SmError;

/// Whether the value starts with an extended-format ISO 8601 DATE —
/// `YYYY-MM-DD…` (hyphen-separated, the `-`s at byte offsets 4 and 7).
/// The extended/basic distinction lives in the separators (ISO 8601;
/// `Resources.md` §"Datetime format" MUSTs the extended form for parameters),
/// and jiff's Temporal-grammar parsers accept the basic date form, so the
/// guard is ours.
fn has_extended_date(raw: &str) -> bool {
    let b = raw.as_bytes();
    b.len() >= 10 && b.get(4) == Some(&b'-') && b.get(7) == Some(&b'-')
}

/// The instant an extended-ISO-8601 datetime parameter names, or `None` when
/// the value is not one (basic format, date-only, or garbage).
fn resolve(raw: &str) -> Option<jiff::Timestamp> {
    if !has_extended_date(raw) {
        return None;
    }
    // (1) An offset is present — `…Z` / `…±hh:mm` names an instant outright.
    // `jiff::Timestamp`'s grammar requires both a time and an offset, so this
    // accepts exactly the offset-carrying form and nothing else.
    if let Ok(instant) = raw.parse::<jiff::Timestamp>() {
        return Some(instant);
    }
    // (2) No offset, so the value is read as civil — but only after two guards,
    // because jiff's civil parser is more permissive than the general form the
    // spec MUSTs:
    //   * an RFC 9557 annotation suffix (`…[Europe/Amsterdam]`) is not part of
    //     the extended ISO 8601 form and the civil parser DISCARDS it;
    //   * a bare `YYYY-MM-DD` is silently defaulted to midnight, so the ISO `T`
    //     designator of the general form is required.
    if raw.contains('[') || !raw.contains(['T', 't']) {
        return None;
    }
    let zoned = raw
        .parse::<jiff::civil::DateTime>()
        .ok()?
        .to_zoned(jiff::tz::TimeZone::system())
        .ok()?;
    Some(zoned.timestamp())
}

/// Parse a `version_at_time` query parameter into the instant a time-travel
/// read addresses.
///
/// # Errors
/// [`SmError`] `precondition_violation` (an argument-validity failure → `400`)
/// when the value is not an extended-ISO-8601 datetime.
pub(in crate::service) fn parse_at_time(raw: &str) -> Result<jiff::Timestamp, SmError> {
    resolve(raw).ok_or_else(|| SmError::precondition(format!("invalid version_at_time: {raw}")))
}

/// Parse one bound of a contribution `time_range` (the SM
/// `Interval<Iso8601_date_time>` bounds) into an instant.
///
/// # Errors
/// [`SmError`] `precondition_violation` (→ `400`) when the bound is not an
/// extended-ISO-8601 datetime.
pub(in crate::service) fn parse_time_range_bound(raw: &str) -> Result<jiff::Timestamp, SmError> {
    resolve(raw).ok_or_else(|| SmError::precondition(format!("invalid time_range bound: {raw}")))
}

#[cfg(test)]
mod tests {
    use super::{parse_at_time, parse_time_range_bound, resolve};

    /// The same civil datetime resolved in the server's own timezone — the
    /// TZ-independent expectation for every offset-less assertion below.
    fn local(civil: &str) -> jiff::Timestamp {
        civil
            .parse::<jiff::civil::DateTime>()
            .unwrap()
            .to_zoned(jiff::tz::TimeZone::system())
            .unwrap()
            .timestamp()
    }

    /// The offset-carrying forms the fast path already accepted are unchanged:
    /// `Z` and an explicit `±hh:mm` both name the instant directly
    /// (`Resources.md` §Datetime format, `YYYY-MM-DDThh:mm:ss.sss[Z|±hh:mm]`).
    #[test]
    fn offset_forms_name_the_instant_directly() {
        let utc = parse_at_time("2016-06-23T13:42:16.117Z").unwrap();
        assert_eq!(
            utc,
            "2016-06-23T13:42:16.117Z"
                .parse::<jiff::Timestamp>()
                .unwrap()
        );

        // The spec's own example value.
        let plus_two = parse_at_time("2016-06-23T13:42:16.117+02:00").unwrap();
        assert_eq!(
            plus_two,
            "2016-06-23T11:42:16.117Z"
                .parse::<jiff::Timestamp>()
                .unwrap()
        );
    }

    /// An offset-less extended datetime is accepted and resolved in the
    /// server's local timezone ("Timezone SHOULD be only supplied when needed,
    /// otherwise the local timezone is assumed"). Asserted against the same
    /// `TimeZone::system()` resolution rather than a fixed offset, so the test
    /// is independent of the machine's zone.
    #[test]
    fn offset_less_datetime_resolves_in_the_system_timezone() {
        assert_eq!(
            parse_at_time("2016-06-23T13:42:16.117").unwrap(),
            local("2016-06-23T13:42:16.117")
        );
        // Seconds precision (no fractional part) is the same value class.
        assert_eq!(
            parse_at_time("2016-06-23T13:42:16").unwrap(),
            local("2016-06-23T13:42:16")
        );
    }

    /// Local midnight stated in full IS a time and must be accepted — the
    /// date-only rejection must not swallow it.
    #[test]
    fn explicit_local_midnight_is_accepted() {
        assert_eq!(
            parse_at_time("2016-06-23T00:00:00").unwrap(),
            local("2016-06-23T00:00:00")
        );
    }

    /// A bare date names a day, not "a given time"
    /// (`parameters/query/version_at_time.yaml`) — rejected rather than
    /// silently read as local midnight.
    #[test]
    fn date_only_is_rejected() {
        assert!(resolve("2016-06-23").is_none());
        assert!(parse_at_time("2016-06-23").is_err());
    }

    /// An RFC 9557 annotation is not the extended ISO 8601 form; without an
    /// offset the civil parser would DISCARD the named zone and resolve in the
    /// server's, so the value is rejected instead of answered wrongly.
    #[test]
    fn offset_less_zone_annotation_is_rejected() {
        assert!(parse_at_time("2016-06-23T13:42:16[Europe/Amsterdam]").is_err());
        assert!(parse_at_time("2016-06-23[Etc/UTC]").is_err());
    }

    /// Basic-format ISO 8601 is not the *extended* format the spec MUSTs.
    #[test]
    fn basic_format_is_rejected() {
        assert!(resolve("20160623T134216Z").is_none());
        assert!(resolve("20160623T134216").is_none());
        assert!(parse_at_time("20160623T134216Z").is_err());
    }

    /// Garbage stays a `400`, unchanged.
    #[test]
    fn garbage_is_rejected() {
        for bad in [
            "not-a-time",
            "",
            "T",
            "2016-13-45T99:99:99",
            "13:42:16",
            "2016-06-23 13:42:16",
        ] {
            assert!(resolve(bad).is_none(), "{bad:?} must not resolve");
            assert!(parse_at_time(bad).is_err(), "{bad:?} must be a 400");
        }
    }

    /// The `time_range` bounds share the decoder (and so the offset-less
    /// acceptance), differing only in the message they report.
    #[test]
    fn time_range_bounds_share_the_decoder() {
        assert_eq!(
            parse_time_range_bound("2016-06-23T13:42:16").unwrap(),
            local("2016-06-23T13:42:16")
        );
        let err = parse_time_range_bound("not-a-time").unwrap_err();
        assert!(
            err.to_string().contains("time_range bound"),
            "the bound error names its own parameter: {err}"
        );
    }
}
