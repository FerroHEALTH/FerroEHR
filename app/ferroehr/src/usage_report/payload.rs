// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The report payload, version 1: the `FerroPULSE` envelope with FerroEHR's
//! `metrics` object.
//!
//! No openEHR spec governs this — our own design. The field set, the value sets
//! and the bounds are `FerroPULSE`'s report v1 contract; the integration tests
//! validate every payload this module renders against that contract's JSON
//! Schemas, vendored under `corpus/ferropulse/`. Every type here is closed: a
//! field that is not declared cannot be serialized, and the `start` and `daily`
//! variants carry exactly the members their event admits.

use serde::{Serialize, Serializer};
use uuid::Uuid;

use crate::config::profile::SpecProfile;
use crate::usage_report::config::Deployment;

/// The number of latency buckets in a histogram.
pub const BUCKETS: usize = 11;

/// The upper bounds of the finite latency buckets, in milliseconds; the
/// eleventh bucket is `+Inf`.
pub const BUCKET_BOUNDS_MS: [u32; BUCKETS - 1] = [5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000];

/// The largest count a report may carry for a 24-hour window.
pub const MAX_COUNT: u64 = 10_000_000;

/// The largest latency a report may carry, in milliseconds.
pub const MAX_MS: f64 = 600_000.0;

/// The largest uptime a report may carry: ten years of seconds.
pub const MAX_UPTIME_S: u64 = 315_576_000;

/// The payload schema version; serializes as the integer `1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaV1;

impl Serialize for SchemaV1 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(1)
    }
}

/// The reporting product.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Product {
    /// This CDR.
    #[serde(rename = "ferroehr")]
    FerroEhr,
}

/// The licence grant in force, as its type alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LicenceGrant {
    /// The grant every build embeds.
    NonCommercial,
    /// An installed commercial licence.
    Commercial,
}

/// The CPUs available to the process, bucketed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CpuBucket {
    /// One or two.
    #[serde(rename = "1-2")]
    UpTo2,
    /// Three or four.
    #[serde(rename = "3-4")]
    UpTo4,
    /// Five to eight.
    #[serde(rename = "5-8")]
    UpTo8,
    /// Nine to sixteen.
    #[serde(rename = "9-16")]
    UpTo16,
    /// Seventeen or more.
    #[serde(rename = "17+")]
    Above16,
}

impl CpuBucket {
    /// Returns the bucket holding `cpus`.
    #[must_use]
    pub const fn of(cpus: usize) -> Self {
        match cpus {
            0..=2 => Self::UpTo2,
            3..=4 => Self::UpTo4,
            5..=8 => Self::UpTo8,
            9..=16 => Self::UpTo16,
            _ => Self::Above16,
        }
    }
}

/// The memory available to the process in GiB, bucketed by half-open ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum MemoryBucket {
    /// Under 4 GiB.
    #[serde(rename = "<4")]
    Below4,
    /// 4 GiB to under 8.
    #[serde(rename = "4-8")]
    Below8,
    /// 8 GiB to under 16.
    #[serde(rename = "8-16")]
    Below16,
    /// 16 GiB to under 32.
    #[serde(rename = "16-32")]
    Below32,
    /// 32 GiB or more.
    #[serde(rename = "32+")]
    AtLeast32,
}

impl MemoryBucket {
    /// Returns the bucket holding `bytes`.
    #[must_use]
    pub const fn of(bytes: u64) -> Self {
        match bytes >> 30 {
            0..=3 => Self::Below4,
            4..=7 => Self::Below8,
            8..=15 => Self::Below16,
            16..=31 => Self::Below32,
            _ => Self::AtLeast32,
        }
    }
}

/// The requests served in the window, bucketed by half-open ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RequestsBucket {
    /// None.
    #[serde(rename = "0")]
    None,
    /// 1 to 99.
    #[serde(rename = "1-100")]
    Below100,
    /// 100 to 999.
    #[serde(rename = "100-1k")]
    Below1k,
    /// 1,000 to 9,999.
    #[serde(rename = "1k-10k")]
    Below10k,
    /// 10,000 to 99,999.
    #[serde(rename = "10k-100k")]
    Below100k,
    /// 100,000 or more.
    #[serde(rename = "100k+")]
    AtLeast100k,
}

impl RequestsBucket {
    /// Returns the bucket holding `requests`.
    #[must_use]
    pub const fn of(requests: u64) -> Self {
        match requests {
            0 => Self::None,
            1..=99 => Self::Below100,
            100..=999 => Self::Below1k,
            1_000..=9_999 => Self::Below10k,
            10_000..=99_999 => Self::Below100k,
            _ => Self::AtLeast100k,
        }
    }
}

