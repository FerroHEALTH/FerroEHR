// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The release performance guard's load test.
//!
//! It drives one running FerroEHR over the openEHR ITS-REST API with a fixed
//! workload and writes what it measured as one JSON report. The workload is
//! the operations of the 4.x to 5.x throughput regression: composition create
//! and read, an AQL query with `CONTAINS`, and an AQL query ordered by a
//! context path.
//!
//! A run seeds a fixed number of EHRs and compositions from the vendored
//! vital signs template, runs an unrecorded warm-up, and then runs a fixed
//! number of operations over a fixed number of concurrent workers. The count
//! is fixed, never the duration, so two servers measured with the same
//! arguments finish holding the same data. Any answer outside the expected
//! status fails the run: an error is never counted as a fast operation.
//!
//! No openEHR specification governs load testing: our own design. The
//! comparison against the previous release is `scripts/perf/compare.sh`.
//!
//! ```text
//! loadtest --out run.json
//! loadtest --base-url http://localhost:8080/ferroehr/rest/openehr/v1 --operations 4000 --out run.json
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use clap::Parser;
use reqwest::header::{ACCEPT, CONTENT_TYPE, LOCATION};
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use serde::Serialize;
use tokio::task::JoinSet;

/// The ADL 1.4 operational template every composition conforms to.
const TEMPLATE: &str = include_str!("../../../corpus/templates/ckm/vital-signs.opt");
/// The canonical JSON composition every write commits, its start time varied.
const COMPOSITION: &str = include_str!("../../../corpus/templates/ckm/vital-signs.example.json");
/// The context start time the example composition carries.
const EXAMPLE_START_TIME: &str = "2022-02-03T04:05:06Z";
/// 2026-01-01T00:00:00Z: the first start time a run writes, one minute apart after it.
const START_TIME_BASE: i64 = 1_767_225_600;

/// The query with a containment over every composition's observations.
const AQL_CONTAINS: &str = "SELECT c/uid/value, o/name/value \
     FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.blood_pressure.v2] \
     LIMIT 100";
/// The query ordered by a context path, newest first, one page.
const AQL_ORDER_BY: &str = "SELECT c/uid/value, c/context/start_time/value \
     FROM EHR e CONTAINS COMPOSITION c \
     ORDER BY c/context/start_time/value DESC \
     LIMIT 10";

/// The command line.
#[derive(Debug, Parser)]
#[command(
    version,
    about = "A fixed mixed ITS-REST and AQL workload against a running FerroEHR"
)]
struct Args {
    /// The ITS-REST base URL.
    #[arg(long, default_value = "http://localhost:8080/ferroehr/rest/openehr/v1")]
    base_url: String,
    /// The HTTP Basic user.
    #[arg(long, default_value = "ferroehr")]
    user: String,
    /// The HTTP Basic password.
    #[arg(long, default_value = "ferroehr")]
    password: String,
    /// EHRs created before the measured phase.
    #[arg(long, default_value_t = 20)]
    ehrs: usize,
    /// Compositions committed to each seeded EHR before the measured phase.
    #[arg(long, default_value_t = 10)]
    compositions_per_ehr: usize,
    /// Operations run, in the measured mix, before measuring starts.
    #[arg(long, default_value_t = 200)]
    warmup: usize,
    /// Operations measured.
    #[arg(long, default_value_t = 2000)]
    operations: usize,
    /// Concurrent workers.
    #[arg(long, default_value_t = 8)]
    workers: usize,
    /// Where the JSON report is written.
    #[arg(long)]
    out: PathBuf,
}

/// One kind of operation in the measured mix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    CompositionCreate,
    CompositionRead,
    AqlContains,
    AqlOrderBy,
}

impl Op {
    /// The operation the `i`th slot of the mix runs: of every 20, six creates,
    /// eight reads, three containment queries and three ordered queries.
    fn at(i: usize) -> Self {
        match i % 20 {
            0..=5 => Self::CompositionCreate,
            6..=13 => Self::CompositionRead,
            14..=16 => Self::AqlContains,
            _ => Self::AqlOrderBy,
        }
    }

