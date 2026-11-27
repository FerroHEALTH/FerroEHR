// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The activity report: the repository's change history counted per service
//! and interval (`{ext}/admin/report/*`).
//!
//! The four routes take the same `a_service` and `time_interval`; the viewer
//! asks all four with one interval and draws them together. No openEHR spec
//! surfaces these routes over REST: the CDR's own extension.

use crate::error::ViewerError;
use crate::locale::server::ServerMsg;
use crate::locale::tr_served;

/// The services the report counts over, as the CDR matches them (case
/// does not matter on the wire).
pub const SERVICES: [&str; 8] = [
    "EHR",
    "DEMOGRAPHIC",
    "DEFINITIONS",
    "QUERY",
    "ADMIN",
    "EHR_INDEX",
    "MESSAGE",
    "SYSTEM_LOG",
];

/// What the report is asked for: a service and an interval, each bound as
/// typed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ask {
    /// The service to count over.
    pub service: String,
    /// The interval's lower bound, a date or a date-time; empty for an open
    /// bound.
    pub from: String,
    /// The interval's upper bound, a date or a date-time; empty for an open
    /// bound.
    pub to: String,
}

impl Ask {
    /// The query the four routes take: the service and, with either bound
    /// set, the interval `<lower>/<upper>`. A bare date reads as the whole
    /// day in UTC, its lower bound at its start and its upper at its end.
    #[must_use]
    pub fn query(&self) -> String {
        let mut query = format!("a_service={}", urlencoding::encode(&self.service));
        if !self.from.is_empty() || !self.to.is_empty() {
            let interval = format!("{}/{}", bound(&self.from, false), bound(&self.to, true));
            query.push_str("&time_interval=");
            query.push_str(&urlencoding::encode(&interval));
        }
        query
    }
}

/// A bound as the CDR reads it: a bare `YYYY-MM-DD` is its day's first
/// instant in UTC, or with `last` its last millisecond; anything else goes as
/// typed, so the CDR judges it.
fn bound(typed: &str, last: bool) -> String {
    let typed = typed.trim();
    let day = typed
        .parse::<jiff::civil::Date>()
        .ok()
        .filter(|_| typed.len() == 10);
    let instant = day.and_then(|day| {
        let day = if last { day.tomorrow().ok()? } else { day };
        let start = day.to_zoned(jiff::tz::TimeZone::UTC).ok()?.timestamp();
        if last {
            start.checked_sub(jiff::SignedDuration::from_millis(1)).ok()
        } else {
            Some(start)
        }
    });
    instant.map_or_else(|| typed.to_owned(), |instant| instant.to_string())
}

/// The activity report for one ask.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// The matching contribution ids, by commit time then id.
    pub contributions: Vec<String>,
    /// How many contributions match.
    pub contribution_count: u64,
    /// How many composition containers had a version committed.
    pub versioned_compositions: u64,
    /// How many composition versions were committed.
    pub composition_versions: u64,
}

/// Reads the activity report for `ask`: the four routes, concurrently.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); [`ViewerError::Cdr`]
/// with the CDR's words for a `400` (an unknown service, a bad interval) and
/// a `405` (the admin group is off); [`ViewerError::Failed`] for a body that
/// is not the documented shape.
pub async fn fetch_report(ask: &Ask) -> Result<Report, ViewerError> {
    let cdr = &crate::bff::cdr()?;
    let query = &ask.query();
    let read = |route: &str| {
        let url = cdr.extensions_v1(&format!("admin/report/{route}?{query}"));
        async move { cdr.get(&url).ok().await.map(|answer| answer.body) }
    };
    let (ids, count, containers, versions) = Box::pin(async {
        futures::try_join!(
            read("contribution"),
            read("contribution/count"),
            read("versioned_composition/count"),
            read("composition_version/count"),
        )
    })
    .await?;
    let number = |body: &str| {
        body.trim()
            .parse::<u64>()
            .map_err(|error| ViewerError::failed(tr_served(ServerMsg::ActivityReportJson), error))
    };
    Ok(Report {
        contributions: serde_json::from_str(&ids).map_err(|error| {
            ViewerError::failed(tr_served(ServerMsg::ActivityReportJson), error)
        })?,
        contribution_count: number(&count)?,
        versioned_compositions: number(&containers)?,
        composition_versions: number(&versions)?,
    })
}

#[cfg(test)]
mod tests {
    use super::Ask;

    /// A bare date covers its whole day; a date-time and a refused value go
    /// as typed; no bound asks over all time.
    #[test]
    fn the_interval_covers_whole_days_and_keeps_what_was_typed() {
        let ask = |from: &str, to: &str| Ask {
            service: "Ehr".to_owned(),
            from: from.to_owned(),
            to: to.to_owned(),
        };
        assert_eq!(ask("", "").query(), "a_service=Ehr");
        assert_eq!(
            ask("2026-01-01", "2026-01-31").query(),
            "a_service=Ehr&time_interval=2026-01-01T00%3A00%3A00Z%2F2026-01-31T23%3A59%3A59.999Z"
        );
        assert_eq!(
            ask("2026-01-01T08:00:00Z", "").query(),
            "a_service=Ehr&time_interval=2026-01-01T08%3A00%3A00Z%2F"
        );
        assert_eq!(
            ask("soon", "").query(),
            "a_service=Ehr&time_interval=soon%2F"
        );
    }
}
