// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Activity timelines: the pure day-bucketing every activity chart in the
//! viewer derives its points from.
//!
//! Kept out of the components (and out of the `#[server]` bodies) on purpose —
//! business logic lives in plain, unit-tested Rust and the views stay thin. It
//! is also a pure, deterministic function of its input: no clock, no locale,
//! no network, so a chart derived from it renders identically on the server
//! pass and after hydration.
//!
//! No openEHR spec governs the viewer — our own design / product extension.
//! The timestamps it buckets ARE spec-bound: `AUDIT_DETAILS.time_committed`
//! is a `DV_DATE_TIME` in extended ISO 8601
//! (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.audit_details.adoc`),
//! whose first ten characters are the `YYYY-MM-DD` calendar day.

use serde::{Deserialize, Serialize};

/// One day of activity: the calendar day and how many events fell on it.
///
/// The count is a fixed-size `u32` (never `usize`) because the type crosses the
/// server-fn boundary onto the 32-bit WASM target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityPoint {
    /// The `YYYY-MM-DD` calendar day.
    pub day: String,
    /// How many events were committed on that day.
    pub count: u32,
}

/// Bucket ISO-8601 timestamps into one [`ActivityPoint`] per calendar day,
/// ascending by day.
///
/// The day is the `YYYY-MM-DD` date prefix read with `s.get(..10)` (never
/// `&s[..10]`, which can panic on a non-char boundary — the `string_slice`
/// reliability lint); a value without a ten-character prefix is skipped rather
/// than guessed at. Counts saturate.
#[must_use]
pub fn bucket_by_day(times: &[String]) -> Vec<ActivityPoint> {
    let mut counts: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for time in times {
        if let Some(day) = time.get(..10) {
            counts
                .entry(day.to_owned())
                .and_modify(|count| *count = count.saturating_add(1))
                .or_insert(1);
        }
    }
    counts
        .into_iter()
        .map(|(day, count)| ActivityPoint { day, count })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{ActivityPoint, bucket_by_day};

    fn point(day: &str, count: u32) -> ActivityPoint {
        ActivityPoint {
            day: day.to_owned(),
            count,
        }
    }

    #[test]
    fn counts_the_date_prefix_ascending() {
        let times = vec![
            "2026-07-15T09:00:00Z".to_owned(),
            "2026-07-15T18:30:00Z".to_owned(),
            "2026-07-14T00:00:00Z".to_owned(),
            "short".to_owned(),
        ];
        assert_eq!(
            bucket_by_day(&times),
            vec![point("2026-07-14", 1), point("2026-07-15", 2)]
        );
    }

    #[test]
    fn no_timestamps_is_no_points() {
        assert!(bucket_by_day(&[]).is_empty());
        // A row whose timestamp the CDR left empty contributes nothing at all,
        // rather than a bogus bucket.
        assert!(bucket_by_day(&[String::new()]).is_empty());
    }

    #[test]
    fn an_offset_timestamp_buckets_on_its_written_day() {
        // The wire value's own date prefix is the day — the viewer does not
        // re-zone a committed timestamp (no clock, no locale: that is what
        // keeps the derived chart hydration-stable).
        assert_eq!(
            bucket_by_day(&["2026-07-15T23:30:00+02:00".to_owned()]),
            vec![point("2026-07-15", 1)]
        );
    }
}