    /// The operation's name in the report.
    fn name(self) -> &'static str {
        match self {
            Self::CompositionCreate => "composition_create",
            Self::CompositionRead => "composition_read",
            Self::AqlContains => "aql_contains",
            Self::AqlOrderBy => "aql_order_by",
        }
    }
}

/// The example composition split around its context start time.
#[derive(Debug)]
struct CompositionTemplate {
    before: String,
    after: String,
}

impl CompositionTemplate {
    /// Splits `text` at the start time value that follows its `start_time` key.
    fn parse(text: &str) -> anyhow::Result<Self> {
        let key = text
            .find("\"start_time\"")
            .ok_or_else(|| anyhow!("the example composition has no start_time"))?;
        let (_, rest) = text
            .split_at_checked(key)
            .ok_or_else(|| anyhow!("the start_time key is not at a character boundary"))?;
        let offset = rest.find(EXAMPLE_START_TIME).ok_or_else(|| {
            anyhow!("the example composition's start_time is not {EXAMPLE_START_TIME}")
        })?;
        let at = key.saturating_add(offset);
        let (before, tail) = text
            .split_at_checked(at)
            .ok_or_else(|| anyhow!("the start time offset is not a character boundary"))?;
        let (_, after) = tail
            .split_at_checked(EXAMPLE_START_TIME.len())
            .ok_or_else(|| anyhow!("the start time ends past the composition"))?;
        Ok(Self {
            before: before.to_owned(),
            after: after.to_owned(),
        })
    }

    /// The composition with the start time `minute` minutes after the base.
    fn render(&self, minute: i64) -> anyhow::Result<String> {
        let second = minute
            .checked_mul(60)
            .and_then(|s| s.checked_add(START_TIME_BASE))
            .ok_or_else(|| anyhow!("start time minute {minute} overflows"))?;
        let at = jiff::Timestamp::from_second(second)?;
        Ok(format!("{}{at}{}", self.before, self.after))
    }
}

/// The body of an AQL query request.
#[derive(Debug, Serialize)]
struct AqlRequest<'a> {
    q: &'a str,
}

/// The server under load and the records the workload reads.
#[derive(Debug)]
struct Target {
    client: Client,
    base: String,
    user: String,
    password: String,
    composition: CompositionTemplate,
    next_minute: AtomicI64,
    ehrs: Vec<String>,
    compositions: Vec<(String, String)>,
}

impl Target {
    /// An authenticated request to `path` below the base URL.
    fn request(&self, method: reqwest::Method, path: &str) -> RequestBuilder {
        self.client
            .request(method, format!("{}{path}", self.base))
            .basic_auth(&self.user, Some(&self.password))
    }

    /// Creates one EHR and returns its id.
    async fn create_ehr(&self) -> anyhow::Result<String> {
        let response = expect(
            "create EHR",
            self.request(reqwest::Method::POST, "/ehr")
                .header("Prefer", "return=minimal"),
            &[StatusCode::CREATED],
        )
        .await?;
        last_location_segment("create EHR", &response)
    }

    /// Uploads the operational template; a template already present is accepted.
    async fn upload_template(&self) -> anyhow::Result<()> {
        expect(
            "upload template",
            self.request(reqwest::Method::POST, "/definition/template/adl1.4")
                .header(CONTENT_TYPE, "application/xml")
                .body(TEMPLATE),
            &[
                StatusCode::CREATED,
                StatusCode::NO_CONTENT,
                StatusCode::CONFLICT,
            ],
        )
        .await?;
        Ok(())
    }

    /// Commits one composition to `ehr` and returns its version id.
    async fn create_composition(&self, ehr: &str) -> anyhow::Result<String> {
        let minute = self.next_minute.fetch_add(1, Ordering::Relaxed);
        let body = self.composition.render(minute)?;
        let response = expect(
            "create composition",
            self.request(reqwest::Method::POST, &format!("/ehr/{ehr}/composition"))
                .header(CONTENT_TYPE, "application/json")
                .header("Prefer", "return=minimal")
                .body(body),
            &[StatusCode::CREATED],
        )
        .await?;
        last_location_segment("create composition", &response)
    }

