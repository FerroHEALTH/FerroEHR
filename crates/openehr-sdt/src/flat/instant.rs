// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The instant an ISO 8601 date time names, so the FLAT build can order the
//! event times it reads.

/// An ISO 8601 extended date time (`YYYY-MM-DDThh:mm[:ss[.f…]]` with `Z` or
/// `±hh:mm`, or none for local time read as UTC) as nanoseconds since
/// 1970-01-01T00:00Z, or `None` when `text` is not of that form.
pub(crate) fn instant(text: &str) -> Option<i128> {
    let (date, rest) = text.split_once('T')?;
    let mut ymd = date.splitn(3, '-');
    let year: i64 = ymd.next()?.parse().ok()?;
    let month: i64 = ymd.next()?.parse().ok()?;
    let day: i64 = ymd.next()?.parse().ok()?;
    let (clock, offset_minutes) = if let Some(clock) = rest.strip_suffix('Z') {
        (clock, 0)
    } else if let Some(at) = rest.rfind(['+', '-']) {
        let (clock, offset) = rest.split_at(at);
        let sign = if offset.starts_with('-') { -1 } else { 1 };
        let (hours, minutes) = offset.get(1..)?.split_once(':')?;
        let minutes = hours.parse::<i64>().ok()? * 60 + minutes.parse::<i64>().ok()?;
        (clock, sign * minutes)
    } else {
        (rest, 0)
    };
    let mut hms = clock.splitn(3, ':');
    let hour: i64 = hms.next()?.parse().ok()?;
    let minute: i64 = hms.next()?.parse().ok()?;
    let (second, nanos) = match hms.next() {
        None => (0, 0),
        Some(sec) => match sec.split_once(['.', ',']) {
            None => (sec.parse::<i64>().ok()?, 0),
            Some((whole, frac)) => {
                let digits: String = frac.chars().chain("000000000".chars()).take(9).collect();
                (whole.parse::<i64>().ok()?, digits.parse::<i64>().ok()?)
            }
        },
    };
    // NOTE: the days-from-civil count of the proleptic Gregorian calendar
    // (H. Hinnant, "chrono-Compatible Low-Level Date Algorithms").
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2).div_euclid(5) + day - 1;
    let doe = yoe * 365 + yoe.div_euclid(4) - yoe.div_euclid(100) + doy;
    let days = era * 146_097 + doe - 719_468;
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second - offset_minutes * 60;
    Some(i128::from(seconds) * 1_000_000_000 + i128::from(nanos))
}

#[cfg(test)]
mod tests {
    use super::instant;

    #[test]
    fn instants_order_across_offsets() {
        assert!(instant("2026-03-02T10:00:00+01:00") > instant("2026-03-02T08:30:00Z"));
        assert_eq!(instant("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(instant("1970-01-01T01:00:00.5+01:00"), Some(500_000_000));
        assert_eq!(instant("not a time"), None);
    }
}
