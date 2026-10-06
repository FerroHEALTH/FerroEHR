// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The usage report client: assembles a [`Report`], sends it, and schedules the
//! start and daily events.
//!
//! No openEHR spec governs this — our own design. The channel is outbound
//! only: nothing the collector answers is read beyond the status code, and no
//! reply changes configuration or behaviour. A send is bounded by a short
//! timeout and a body cap, is never retried in a loop, and logs its failure
//! once at DEBUG; the background task is detached from boot, health and
//! readiness, so a slow or unreachable collector reaches none of them.

use std::time::{Duration, Instant};

use http::StatusCode;
use rand::Rng as _;
use sqlx::PgPool;
use tokio::task::JoinHandle;

use crate::config::profile::SpecProfile;
use crate::licence::document::Use;
use crate::licence::state::LicenceState;
use crate::telemetry::build_info::BuildInfo;
use crate::telemetry::health::{HealthIndicator as _, HealthStatus};
use crate::telemetry::indicators::{DbHealth, MigrationsHealth};
use crate::usage_report::config::{Deployment, UsageReportConfig};
use crate::usage_report::payload::{
    CpuBucket, Event, LicenceGrant, MAX_UPTIME_S, MemoryBucket, Product, Report, SchemaV1,
    report_git_sha,
};
use crate::usage_report::store::{self, DailyClaim, StoreError};
use crate::usage_report::window;

/// The book page that carries the full notice.
pub const NOTICE_URL: &str = "https://ferroehr.eu/docs/latest/usage-report.html";

/// The largest body the collector accepts, in bytes.
pub const MAX_BODY_BYTES: usize = 64 * 1024;

/// How long after boot the first daily report is attempted.
pub const FIRST_DAILY_AFTER: Duration = Duration::from_mins(10);

/// How often a replica adds its counts to the shared window.
pub const FLUSH_EVERY: Duration = Duration::from_mins(5);

/// How long a daily attempt that failed before its claim waits to try again.
const RETRY_AFTER_FAILURE: Duration = Duration::from_hours(1);

/// The facts about this build and deployment a report states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// The server version.
    pub version: String,
    /// The commit, in the contract's form.
    pub git_sha: String,
    /// The specification generation set in force.
    pub spec_profile: SpecProfile,
    /// The licence grant type.
    pub licence: LicenceGrant,
    /// How the instance was deployed.
    pub deployment: Deployment,
}

impl Identity {
    /// Collects the identity from the build, the licence in force and the
    /// configured deployment.
    #[must_use]
    pub fn new(build: &BuildInfo, licence: &LicenceState, deployment: Deployment) -> Self {
        Self {
            version: build.version.to_owned(),
            git_sha: report_git_sha(build.git_sha),
            spec_profile: build.spec_profile,
            licence: licence_grant(licence),
            deployment,
        }
    }
}

/// The licence grant type a licence state reports.
///
/// A build with no licence in force runs under what BUSL-1.1 grants everyone,
/// which is the non-commercial grant.
#[must_use]
pub fn licence_grant(licence: &LicenceState) -> LicenceGrant {
    match licence {
        LicenceState::Licensed { verified, .. } => match verified.licence.permitted_use {
            Use::Commercial => LicenceGrant::Commercial,
            Use::NonCommercial => LicenceGrant::NonCommercial,
        },
        LicenceState::NoLicence(_) => LicenceGrant::NonCommercial,
    }
}

/// Which report to assemble.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// The start report.
    Start,
    /// The daily report.
    Daily,
}

/// A report assembled for display, and whether its instance id is the stored
/// one.
#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    /// The report, exactly as a send would render it now.
    pub report: Report,
    /// False when the database holds no instance id yet, so the one shown is a
    /// sample; the first start creates the real one.
    pub instance_stored: bool,
}

/// What a start attempt did.
#[derive(Debug, Clone, PartialEq)]
pub enum StartOutcome {
    /// The report was sent and the collector answered with this status.
    Sent(StatusCode),
    /// Another start report of this instance went out in the last ten minutes.
    RateLimited,
}

/// What a daily attempt did.
#[derive(Debug, Clone, PartialEq)]
pub enum DailyOutcome {
    /// The report was sent and the collector answered with this status.
    Sent(StatusCode),
    /// A replica already sent within the window, at this time.
    NotDue(jiff::Timestamp),
}

