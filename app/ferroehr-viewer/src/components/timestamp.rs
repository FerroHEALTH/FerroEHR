// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A timestamp drawn for reading: `2026-10-18 23:18:37 UTC` in a `<time>`
//! element on one line, the value exactly as the CDR sent it in its
//! `datetime` attribute and its title.
//!
//! An ISO 8601 instant is cut to the second and shown in UTC; a date-time
//! with no offset is shown as it stands, without a zone; anything else is
//! shown verbatim. No specification governs this: our own design.

use leptos::prelude::*;

/// The reading form of `raw`, or `None` when it is no date-time this view
/// knows (a date alone, an empty cell), which the caller shows verbatim.
#[must_use]
pub fn reading(raw: &str) -> Option<String> {
    let text = raw.trim();
    // NOTE: a value that is no instant is legitimately shown as it stands, so
    // the parse failure means "not of this form", never a defect.
    if let Ok(instant) = text.parse::<jiff::Timestamp>() {
        return Some(instant.strftime("%Y-%m-%d %H:%M:%S UTC").to_string());
    }
    // A date alone also parses as a date-time at midnight; it stays a date.
    if !text.contains(['T', 't', ' ']) {
        return None;
    }
    text.parse::<jiff::civil::DateTime>()
        .ok()
        .map(|local| local.strftime("%Y-%m-%d %H:%M:%S").to_string())
}

/// Returns `raw` as a `<time>` on one line: its reading form, with the full
/// value in `datetime` and the title.
#[must_use]
pub fn timestamp(raw: String) -> AnyView {
    let shown = reading(&raw).unwrap_or_else(|| raw.clone());
    let title = raw.clone();
    view! {
        <time datetime=raw title=title class="whitespace-nowrap">
            {shown}
        </time>
    }
    .into_any()
}

/// Returns a fact's value: a date-time as [`timestamp`] draws it, anything
/// else as text, an empty value as an em dash.
#[must_use]
pub fn fact_value(value: String) -> AnyView {
    if value.is_empty() {
        "—".into_any()
    } else if reading(&value).is_some() {
        timestamp(value)
    } else {
        value.into_any()
    }
}

#[cfg(test)]
mod tests {
    use super::reading;

    #[test]
    fn an_instant_reads_to_the_second_in_utc() {
        assert_eq!(
            Some(String::from("2026-10-18 23:18:37 UTC")),
            reading("2026-10-18T23:18:37.039437+00:00")
        );
        assert_eq!(
            Some(String::from("2026-10-18 21:18:37 UTC")),
            reading("2026-10-18T23:18:37+02:00")
        );
        assert_eq!(
            Some(String::from("2026-08-29 09:00:00 UTC")),
            reading("2026-08-29T09:00:00Z")
        );
    }

    #[test]
    fn a_local_date_time_keeps_no_zone_and_anything_else_stands() {
        assert_eq!(
            Some(String::from("2022-02-03 04:05:06")),
            reading("2022-02-03T04:05:06")
        );
        assert_eq!(None, reading("2026-10-18"));
        assert_eq!(None, reading(""));
        assert_eq!(None, reading("not a time"));
    }
}
