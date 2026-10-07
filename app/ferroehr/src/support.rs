// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The support period of this release, as the running server states it.
//!
//! `docs/law/eu/cra/text.html Art. 13(8)` sets the support period to "at least
//! five years", and Art. 13(19) asks that its end date, "including at least
//! the month and the year", be specified, and that "where technically
//! feasible" the product display a notification once it "has reached the end
//! of its support period". The support period runs five years from the month
//! the release is published; the release date is the `CHANGELOG.md` heading
//! `## [X.Y.Z] - YYYY-MM-DD` of this package version, embedded at build time
//! by `build.rs`. A build of a version with no dated heading is not a release
//! and carries no support period.
//!
//! The boot banner, `ferroehr --version`, `GET /management/info` and
//! `GET {rest root}/status` show the end month; the server logs a `WARN` at
//! boot and once a day after it has passed.

use std::time::Duration;

use jiff::civil::Date;
use serde::{Serialize, Serializer};

use crate::manufacturer::MANUFACTURER;

/// The length of the support period, in years from the release month.
pub const SUPPORT_YEARS: i16 = 5;

/// How often a running server re-checks whether its support period has ended.
const RECHECK_INTERVAL: Duration = Duration::from_hours(24);

/// The support period of one release.
///
/// Its [`Serialize`] form is a [`SupportReport`] judged on the day it is
/// serialized, so a long-running server never reports a stale status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupportPeriod {
    /// The release date; `None` for a build that is not a release.
    released: Option<Date>,
}

/// Where a release stands against its support period on a given day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportStatus {
    /// The support period has not ended.
    Supported,
    /// The support period has ended: the release is out of support.
    Ended,
    /// The build is not a release, so it has no support period.
    Unreleased,
}

/// The serialized support statement: the release date, the end month and the
/// status on the day of the report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SupportReport {
    /// The release date (`YYYY-MM-DD`); `None` for an unreleased build.
    pub release_date: Option<String>,
    /// The last month of the support period (`YYYY-MM`); `None` for an
    /// unreleased build.
    pub support_ends: Option<String>,
    /// The status on the day of the report.
    pub status: SupportStatus,
}

impl SupportPeriod {
    /// Returns the support period of the release this binary was built as.
    #[must_use]
    pub fn current() -> Self {
        // NOTE: no openEHR spec governs this — our own design; build.rs emits a
        // shape-checked `YYYY-MM-DD` or nothing, so a value that fails to parse is no release date.
        let released = env!("FERROEHR_RELEASE_DATE").parse::<Date>().ok();
        Self { released }
    }

    /// Returns the support period of a release published on `released`
    /// (`None` for a build that is not a release).
    #[must_use]
    pub const fn from_release_date(released: Option<Date>) -> Self {
        Self { released }
    }

    /// Returns the release date, when the build is a release.
    #[must_use]
    pub const fn release_date(self) -> Option<Date> {
        self.released
    }

    /// Returns the first day of the last month of the support period: the
    /// release month, [`SUPPORT_YEARS`] later.
    #[must_use]
    pub fn end_month(self) -> Option<Date> {
        let released = self.released?;
        // NOTE: no openEHR spec governs this — our own design; `None` arises only
        // past jiff's year-9999 range, which no release date reaches.
        Date::new(
            released.year().checked_add(SUPPORT_YEARS)?,
            released.month(),
            1,
        )
        .ok()
    }

    /// Returns where the release stands on `today`: supported through the last
    /// day of the end month, ended from the day after.
    #[must_use]
    pub fn status_on(self, today: Date) -> SupportStatus {
        match self.end_month() {
            None => SupportStatus::Unreleased,
            Some(end) if today > end.last_of_month() => SupportStatus::Ended,
            Some(_) => SupportStatus::Supported,
        }
    }

    /// Returns the support statement as of `today`.
    #[must_use]
    pub fn report_on(self, today: Date) -> SupportReport {
        SupportReport {
            release_date: self.released.map(|date| date.to_string()),
            support_ends: self
                .end_month()
                .map(|end| end.strftime("%Y-%m").to_string()),
            status: self.status_on(today),
        }
    }

    /// Returns the one line the banner and `ferroehr --version` print as of
    /// `today`.
    #[must_use]
    pub fn describe_on(self, today: Date) -> String {
        let Some(end) = self.end_month() else {
            return "unreleased build: no support period".to_owned();
        };
        let month = end.strftime("%B %Y");
        match self.status_on(today) {
            SupportStatus::Ended => format!("ENDED at the end of {month}"),
            SupportStatus::Supported | SupportStatus::Unreleased => {
                format!("until the end of {month}")
            }
        }
    }

