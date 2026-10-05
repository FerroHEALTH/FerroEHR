// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The usage report's metrics window: latency histograms per route-template
//! group and for AQL, recorded in process at the points that feed the
//! Prometheus instruments.
//!
//! No openEHR spec governs this — our own design. A recording carries a route
//! TEMPLATE (never a concrete path), a duration and a status class; nothing of
//! a request's path parameters, body, query text or caller reaches it, so
//! nothing of them can reach a report. Each replica adds its counts to the
//! shared database window ([`crate::usage_report::store`]), and the replica
//! that claims the daily report renders the totals with [`WindowCounts::to_metrics`].

use std::collections::BTreeMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use http::StatusCode;

use crate::usage_report::payload::{
    AqlStats, BUCKET_BOUNDS_MS, BUCKETS, MAX_COUNT, MAX_MS, Metrics, RequestsBucket, RouteStats,
    Routes, Window24h,
};

/// A route-template group of the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RouteGroup {
    /// EHR and `EHR_STATUS` endpoints.
    Ehr,
    /// COMPOSITION endpoints.
    Composition,
    /// CONTRIBUTION endpoints.
    Contribution,
    /// Query execution endpoints.
    Query,
    /// Definition endpoints.
    Definition,
    /// DIRECTORY endpoints.
    Directory,
    /// Administrative endpoints.
    Admin,
    /// Every other endpoint of the API tree.
    Other,
}

impl RouteGroup {
    /// Returns the group a matched route template belongs to.
    ///
    /// The first literal segment naming an API family decides (`admin`,
    /// `definition`, `query`, `ehr`, or a family the report folds into
    /// `other`); under `ehr` the resource segment refines it. Parameter
    /// segments (`{ehr_id}`) are never read.
    #[must_use]
    pub fn of_template(template: &str) -> Self {
        let mut segments = template
            .split('/')
            .filter(|segment| !segment.is_empty() && !segment.starts_with('{'));
        while let Some(segment) = segments.next() {
            match segment {
                "admin" => return Self::Admin,
                "definition" => return Self::Definition,
                "query" => return Self::Query,
                "ehr" => {
                    return segments
                        .find_map(|resource| match resource {
                            "composition" | "versioned_composition" => Some(Self::Composition),
                            "contribution" => Some(Self::Contribution),
                            "directory" => Some(Self::Directory),
                            _ => None,
                        })
                        .unwrap_or(Self::Ehr);
                }
                "demographic" | "message" | "terminology" | "fhir" => return Self::Other,
                _ => {}
            }
        }
        Self::Other
    }
}

/// One series of the window: a route-template group, or AQL execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Series {
    /// Requests of one route-template group.
    Route(RouteGroup),
    /// AQL executions.
    Aql,
}

impl Series {
    /// Every series, in storage order.
    pub const ALL: [Self; 9] = [
        Self::Route(RouteGroup::Ehr),
        Self::Route(RouteGroup::Composition),
        Self::Route(RouteGroup::Contribution),
        Self::Route(RouteGroup::Query),
        Self::Route(RouteGroup::Definition),
        Self::Route(RouteGroup::Directory),
        Self::Route(RouteGroup::Admin),
        Self::Route(RouteGroup::Other),
        Self::Aql,
    ];

    /// Returns the storage key, which is also the report's group name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Route(RouteGroup::Ehr) => "ehr",
            Self::Route(RouteGroup::Composition) => "composition",
            Self::Route(RouteGroup::Contribution) => "contribution",
            Self::Route(RouteGroup::Query) => "query",
            Self::Route(RouteGroup::Definition) => "definition",
            Self::Route(RouteGroup::Directory) => "directory",
            Self::Route(RouteGroup::Admin) => "admin",
            Self::Route(RouteGroup::Other) => "other",
            Self::Aql => "aql",
        }
    }

    /// Returns the series a storage key names, if it names one.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|series| series.as_str() == key)
    }
}

/// One series' counts over a window.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SeriesCounts {
    /// The count per latency bucket.
    pub buckets: [u64; BUCKETS],
    /// Responses with a 5xx status (route series only).
    pub errors_5xx: u64,
    /// Executions above the slow threshold (the AQL series only).
    pub slow: u64,
    /// The largest latency recorded, in milliseconds.
    pub max_ms: f64,
}