/// Why a report could not be assembled or sent.
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    /// The database state could not be read or claimed.
    #[error("usage report database state")]
    Store(#[from] StoreError),
    /// The HTTP client could not be built or the request failed.
    #[error("usage report request")]
    Http(#[from] reqwest::Error),
    /// The report did not serialize.
    #[error("usage report serialization")]
    Json(#[from] serde_json::Error),
    /// The rendered report exceeds the collector's body limit.
    #[error("usage report body of {0} bytes exceeds the {MAX_BODY_BYTES}-byte limit")]
    TooLarge(usize),
}

/// The usage report client of one process.
#[derive(Debug, Clone)]
pub struct UsageReporter {
    config: UsageReportConfig,
    pool: PgPool,
    identity: Identity,
    started: Instant,
    client: reqwest::Client,
}

impl UsageReporter {
    /// Creates the client over the clinical pool, which every replica of the
    /// instance shares, and sets the slow-AQL threshold of the live window.
    ///
    /// `started` is when this process started; reports state the uptime from
    /// it.
    ///
    /// # Errors
    /// [`ReportError::Http`] when the HTTP client cannot be built.
    pub fn new(
        config: UsageReportConfig,
        pool: PgPool,
        identity: Identity,
        started: Instant,
    ) -> Result<Self, ReportError> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .https_only(!config.is_loopback_http())
            .user_agent(format!("ferroehr/{}", identity.version))
            .build()?;
        window::set_slow_aql_ms(config.slow_aql_ms);
        Ok(Self {
            config,
            pool,
            identity,
            started,
            client,
        })
    }

    /// Assembles the report `kind` exactly as a send would now, without
    /// writing anything.
    ///
    /// The start report reads the stored instance id; when none exists yet it
    /// is created inside a transaction that is rolled back. The daily report
    /// reads the shared window and this process's live counts without taking
    /// them.
    ///
    /// # Errors
    /// [`ReportError::Store`] when the database cannot be read.
    pub async fn preview(&self, kind: EventKind) -> Result<Preview, ReportError> {
        let mut tx = self.pool.begin().await.map_err(StoreError::from)?;
        let instance = store::ensure_instance_on(&mut tx).await?;
        tx.rollback().await.map_err(StoreError::from)?;
        let event = match kind {
            EventKind::Start => Event::Start {
                first_start: instance.created,
            },
            EventKind::Daily => {
                let mut counts = store::peek_window(&self.pool).await?;
                counts.merge(&window::peek_live());
                Event::Daily {
                    metrics: Box::new(counts.to_metrics()),
                }
            }
        };
        Ok(Preview {
            report: self.assemble(instance.id, event).await?,
            instance_stored: !instance.created,
        })
    }

    /// Sends the start report unless one went out in the last ten minutes,
    /// creating the instance id when the database holds none.
    ///
    /// # Errors
    /// [`ReportError::Store`] when the database cannot be read or claimed,
    /// [`ReportError::Http`] when the request fails or times out, and
    /// [`ReportError::TooLarge`] when the body exceeds the limit.
    pub async fn send_start(&self) -> Result<StartOutcome, ReportError> {
        let instance = store::ensure_instance(&self.pool).await?;
        if !store::claim_start(&self.pool).await? {
            return Ok(StartOutcome::RateLimited);
        }
        let report = self
            .assemble(
                instance.id,
                Event::Start {
                    first_start: instance.created,
                },
            )
            .await?;
        Ok(StartOutcome::Sent(self.post(&report).await?))
    }

    /// Sends the daily report when this replica claims the window, with the
    /// window's metrics.
    ///
    /// This process's live counts are added to the shared window first. A
    /// claimed window is cleared whether or not the send then succeeds: a lost
    /// report is not retried.
    ///
    /// # Errors
    /// [`ReportError::Store`] when the database cannot be read or claimed,
    /// [`ReportError::Http`] when the request fails or times out, and
    /// [`ReportError::TooLarge`] when the body exceeds the limit.
    pub async fn send_daily(&self) -> Result<DailyOutcome, ReportError> {
        let instance = store::ensure_instance(&self.pool).await?;
        self.flush().await?;
        let counts = match store::claim_daily(&self.pool).await? {
            DailyClaim::Claimed(counts) => counts,
            DailyClaim::NotDue(last) => return Ok(DailyOutcome::NotDue(last)),
        };
        let event = Event::Daily {
            metrics: Box::new(counts.to_metrics()),
        };
        let report = self.assemble(instance.id, event).await?;
        Ok(DailyOutcome::Sent(self.post(&report).await?))
    }

    /// Adds this process's live counts to the shared window.
    ///
    /// # Errors
    /// [`ReportError::Store`] when the write fails; the counts are kept for
    /// the next flush.
    pub async fn flush(&self) -> Result<(), ReportError> {
        let live = window::take_live();
        if let Err(error) = store::add_to_window(&self.pool, &live).await {
            window::restore_live(&live);
            return Err(error.into());
        }
        Ok(())
    }

    /// Starts the background task: the start report at once, then the daily
    /// report [`FIRST_DAILY_AFTER`] boot and about every 24 hours, with the
    /// window flushed every [`FLUSH_EVERY`].
    ///
    /// Returns the task handle so shutdown can abort it.
    #[must_use]
    pub fn spawn(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            match self.send_start().await {
                Ok(outcome) => log_start(&outcome),
                Err(error) => tracing::debug!(
                    error = %ErrorChain(&error),
                    "usage report: the start report was not sent"
                ),
            }
            let mut flush =
                tokio::time::interval_at(tokio::time::Instant::now() + FLUSH_EVERY, FLUSH_EVERY);
            flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut next_daily = tokio::time::Instant::now() + FIRST_DAILY_AFTER;
            loop {
                tokio::select! {
                    _ = flush.tick() => {
                        if let Err(error) = self.flush().await {
                            tracing::debug!(
                                error = %ErrorChain(&error),
                                "usage report: the window was not flushed"
                            );
                        }
                    }
                    () = tokio::time::sleep_until(next_daily) => {
                        let wait = match self.send_daily().await {
                            Ok(outcome) => {
                                log_daily(&outcome);
                                match outcome {
                                    DailyOutcome::Sent(_) => next_daily_wait(None),
                                    DailyOutcome::NotDue(last) => next_daily_wait(Some(last)),
                                }
                            }
                            Err(error) => {
                                tracing::debug!(
                                    error = %ErrorChain(&error),
                                    "usage report: the daily report was not sent"
                                );
                                RETRY_AFTER_FAILURE
                            }
                        };
                        next_daily = tokio::time::Instant::now() + wait;
                    }
                }
            }
        })
    }

    /// Fills the envelope around `event` from the identity, the database and
    /// the host.
    async fn assemble(&self, instance_id: uuid::Uuid, event: Event) -> Result<Report, ReportError> {
        let db_ok = DbHealth::new(self.pool.clone()).check().await.status == HealthStatus::Up;
        let migrations_ok = MigrationsHealth::new(self.pool.clone())
            .check()
            .await
            .status
            == HealthStatus::Up;
        let postgres_major = store::postgres_major(&self.pool).await?;
        Ok(Report {
            schema: SchemaV1,
            event,
            product: Product::FerroEhr,
            instance_id,
            version: self.identity.version.clone(),
            git_sha: self.identity.git_sha.clone(),
            spec_profile: self.identity.spec_profile,
            licence: self.identity.licence,
            deployment: self.identity.deployment,
            uptime_s: self.started.elapsed().as_secs().min(MAX_UPTIME_S),
            db_ok,
            migrations_ok,
            postgres_major,
            cpu_bucket: CpuBucket::of(
                std::thread::available_parallelism().map_or(1, std::num::NonZero::get),
            ),
            memory_bucket: MemoryBucket::of(host_memory_bytes().unwrap_or_default()),
        })
    }

    /// Posts one report and returns the collector's status; the response body
    /// is never read.
    async fn post(&self, report: &Report) -> Result<StatusCode, ReportError> {
        let body = serde_json::to_vec(report)?;
        if body.len() > MAX_BODY_BYTES {
            return Err(ReportError::TooLarge(body.len()));
        }
        let response = self
            .client
            .post(self.config.endpoint.expose())
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await?;
        Ok(response.status())
    }
}

