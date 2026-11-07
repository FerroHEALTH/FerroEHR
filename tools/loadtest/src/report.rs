// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The report a run writes: throughput and the p50, p95 and p99 latency over
//! the measured phase, the same per operation, and what `--audit` measured.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Context as _, bail};
use serde::Serialize;

use super::Op;

/// The nearest-rank `pct`th percentile of `sorted`, ascending.
pub(crate) fn percentile(sorted: &[Duration], pct: usize) -> Option<Duration> {
    let rank = sorted.len().saturating_mul(pct).div_ceil(100).max(1);
    sorted.get(rank.saturating_sub(1)).copied()
}

/// Milliseconds as a float, for the report.
pub(crate) fn millis(duration: Option<Duration>) -> f64 {
    duration.map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

/// One operation kind's figures.
#[derive(Debug, Serialize)]
pub(crate) struct OperationReport {
    pub(crate) count: usize,
    pub(crate) p50_ms: f64,
    pub(crate) p95_ms: f64,
    pub(crate) p99_ms: f64,
}

/// What one run measured.
#[derive(Debug, Serialize)]
pub(crate) struct Report {
    pub(crate) workers: usize,
    pub(crate) operations: usize,
    pub(crate) elapsed_seconds: f64,
    pub(crate) throughput_ops_per_second: f64,
    pub(crate) p50_ms: f64,
    pub(crate) p95_ms: f64,
    pub(crate) p99_ms: f64,
    pub(crate) by_operation: BTreeMap<&'static str, OperationReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) audit: Option<AuditReport>,
}

/// What the `--audit` run measured of the access log.
#[derive(Debug, Serialize)]
pub(crate) struct AuditReport {
    /// The ITS-REST operation id of each kind in the mix to the access records
    /// the measured window holds for it (ITI-81 `subtype`).
    pub(crate) records_by_operation: BTreeMap<&'static str, u64>,
    /// The operation id to the operations of the measured phase that ran it.
    pub(crate) operations_by_operation: BTreeMap<&'static str, u64>,
    /// Whether every operation has its record: a composition create or read
    /// exactly one, an AQL execution at least one (one per EHR it served).
    pub(crate) complete: bool,
    /// Seconds from the end of the measured phase until the counts settled.
    pub(crate) settled_after_seconds: f64,
    /// The paged ITI-81 retrievals over the measured window.
    pub(crate) retrieval: OperationReport,
    /// Records per retrieved page.
    pub(crate) page_size: usize,
}

impl Report {
    /// Summarises `timings`, measured over `elapsed` with `workers` workers.
    pub(crate) fn new(
        workers: usize,
        timings: &[(Op, Duration)],
        elapsed: Duration,
    ) -> anyhow::Result<Self> {
        let mut all: Vec<Duration> = timings.iter().map(|(_, d)| *d).collect();
        all.sort_unstable();
        let mut by_kind: BTreeMap<&'static str, Vec<Duration>> = BTreeMap::new();
        for (op, d) in timings {
            by_kind.entry(op.name()).or_default().push(*d);
        }
        let by_operation = by_kind
            .into_iter()
            .map(|(name, mut ds)| {
                ds.sort_unstable();
                let figures = OperationReport {
                    count: ds.len(),
                    p50_ms: millis(percentile(&ds, 50)),
                    p95_ms: millis(percentile(&ds, 95)),
                    p99_ms: millis(percentile(&ds, 99)),
                };
                (name, figures)
            })
            .collect();
        let seconds = elapsed.as_secs_f64();
        if seconds <= 0.0 {
            bail!("the measured phase took no time");
        }
        let count = f64::from(u32::try_from(all.len()).context("operation count exceeds u32")?);
        Ok(Self {
            workers,
            operations: all.len(),
            elapsed_seconds: seconds,
            throughput_ops_per_second: count / seconds,
            p50_ms: millis(percentile(&all, 50)),
            p95_ms: millis(percentile(&all, 95)),
            p99_ms: millis(percentile(&all, 99)),
            by_operation,
            audit: None,
        })
    }
}