impl SeriesCounts {
    /// Returns the number of recordings.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.buckets
            .iter()
            .fold(0_u64, |sum, count| sum.saturating_add(*count))
    }

    /// Returns whether nothing was recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.total() == 0 && self.errors_5xx == 0 && self.slow == 0
    }

    /// Adds `other` into these counts.
    pub fn merge(&mut self, other: &Self) {
        for (sum, count) in self.buckets.iter_mut().zip(other.buckets) {
            *sum = sum.saturating_add(count);
        }
        self.errors_5xx = self.errors_5xx.saturating_add(other.errors_5xx);
        self.slow = self.slow.saturating_add(other.slow);
        self.max_ms = self.max_ms.max(other.max_ms);
    }

    /// Returns the `q` quantile in milliseconds, interpolated linearly inside
    /// its bucket; the `+Inf` bucket reaches up to the largest recording.
    #[must_use]
    pub fn quantile_ms(&self, q: f64) -> f64 {
        let total = self.total();
        if total == 0 {
            return 0.0;
        }
        let rank = q.clamp(0.0, 1.0) * to_f64(total);
        let mut cumulative = 0_u64;
        for (index, count) in self.buckets.iter().copied().enumerate() {
            if count == 0 {
                continue;
            }
            let before = cumulative;
            cumulative = cumulative.saturating_add(count);
            if to_f64(cumulative) >= rank {
                let lower = lower_bound_ms(index);
                let upper = upper_bound_ms(index, self.max_ms).max(lower);
                let fraction = ((rank - to_f64(before)) / to_f64(count)).clamp(0.0, 1.0);
                return report_ms((upper - lower).mul_add(fraction, lower));
            }
        }
        report_ms(self.max_ms)
    }

    /// Returns the histogram as the report carries it, each count capped.
    #[must_use]
    pub fn report_histogram(&self) -> [u64; BUCKETS] {
        self.buckets.map(|count| count.min(MAX_COUNT))
    }
}

/// The window's counts per series.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WindowCounts {
    /// The counts of each series with any recording.
    pub series: BTreeMap<Series, SeriesCounts>,
}

impl WindowCounts {
    /// Adds `counts` into `series`.
    pub fn add(&mut self, series: Series, counts: &SeriesCounts) {
        self.series.entry(series).or_default().merge(counts);
    }

    /// Adds every series of `other` into these counts.
    pub fn merge(&mut self, other: &Self) {
        for (series, counts) in &other.series {
            self.add(*series, counts);
        }
    }

    /// Returns whether no series recorded anything.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.series.values().all(SeriesCounts::is_empty)
    }

    /// Renders the report's `metrics` object.
    #[must_use]
    pub fn to_metrics(&self) -> Metrics {
        let route = |group: RouteGroup| {
            self.series
                .get(&Series::Route(group))
                .filter(|counts| counts.total() > 0)
                .map(|counts| RouteStats {
                    p50_ms: counts.quantile_ms(0.50),
                    p95_ms: counts.quantile_ms(0.95),
                    p99_ms: counts.quantile_ms(0.99),
                    errors_5xx: counts.errors_5xx.min(MAX_COUNT),
                    histogram: counts.report_histogram(),
                })
        };
        let requests = self
            .series
            .iter()
            .filter(|(series, _)| matches!(series, Series::Route(_)))
            .fold(0_u64, |sum, (_, counts)| sum.saturating_add(counts.total()));
        let aql = self.series.get(&Series::Aql).copied().unwrap_or_default();
        Metrics {
            window_24h: Window24h {
                requests_bucket: RequestsBucket::of(requests),
                routes: Routes {
                    ehr: route(RouteGroup::Ehr),
                    composition: route(RouteGroup::Composition),
                    contribution: route(RouteGroup::Contribution),
                    query: route(RouteGroup::Query),
                    definition: route(RouteGroup::Definition),
                    directory: route(RouteGroup::Directory),
                    admin: route(RouteGroup::Admin),
                    other: route(RouteGroup::Other),
                },
                aql: AqlStats {
                    executions: aql.total().min(MAX_COUNT),
                    slow: aql.slow.min(MAX_COUNT),
                    p95_ms: aql.quantile_ms(0.95),
                    histogram: aql.report_histogram(),
                },
            },
        }
    }
}