/// Says at INFO whether the report is on, then starts the background task
/// when it is.
///
/// `None` when the report is off, and when the HTTP client cannot be built,
/// which is logged at DEBUG: the report never fails boot. Nothing is sent and
/// nothing is written to the database while it is off.
#[must_use]
pub fn start(
    config: &UsageReportConfig,
    pool: &PgPool,
    identity: Identity,
    started: Instant,
) -> Option<JoinHandle<()>> {
    announce(config);
    if !config.enabled {
        return None;
    }
    match UsageReporter::new(config.clone(), pool.clone(), identity, started) {
        Ok(reporter) => Some(reporter.spawn()),
        Err(error) => {
            tracing::debug!(
                error = %ErrorChain(&error),
                "usage report: the client could not be built"
            );
            None
        }
    }
}

/// Logs the boot line that says whether the report is on, what it carries,
/// and how to switch it off.
pub fn announce(config: &UsageReportConfig) {
    if config.enabled {
        tracing::info!(
            endpoint = %config.endpoint,
            "usage report ON: instance id, version, licence type, deployment, uptime, host size \
             and coarse performance aggregates, at each start and once a day; no patient data. \
             Off: FERROEHR__USAGE_REPORT__ENABLED=false. Details: {NOTICE_URL}"
        );
    } else {
        tracing::info!(
            "usage report OFF (usage_report.enabled = false): nothing is sent. Details: \
             {NOTICE_URL}"
        );
    }
}