    /// Runs the `i`th slot of the mix and returns how long it took.
    async fn run(&self, i: usize) -> anyhow::Result<(Op, Duration)> {
        let op = Op::at(i);
        let started = Instant::now();
        match op {
            Op::CompositionCreate => {
                let ehr = pick(&self.ehrs, i)?;
                self.create_composition(ehr).await?;
            }
            Op::CompositionRead => {
                let (ehr, uid) = pick(&self.compositions, i)?;
                let response = expect(
                    "read composition",
                    self.request(
                        reqwest::Method::GET,
                        &format!("/ehr/{ehr}/composition/{uid}"),
                    )
                    .header(ACCEPT, "application/json"),
                    &[StatusCode::OK],
                )
                .await?;
                response.bytes().await.context("read composition body")?;
            }
            Op::AqlContains | Op::AqlOrderBy => {
                let q = if op == Op::AqlContains {
                    AQL_CONTAINS
                } else {
                    AQL_ORDER_BY
                };
                let response = expect(
                    op.name(),
                    self.request(reqwest::Method::POST, "/query/aql")
                        .header(ACCEPT, "application/json")
                        .json(&AqlRequest { q }),
                    &[StatusCode::OK],
                )
                .await?;
                response
                    .bytes()
                    .await
                    .with_context(|| format!("{} body", op.name()))?;
            }
        }
        Ok((op, started.elapsed()))
    }
}

/// The element of `items` the `i`th slot uses.
fn pick<T>(items: &[T], i: usize) -> anyhow::Result<&T> {
    i.checked_rem(items.len())
        .and_then(|at| items.get(at))
        .ok_or_else(|| anyhow!("the seed produced nothing to pick from"))
}

/// Sends `request` and refuses any status outside `accepted`, naming the
/// status and the start of the body.
async fn expect(
    what: &str,
    request: RequestBuilder,
    accepted: &[StatusCode],
) -> anyhow::Result<Response> {
    let response = request
        .send()
        .await
        .with_context(|| format!("{what}: request failed"))?;
    let status = response.status();
    if accepted.contains(&status) {
        return Ok(response);
    }
    let body = response
        .text()
        .await
        .unwrap_or_else(|e| format!("<unreadable body: {e}>"));
    let start: String = body.chars().take(2000).collect();
    bail!("{what}: the server answered {status}: {start}")
}

/// The last path segment of the response's `Location` header.
fn last_location_segment(what: &str, response: &Response) -> anyhow::Result<String> {
    response
        .headers()
        .get(LOCATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|location| location.rsplit('/').next())
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("{what}: the answer carries no usable Location header"))
}

/// Runs slots `0..count` over `workers` concurrent workers and returns every
/// operation's duration, failing on the first refused operation.
async fn drive(
    target: &Arc<Target>,
    workers: usize,
    count: usize,
) -> anyhow::Result<Vec<(Op, Duration)>> {
    let next = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let mut set = JoinSet::new();
    for _ in 0..workers.max(1) {
        let target = Arc::clone(target);
        let next = Arc::clone(&next);
        let stop = Arc::clone(&stop);
        set.spawn(async move {
            let mut timings = Vec::new();
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= count || stop.load(Ordering::Relaxed) {
                    return Ok(timings);
                }
                match target.run(i).await {
                    Ok(timing) => timings.push(timing),
                    Err(e) => {
                        stop.store(true, Ordering::Relaxed);
                        return Err(e);
                    }
                }
            }
        });
    }
    let mut all = Vec::with_capacity(count);
    while let Some(joined) = set.join_next().await {
        all.extend(joined.context("a worker panicked")??);
    }
    Ok(all)
}

/// The nearest-rank `pct`th percentile of `sorted`, ascending.
fn percentile(sorted: &[Duration], pct: usize) -> Option<Duration> {
    let rank = sorted.len().saturating_mul(pct).div_ceil(100).max(1);
    sorted.get(rank.saturating_sub(1)).copied()
}