/// Records one request of the API tree.
///
/// `route_template` is the matched route template, never the request path.
pub fn record_request(route_template: &str, elapsed: Duration, status: StatusCode) {
    let cell = LIVE.cell(Series::Route(RouteGroup::of_template(route_template)));
    cell.record(micros(elapsed), status.is_server_error(), false);
}

/// Records one AQL execution.
pub fn record_aql(elapsed: Duration) {
    let us = micros(elapsed);
    let slow = us > LIVE.slow_aql_us.load(Ordering::Relaxed);
    LIVE.cell(Series::Aql).record(us, false, slow);
}

/// Sets the execution time above which an AQL execution counts as slow.
pub fn set_slow_aql_ms(ms: u64) {
    LIVE.slow_aql_us
        .store(ms.saturating_mul(1000), Ordering::Relaxed);
}

/// Takes this process's counts since the last take, leaving zeros behind.
#[must_use]
pub fn take_live() -> WindowCounts {
    let mut counts = WindowCounts::default();
    for series in Series::ALL {
        let taken = LIVE.cell(series).take();
        if !taken.is_empty() {
            counts.add(series, &taken);
        }
    }
    counts
}

/// Returns this process's counts since the last take, leaving them in place.
#[must_use]
pub fn peek_live() -> WindowCounts {
    let mut counts = WindowCounts::default();
    for series in Series::ALL {
        let peeked = LIVE.cell(series).peek();
        if !peeked.is_empty() {
            counts.add(series, &peeked);
        }
    }
    counts
}

/// Puts counts a failed flush took back into this process's window.
pub fn restore_live(counts: &WindowCounts) {
    for (series, taken) in &counts.series {
        LIVE.cell(*series).restore(taken);
    }
}

/// One series' live counters.
#[derive(Debug, Default)]
struct Cell {
    /// The count per latency bucket.
    buckets: [AtomicU64; BUCKETS],
    /// Responses with a 5xx status.
    errors_5xx: AtomicU64,
    /// Executions above the slow threshold.
    slow: AtomicU64,
    /// The largest recording, in microseconds.
    max_us: AtomicU64,
}

impl Cell {
    fn record(&self, us: u64, server_error: bool, slow: bool) {
        if let Some(bucket) = self.buckets.get(bucket_index(us)) {
            bucket.fetch_add(1, Ordering::Relaxed);
        }
        if server_error {
            self.errors_5xx.fetch_add(1, Ordering::Relaxed);
        }
        if slow {
            self.slow.fetch_add(1, Ordering::Relaxed);
        }
        self.max_us.fetch_max(us, Ordering::Relaxed);
    }

    fn take(&self) -> SeriesCounts {
        SeriesCounts {
            buckets: std::array::from_fn(|index| {
                self.buckets
                    .get(index)
                    .map_or(0, |bucket| bucket.swap(0, Ordering::Relaxed))
            }),
            errors_5xx: self.errors_5xx.swap(0, Ordering::Relaxed),
            slow: self.slow.swap(0, Ordering::Relaxed),
            max_ms: to_f64(self.max_us.swap(0, Ordering::Relaxed)) / 1000.0,
        }
    }

    fn peek(&self) -> SeriesCounts {
        SeriesCounts {
            buckets: std::array::from_fn(|index| {
                self.buckets
                    .get(index)
                    .map_or(0, |bucket| bucket.load(Ordering::Relaxed))
            }),
            errors_5xx: self.errors_5xx.load(Ordering::Relaxed),
            slow: self.slow.load(Ordering::Relaxed),
            max_ms: to_f64(self.max_us.load(Ordering::Relaxed)) / 1000.0,
        }
    }

    fn restore(&self, counts: &SeriesCounts) {
        for (bucket, count) in self.buckets.iter().zip(counts.buckets) {
            bucket.fetch_add(count, Ordering::Relaxed);
        }
        self.errors_5xx
            .fetch_add(counts.errors_5xx, Ordering::Relaxed);
        self.slow.fetch_add(counts.slow, Ordering::Relaxed);
    }
}