    /// Returns the warning a server out of support logs, as of `today`;
    /// `None` while the release is supported or when it is no release.
    #[must_use]
    pub fn ended_notice(self, today: Date) -> Option<String> {
        if self.status_on(today) != SupportStatus::Ended {
            return None;
        }
        let end = self.end_month()?;
        Some(format!(
            "this release reached the end of its support period at the end of {}: {} no \
             longer handles its vulnerabilities, so move to the newest release \
             (Regulation (EU) 2024/2847 Art. 13(19))",
            end.strftime("%B %Y"),
            MANUFACTURER.name
        ))
    }
}

impl Serialize for SupportPeriod {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.report_on(today_utc()).serialize(serializer)
    }
}

/// Returns today's date in UTC.
#[must_use]
pub fn today_utc() -> Date {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
}

/// Logs the support statement at boot, a `WARN` when the period has ended,
/// and then re-checks once a day, warning on every day it has ended.
#[expect(
    clippy::infinite_loop,
    reason = "the support watch is a detached background task with no shutdown \
              channel — it ends when the runtime drops the task; declaring `-> !` \
              is not an option because `tokio::spawn` would then need the never \
              type as a type argument, which is unstable"
)]
pub async fn watch(period: SupportPeriod) {
    let today = today_utc();
    if let Some(notice) = period.ended_notice(today) {
        tracing::warn!(support = "ended", "{notice}");
    } else {
        tracing::info!(
            support = ?period.status_on(today),
            "support period: {}",
            period.describe_on(today)
        );
    }
    let mut ticks = tokio::time::interval_at(
        tokio::time::Instant::now() + RECHECK_INTERVAL,
        RECHECK_INTERVAL,
    );
    loop {
        ticks.tick().await;
        if let Some(notice) = period.ended_notice(today_utc()) {
            tracing::warn!(support = "ended", "{notice}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> Date {
        text.parse().expect("a test date")
    }

    /// The period runs five years from the release month, through the last
    /// day of the end month (CRA Art. 13(8)).
    #[test]
    fn the_period_ends_with_the_release_month_five_years_on() {
        let period = SupportPeriod::from_release_date(Some(date("2026-10-06")));
        assert_eq!(period.end_month(), Some(date("2031-10-01")));
        assert_eq!(
            period.status_on(date("2026-10-06")),
            SupportStatus::Supported
        );
        assert_eq!(
            period.status_on(date("2031-10-31")),
            SupportStatus::Supported
        );
        assert_eq!(period.status_on(date("2031-11-01")), SupportStatus::Ended);
        assert_eq!(
            period.describe_on(date("2027-01-01")),
            "until the end of October 2031"
        );
    }

    /// Past the end the status field says so and the server has a warning to
    /// log; before it there is none (CRA Art. 13(19)).
    #[test]
    fn an_ended_period_reports_ended_and_warns() {
        let period = SupportPeriod::from_release_date(Some(date("2026-02-28")));
        let before = period.report_on(date("2031-02-28"));
        assert_eq!(before.status, SupportStatus::Supported);
        assert!(period.ended_notice(date("2031-02-28")).is_none());

        let after = period.report_on(date("2031-03-01"));
        assert_eq!(
            after,
            SupportReport {
                release_date: Some("2026-02-28".to_owned()),
                support_ends: Some("2031-02".to_owned()),
                status: SupportStatus::Ended,
            }
        );
        let notice = period
            .ended_notice(date("2031-03-01"))
            .expect("an ended period warns");
        assert!(notice.contains("February 2031"), "{notice}");
        assert!(notice.contains("Art. 13(19)"), "{notice}");
        assert_eq!(
            period.describe_on(date("2031-03-01")),
            "ENDED at the end of February 2031"
        );
        assert_eq!(
            serde_json::to_value(after).expect("serialize")["status"],
            "ended"
        );
    }

    /// A build of a version with no dated changelog heading is no release:
    /// no end month, no warning, and it says so.
    #[test]
    fn an_unreleased_build_has_no_support_period() {
        let period = SupportPeriod::from_release_date(None);
        let today = date("2040-01-01");
        assert_eq!(period.status_on(today), SupportStatus::Unreleased);
        assert!(period.ended_notice(today).is_none());
        assert_eq!(
            period.report_on(today),
            SupportReport {
                release_date: None,
                support_ends: None,
                status: SupportStatus::Unreleased,
            }
        );
        assert_eq!(
            period.describe_on(today),
            "unreleased build: no support period"
        );
    }

    /// The live serialization carries the three fields.
    #[test]
    fn the_serialized_form_is_the_report_of_today() {
        let value = serde_json::to_value(SupportPeriod::current()).expect("serialize");
        assert!(value.get("release_date").is_some(), "{value}");
        assert!(value.get("support_ends").is_some(), "{value}");
        assert!(value["status"].is_string(), "{value}");
    }
}