/// Milliseconds as a float, for the report.
fn millis(duration: Option<Duration>) -> f64 {
    duration.map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

/// One operation kind's figures.
#[derive(Debug, Serialize)]
struct OperationReport {
    count: usize,
    p50_ms: f64,
    p95_ms: f64,
}

/// What one run measured.
#[derive(Debug, Serialize)]
struct Report {
    workers: usize,
    operations: usize,
    elapsed_seconds: f64,
    throughput_ops_per_second: f64,
    p50_ms: f64,
    p95_ms: f64,
    by_operation: BTreeMap<&'static str, OperationReport>,
}

impl Report {
    /// Summarises `timings`, measured over `elapsed` with `workers` workers.
    fn new(workers: usize, timings: &[(Op, Duration)], elapsed: Duration) -> anyhow::Result<Self> {
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
            by_operation,
        })
    }
}

/// Seeds the server, runs the warm-up and the measured phase, and writes the report.
async fn run(args: Args) -> anyhow::Result<()> {
    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .context("build the HTTP client")?;
    let mut target = Target {
        client,
        base: args.base_url.trim_end_matches('/').to_owned(),
        user: args.user,
        password: args.password,
        composition: CompositionTemplate::parse(COMPOSITION)?,
        next_minute: AtomicI64::new(0),
        ehrs: Vec::new(),
        compositions: Vec::new(),
    };
    target.upload_template().await?;
    for _ in 0..args.ehrs.max(1) {
        let ehr = target.create_ehr().await?;
        for _ in 0..args.compositions_per_ehr.max(1) {
            let uid = target.create_composition(&ehr).await?;
            target.compositions.push((ehr.clone(), uid));
        }
        target.ehrs.push(ehr);
    }
    let target = Arc::new(target);
    drive(&target, args.workers, args.warmup)
        .await
        .context("warm-up")?;
    let started = Instant::now();
    let timings = drive(&target, args.workers, args.operations)
        .await
        .context("measured phase")?;
    let report = Report::new(args.workers, &timings, started.elapsed())?;
    let json = serde_json::to_string_pretty(&report).context("serialise the report")?;
    std::fs::write(&args.out, json).with_context(|| format!("write {}", args.out.display()))?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run(Args::parse()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mix_holds_six_creates_eight_reads_and_three_of_each_query_per_twenty() {
        let ops: Vec<Op> = (0..20).map(Op::at).collect();
        let count = |op| ops.iter().filter(|o| **o == op).count();
        assert_eq!(count(Op::CompositionCreate), 6);
        assert_eq!(count(Op::CompositionRead), 8);
        assert_eq!(count(Op::AqlContains), 3);
        assert_eq!(count(Op::AqlOrderBy), 3);
        assert_eq!(Op::at(20), Op::at(0));
    }

    #[test]
    fn the_example_composition_takes_a_new_context_start_time() {
        let template = CompositionTemplate::parse(COMPOSITION).unwrap();
        let rendered = template.render(61).unwrap();
        assert!(
            rendered.contains("\"value\": \"2026-01-01T01:01:00Z\""),
            "{rendered}"
        );
        assert_eq!(
            rendered.matches(EXAMPLE_START_TIME).count(),
            COMPOSITION.matches(EXAMPLE_START_TIME).count() - 1
        );
        assert_eq!(rendered.len(), COMPOSITION.len());
    }

    #[test]
    fn a_composition_without_a_start_time_is_refused() {
        assert!(CompositionTemplate::parse("{\"_type\":\"COMPOSITION\"}").is_err());
    }

    #[test]
    fn the_percentile_is_the_nearest_rank() {
        let sorted: Vec<Duration> = (1..=100).map(Duration::from_millis).collect();
        assert_eq!(percentile(&sorted, 95), Some(Duration::from_millis(95)));
        assert_eq!(percentile(&sorted, 50), Some(Duration::from_millis(50)));
        assert_eq!(percentile(&sorted[..1], 95), Some(Duration::from_millis(1)));
        assert_eq!(percentile(&[], 95), None);
    }

    #[test]
    fn the_report_divides_the_operations_by_the_elapsed_time() {
        let timings: Vec<(Op, Duration)> = (0..40)
            .map(|i| (Op::at(i), Duration::from_millis(10)))
            .collect();
        let report = Report::new(4, &timings, Duration::from_secs(2)).unwrap();
        assert_eq!(report.operations, 40);
        assert!((report.throughput_ops_per_second - 20.0).abs() < 1e-9);
        assert_eq!(
            report.by_operation.get("composition_read").map(|r| r.count),
            Some(16)
        );
    }
}