/// The process's live window.
#[derive(Debug)]
struct Live {
    ehr: Cell,
    composition: Cell,
    contribution: Cell,
    query: Cell,
    definition: Cell,
    directory: Cell,
    admin: Cell,
    other: Cell,
    aql: Cell,
    /// The slow-AQL threshold in microseconds; nothing counts as slow until
    /// the reporter sets it from the configuration.
    slow_aql_us: AtomicU64,
}

impl Live {
    const fn cell(&self, series: Series) -> &Cell {
        match series {
            Series::Route(RouteGroup::Ehr) => &self.ehr,
            Series::Route(RouteGroup::Composition) => &self.composition,
            Series::Route(RouteGroup::Contribution) => &self.contribution,
            Series::Route(RouteGroup::Query) => &self.query,
            Series::Route(RouteGroup::Definition) => &self.definition,
            Series::Route(RouteGroup::Directory) => &self.directory,
            Series::Route(RouteGroup::Admin) => &self.admin,
            Series::Route(RouteGroup::Other) => &self.other,
            Series::Aql => &self.aql,
        }
    }
}

/// The process-wide live window.
static LIVE: LazyLock<Live> = LazyLock::new(|| Live {
    ehr: Cell::default(),
    composition: Cell::default(),
    contribution: Cell::default(),
    query: Cell::default(),
    definition: Cell::default(),
    directory: Cell::default(),
    admin: Cell::default(),
    other: Cell::default(),
    aql: Cell::default(),
    slow_aql_us: AtomicU64::new(u64::MAX),
});

/// A duration in whole microseconds, saturating.
fn micros(elapsed: Duration) -> u64 {
    elapsed
        .as_secs()
        .saturating_mul(1_000_000)
        .saturating_add(u64::from(elapsed.subsec_micros()))
}

/// The bucket a recording of `us` microseconds falls in: the first whose upper
/// bound it does not exceed, else the `+Inf` bucket.
fn bucket_index(us: u64) -> usize {
    BUCKET_BOUNDS_MS
        .iter()
        .position(|bound| us <= u64::from(*bound).saturating_mul(1000))
        .unwrap_or(BUCKETS - 1)
}

/// The lower bound of bucket `index`, in milliseconds.
fn lower_bound_ms(index: usize) -> f64 {
    index
        .checked_sub(1)
        .and_then(|previous| BUCKET_BOUNDS_MS.get(previous))
        .map_or(0.0, |bound| f64::from(*bound))
}

/// The upper bound of bucket `index`, in milliseconds; the `+Inf` bucket ends
/// at the largest recording.
fn upper_bound_ms(index: usize, max_ms: f64) -> f64 {
    BUCKET_BOUNDS_MS
        .get(index)
        .map_or(max_ms, |bound| f64::from(*bound))
}

/// A latency as the report carries it: within the contract's bounds, to a
/// tenth of a millisecond.
fn report_ms(ms: f64) -> f64 {
    if ms.is_finite() {
        (ms.clamp(0.0, MAX_MS) * 10.0).round() / 10.0
    } else {
        0.0
    }
}