/// One route-template group's latency and errors over the window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RouteStats {
    /// The median latency, in milliseconds.
    pub p50_ms: f64,
    /// The 95th percentile latency, in milliseconds.
    pub p95_ms: f64,
    /// The 99th percentile latency, in milliseconds.
    pub p99_ms: f64,
    /// Responses with a 5xx status.
    pub errors_5xx: u64,
    /// The request count per latency bucket.
    pub histogram: [u64; BUCKETS],
}

/// The route-template groups with requests in the window; an absent group had
/// none.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Routes {
    /// EHR and `EHR_STATUS` endpoints.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ehr: Option<RouteStats>,
    /// COMPOSITION endpoints.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub composition: Option<RouteStats>,
    /// CONTRIBUTION endpoints.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contribution: Option<RouteStats>,
    /// Query execution endpoints.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<RouteStats>,
    /// Definition endpoints (templates, archetypes, stored queries).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<RouteStats>,
    /// DIRECTORY endpoints.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<RouteStats>,
    /// Administrative endpoints.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin: Option<RouteStats>,
    /// Every other endpoint of the API tree.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other: Option<RouteStats>,
}

/// AQL execution over the window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AqlStats {
    /// Queries that reached execution.
    pub executions: u64,
    /// Executions above the configured slow threshold.
    pub slow: u64,
    /// The 95th percentile execution time, in milliseconds.
    pub p95_ms: f64,
    /// The execution count per latency bucket.
    pub histogram: [u64; BUCKETS],
}

/// The 24-hour window a daily report covers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Window24h {
    /// All requests of the API tree, bucketed.
    pub requests_bucket: RequestsBucket,
    /// Per route-template group.
    pub routes: Routes,
    /// AQL execution.
    pub aql: AqlStats,
}

/// FerroEHR's `metrics` object.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Metrics {
    /// The window since the previous daily report.
    pub window_24h: Window24h,
}

/// The event a report records, with the members only that event carries.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "lowercase")]
pub enum Event {
    /// A process start.
    Start {
        /// Whether this start created the instance id.
        first_start: bool,
    },
    /// The daily report.
    Daily {
        /// The window's metrics.
        metrics: Box<Metrics>,
    },
}

/// One report, exactly as it is sent.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    /// Always `1`.
    pub schema: SchemaV1,
    /// The event and its event-specific members.
    #[serde(flatten)]
    pub event: Event,
    /// Always `ferroehr`.
    pub product: Product,
    /// The random installation id.
    pub instance_id: Uuid,
    /// The server version.
    pub version: String,
    /// The commit the binary was built from.
    pub git_sha: String,
    /// The openEHR specification generation set in force.
    pub spec_profile: SpecProfile,
    /// The licence grant type.
    pub licence: LicenceGrant,
    /// How the instance was deployed.
    pub deployment: Deployment,
    /// Seconds since this process started.
    pub uptime_s: u64,
    /// Whether the database answered.
    pub db_ok: bool,
    /// Whether the schema carries the migrations.
    pub migrations_ok: bool,
    /// The `PostgreSQL` major version.
    pub postgres_major: u16,
    /// The CPUs available, bucketed.
    pub cpu_bucket: CpuBucket,
    /// The memory available, bucketed.
    pub memory_bucket: MemoryBucket,
}

/// The `git_sha` a build reports: the commit when the build recorded one in
/// the contract's form (7 to 40 lower-case hex digits), else seven zeros.
///
/// The report has no "unknown" spelling for a commit, so a build from a
/// tarball, which records none, reports the all-zero commit.
#[must_use]
pub fn report_git_sha(revision: &str) -> String {
    let valid = (7..=40).contains(&revision.len())
        && revision
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if valid {
        revision.to_owned()
    } else {
        "0000000".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_are_half_open() {
        assert_eq!(RequestsBucket::of(0), RequestsBucket::None);
        assert_eq!(RequestsBucket::of(99), RequestsBucket::Below100);
        assert_eq!(RequestsBucket::of(100), RequestsBucket::Below1k);
        assert_eq!(RequestsBucket::of(100_000), RequestsBucket::AtLeast100k);
        assert_eq!(CpuBucket::of(1), CpuBucket::UpTo2);
        assert_eq!(CpuBucket::of(16), CpuBucket::UpTo16);
        assert_eq!(CpuBucket::of(17), CpuBucket::Above16);
        let gib = 1_u64 << 30;
        assert_eq!(MemoryBucket::of(4 * gib - 1), MemoryBucket::Below4);
        assert_eq!(MemoryBucket::of(4 * gib), MemoryBucket::Below8);
        assert_eq!(MemoryBucket::of(32 * gib), MemoryBucket::AtLeast32);
    }

    #[test]
    fn a_git_sha_outside_the_contract_reports_zeros() {
        assert_eq!(report_git_sha("1a2b3c4d5e6f"), "1a2b3c4d5e6f");
        assert_eq!(report_git_sha("unknown"), "0000000");
        assert_eq!(report_git_sha("1A2B3C4"), "0000000");
        assert_eq!(report_git_sha("abc"), "0000000");
    }
}