/// How long to wait before the next daily attempt: about a day after the last
/// send (this replica's when `last` is `None`), spread by up to an hour.
fn next_daily_wait(last: Option<jiff::Timestamp>) -> Duration {
    let jitter = rand::thread_rng().gen_range(0..=3600_u64);
    // Wake between 23.5 and 24.5 hours after the last send, above the 23-hour
    // claim threshold.
    let base_s: i64 = 23 * 3600 + 1800;
    let since_last = last.map_or(0, |last| {
        jiff::Timestamp::now()
            .as_second()
            .saturating_sub(last.as_second())
    });
    let remaining = u64::try_from(base_s.saturating_sub(since_last)).unwrap_or_default();
    Duration::from_secs(remaining.saturating_add(jitter).max(60))
}

/// Logs a start outcome at DEBUG.
fn log_start(outcome: &StartOutcome) {
    match outcome {
        StartOutcome::Sent(status) if status.is_success() => {
            tracing::debug!(status = status.as_u16(), "usage report: start report sent");
        }
        StartOutcome::Sent(status) => tracing::debug!(
            status = status.as_u16(),
            "usage report: the collector refused the start report"
        ),
        StartOutcome::RateLimited => tracing::debug!(
            "usage report: a start report of this instance went out in the last ten minutes"
        ),
    }
}

/// Logs a daily outcome at DEBUG.
fn log_daily(outcome: &DailyOutcome) {
    match outcome {
        DailyOutcome::Sent(status) if status.is_success() => {
            tracing::debug!(status = status.as_u16(), "usage report: daily report sent");
        }
        DailyOutcome::Sent(status) => tracing::debug!(
            status = status.as_u16(),
            "usage report: the collector refused the daily report"
        ),
        DailyOutcome::NotDue(last) => tracing::debug!(
            %last,
            "usage report: a replica already sent the daily report for this window"
        ),
    }
}

/// The memory available to the process in bytes: the smaller of the host's
/// memory and the cgroup limit, where Linux reports either.
fn host_memory_bytes() -> Option<u64> {
    // NOTE: proc(5) `/proc/meminfo` and the cgroup v2/v1 memory files; absent
    // off Linux, where the report states the smallest bucket.
    let total = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|text| meminfo_total_bytes(&text));
    let limit = std::fs::read_to_string("/sys/fs/cgroup/memory.max")
        .or_else(|_absent| std::fs::read_to_string("/sys/fs/cgroup/memory/memory.limit_in_bytes"))
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok());
    match (total, limit) {
        (Some(total), Some(limit)) => Some(total.min(limit)),
        (total, limit) => total.or(limit),
    }
}

/// The `MemTotal` line of `/proc/meminfo`, in bytes (the file states kB).
fn meminfo_total_bytes(meminfo: &str) -> Option<u64> {
    meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|kib| kib.parse::<u64>().ok())
        .map(|kib| kib.saturating_mul(1024))
}

/// Renders an error and its causes on one line.
struct ErrorChain<'a>(&'a dyn std::error::Error);

impl std::fmt::Display for ErrorChain<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)?;
        let mut cause = self.0.source();
        while let Some(error) = cause {
            write!(f, ": {error}")?;
            cause = error.source();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meminfo_total_is_read_in_bytes() {
        let meminfo = "MemTotal:       16303428 kB\nMemFree:         1000 kB\n";
        assert_eq!(meminfo_total_bytes(meminfo), Some(16_303_428 * 1024));
        assert_eq!(meminfo_total_bytes("MemFree: 1 kB\n"), None);
    }

    #[test]
    fn the_next_daily_wait_lands_after_the_claim_threshold() {
        for _ in 0..32 {
            let fresh = next_daily_wait(None);
            assert!(fresh >= Duration::from_mins(23 * 60 + 30));
            assert!(fresh <= Duration::from_mins(24 * 60 + 30));
        }
        let an_hour_ago = jiff::Timestamp::now() - jiff::SignedDuration::from_hours(1);
        let wait = next_daily_wait(Some(an_hour_ago));
        assert!(wait >= Duration::from_mins(22 * 60 + 28));
        let long_ago = jiff::Timestamp::now() - jiff::SignedDuration::from_hours(48);
        assert!(next_daily_wait(Some(long_ago)) <= Duration::from_mins(61));
    }
}