/// A count as a float, exact up to 2^53 and without an `as` cast.
fn to_f64(value: u64) -> f64 {
    let [b0, b1, b2, b3, b4, b5, b6, b7] = value.to_le_bytes();
    let low = u32::from_le_bytes([b0, b1, b2, b3]);
    let high = u32::from_le_bytes([b4, b5, b6, b7]);
    f64::from(high).mul_add(4_294_967_296.0, f64::from(low))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_map_to_their_group() {
        let base = "/ferroehr/rest/openehr/v1";
        let cases = [
            ("/ehr", RouteGroup::Ehr),
            ("/ehr/{ehr_id}/ehr_status/{version_uid}", RouteGroup::Ehr),
            (
                "/ehr/{ehr_id}/composition/{uid_based_id}",
                RouteGroup::Composition,
            ),
            (
                "/ehr/{ehr_id}/versioned_composition/{versioned_object_uid}/version",
                RouteGroup::Composition,
            ),
            (
                "/ehr/{ehr_id}/contribution/{contribution_uid}",
                RouteGroup::Contribution,
            ),
            ("/ehr/{ehr_id}/directory", RouteGroup::Directory),
            ("/query/aql", RouteGroup::Query),
            ("/query/{qualified_query_name}", RouteGroup::Query),
            (
                "/definition/query/{qualified_query_name}",
                RouteGroup::Definition,
            ),
            ("/definition/template/adl1.4", RouteGroup::Definition),
            ("/admin/ehr/{ehr_id}", RouteGroup::Admin),
            (
                "/admin/query/{qualified_query_name}/{version}",
                RouteGroup::Admin,
            ),
            ("/demographic/person/{uid_based_id}", RouteGroup::Other),
            ("/demographic/contribution", RouteGroup::Other),
            ("/terminology/{terminology_id}", RouteGroup::Other),
            ("/message/export/{ehr_id}", RouteGroup::Other),
        ];
        for (path, group) in cases {
            assert_eq!(RouteGroup::of_template(path), group, "{path}");
            assert_eq!(
                RouteGroup::of_template(&format!("{base}{path}")),
                group,
                "{base}{path}"
            );
        }
        assert_eq!(RouteGroup::of_template("unmatched"), RouteGroup::Other);
        assert_eq!(RouteGroup::of_template(""), RouteGroup::Other);
    }

    #[test]
    fn a_parameter_segment_never_decides_the_group() {
        assert_eq!(RouteGroup::of_template("/ehr/{admin}"), RouteGroup::Ehr);
        assert_eq!(RouteGroup::of_template("/{query}/x"), RouteGroup::Other);
    }

    #[test]
    fn recordings_land_in_inclusive_upper_buckets() {
        assert_eq!(bucket_index(0), 0);
        assert_eq!(bucket_index(5_000), 0);
        assert_eq!(bucket_index(5_001), 1);
        assert_eq!(bucket_index(5_000_000), 9);
        assert_eq!(bucket_index(5_000_001), 10);
        assert_eq!(bucket_index(u64::MAX), 10);
    }

    #[test]
    fn series_keys_round_trip() {
        for series in Series::ALL {
            assert_eq!(Series::from_key(series.as_str()), Some(series));
        }
        assert_eq!(Series::from_key("composition/{uid}"), None);
    }

    #[test]
    fn quantiles_interpolate_inside_their_bucket() {
        let mut counts = SeriesCounts::default();
        // 100 recordings, all in (10, 25] ms.
        if let Some(bucket) = counts.buckets.get_mut(2) {
            *bucket = 100;
        }
        counts.max_ms = 24.0;
        assert!((counts.quantile_ms(0.5) - 17.5).abs() < 1e-9);
        assert!((counts.quantile_ms(1.0) - 25.0).abs() < 1e-9);
        assert!(SeriesCounts::default().quantile_ms(0.95).abs() < f64::EPSILON);
    }

    #[test]
    fn the_inf_bucket_reaches_the_largest_recording() {
        let mut counts = SeriesCounts::default();
        if let Some(bucket) = counts.buckets.get_mut(10) {
            *bucket = 2;
        }
        counts.max_ms = 7_000.0;
        assert!((counts.quantile_ms(1.0) - 7_000.0).abs() < 1e-9);
        counts.max_ms = 10_000_000.0;
        assert!((counts.quantile_ms(1.0) - MAX_MS).abs() < 1e-9);
    }

    #[test]
    fn floats_are_exact_over_the_count_range() {
        assert!((to_f64(MAX_COUNT) - 10_000_000.0).abs() < f64::EPSILON);
        assert!((to_f64(1 << 40) - 1_099_511_627_776.0).abs() < f64::EPSILON);
    }

    #[test]
    fn rendering_omits_groups_without_requests_and_caps_counts() {
        let mut window = WindowCounts::default();
        let mut busy = SeriesCounts::default();
        if let Some(bucket) = busy.buckets.get_mut(0) {
            *bucket = MAX_COUNT + 5;
        }
        busy.errors_5xx = MAX_COUNT + 1;
        window.add(Series::Route(RouteGroup::Composition), &busy);
        let metrics = window.to_metrics();
        let routes = &metrics.window_24h.routes;
        assert!(routes.ehr.is_none());
        let composition = routes
            .composition
            .as_ref()
            .expect("composition is rendered");
        assert_eq!(composition.histogram.first(), Some(&MAX_COUNT));
        assert_eq!(composition.errors_5xx, MAX_COUNT);
        assert_eq!(
            metrics.window_24h.requests_bucket,
            RequestsBucket::AtLeast100k
        );
        assert_eq!(metrics.window_24h.aql.executions, 0);
    }
}
