// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The thread CPU of every hot path by stage (#6321): what the server's own
//! code costs a request, read with `clock_gettime(CLOCK_THREAD_CPUTIME_ID)`
//! on the thread that polls it, over the paths and fixtures of
//! [`super::paths`].
//!
//! The server marks the stages of a request ([`ferroehr::telemetry::stage`]),
//! and [`Stages`] charges the thread's CPU between two marks to the innermost
//! stage entered, so each stage counts its own work and none of the stages it
//! runs. A request's CPU is the CPU of its polls, the stages inside them and
//! the rest (routing, the middleware outside every stage, reading the body),
//! plus what the gathered access-log writer runs for it on its own task. A
//! wait on `PostgreSQL` costs a thread no CPU, so no stage holds one.
//!
//! Only the stage spans are enabled, so neither the console nor the
//! OpenTelemetry layers a server runs with are in the figures, and the
//! counting allocator of the allocation budget is installed in the test
//! binary. Each figure is the lower-quartile request of [`REQUESTS`], as the
//! allocation budget judges its figures (#6291). Runners differ in their CPU,
//! so each request is judged as a share of the [`Reference`] work timed on the
//! same thread right after it, so contention slows both alike (#6438), and a
//! path's share is the lower quartile of its requests' shares, against the
//! ceiling [`CPU_BUDGETS`] names. A path whose references read past
//! [`REFERENCE_BAND`] above the quietest path's was measured on a contended
//! runner, which fails the run as too noisy to judge. Only an optimised build
//! is judged, the `budget` profile CI's `cpu-budget` job runs: a debug build
//! distorts the share serde and the encoders take, so there the test reports
//! its figures and judges none.
//!
//! No openEHR spec governs the cost of a request: our own design.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};

use ferroehr::telemetry::stage::{Stage, TARGET};
use nix::time::{ClockId, clock_gettime};
use tracing::span::Id;
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::registry::LookupSpan;

use super::drained;
use super::paths::{Path, REQUESTS, World, exchange, order};

/// The most CPU each path's lower-quartile request may take, in thousandths
/// of the [`reference`] round trip timed on the same thread: the highest
/// share of three CI runs on main 7db1355587 plus the 50% they spread by at
/// most (the `EHR_STATUS` commit, the delete and the directory writes; every
/// other path within 30%).
const CPU_BUDGETS: [(Path, u64); 31] = [
    (Path::CommitJson, 7420),
    (Path::CommitXml, 9200),
    (Path::CommitFlat, 11_130),
    (Path::ReadJson, 975),
    (Path::ReadXml, 2125),
    (Path::ReadFlat, 2865),
    (Path::EhrRead, 760),
    (Path::AqlSingle, 1080),
    (Path::AqlPopulation, 1100),
    (Path::Iti81, 2400),
    (Path::PartyRead, 920),
    (Path::CommitContribution, 8440),
    (Path::ReadAtTime, 835),
    (Path::CommitStatus, 2220),
    (Path::ReadStatus, 690),
    (Path::CommitParty, 2640),
    (Path::AqlMatches, 1930),
    (Path::AqlParty, 1180),
    (Path::CommitPartyScored, 2880),
    (Path::ReadVersion, 840),
    (Path::CommitUpdate, 7520),
    (Path::CommitDelete, 2130),
    (Path::DirectoryCreate, 2250),
    (Path::DirectoryUpdate, 2270),
    (Path::DirectoryDelete, 2010),
    (Path::PartyUpdate, 2740),
    (Path::PartyDelete, 2380),
    (Path::CommitOrganisation, 2410),
    (Path::CommitGroup, 2430),
    (Path::CommitRole, 2950),
    (Path::CommitAgent, 2360),
];

/// How far, in thousandths, a path's [`Reference`] may read above the
/// quietest path's before its window counts as contended.
const REFERENCE_BAND: u64 = 1100;

/// The reference rounds timed after each measured request, of which the
/// fastest is its reference.
const REFERENCE_ROUNDS: usize = 3;

/// Whether the ceilings are judged: in an optimised build only.
const JUDGED: bool = !cfg!(debug_assertions);

/// How deep the stages of one thread nest at most; a deeper stage is charged
/// to the one it is entered in.
const DEPTH: usize = 32;

/// The stages a thread is in and the CPU it charged each, outside a request's
/// polls and inside them. It holds arrays only, so charging it allocates
/// nothing.
struct Ledger {
    /// Each entered stage with the CPU instant it was last charged up to.
    stack: [(usize, u64); DEPTH],
    depth: usize,
    /// The stages entered past [`DEPTH`] that are not yet left.
    deeper: usize,
    /// Whether the thread is polling the measured request.
    polling: bool,
    inside: [u64; Stage::ALL.len()],
    outside: [u64; Stage::ALL.len()],
    /// The clock reads the ledger made.
    reads: u64,
}

impl Ledger {
    const fn new() -> Self {
        Self {
            stack: [(0, 0); DEPTH],
            depth: 0,
            deeper: 0,
            polling: false,
            inside: [0; Stage::ALL.len()],
            outside: [0; Stage::ALL.len()],
            reads: 0,
        }
    }

    /// Charges the innermost stage up to `now`.
    fn charge(&mut self, now: u64) {
        let Some(top) = self.depth.checked_sub(1) else {
            return;
        };
        let (stage, since) = self.stack[top];
        let spent = now.saturating_sub(since);
        if self.polling {
            self.inside[stage] += spent;
        } else {
            self.outside[stage] += spent;
        }
        self.stack[top].1 = now;
    }

    fn enter(&mut self, stage: usize) {
        if self.depth == DEPTH {
            self.deeper += 1;
            return;
        }
        let now = self.now();
        self.charge(now);
        self.stack[self.depth] = (stage, now);
        self.depth += 1;
    }

    fn exit(&mut self) {
        if self.deeper > 0 {
            self.deeper -= 1;
            return;
        }
        let now = self.now();
        self.charge(now);
        self.depth = self.depth.saturating_sub(1);
        if let Some(top) = self.depth.checked_sub(1) {
            self.stack[top].1 = now;
        }
    }

    fn now(&mut self) -> u64 {
        self.reads += 1;
        thread_cpu()
    }

    /// What the ledger charged since the last take, which it forgets.
    fn take(&mut self) -> Charged {
        let charged = Charged {
            inside: self.inside,
            outside: self.outside,
            reads: self.reads,
            ..Charged::default()
        };
        self.inside = [0; Stage::ALL.len()];
        self.outside = [0; Stage::ALL.len()];
        self.reads = 0;
        charged
    }
}

thread_local! {
    static LEDGER: RefCell<Ledger> = const { RefCell::new(Ledger::new()) };
}

/// The CPU nanoseconds the running thread has used.
fn thread_cpu() -> u64 {
    let now = clock_gettime(ClockId::CLOCK_THREAD_CPUTIME_ID).expect("the thread CPU clock reads");
    u64::try_from(now.tv_sec()).unwrap() * 1_000_000_000 + u64::try_from(now.tv_nsec()).unwrap()
}

/// Times the stage spans with the CPU clock of the thread that enters them.
struct Stages;

impl Stages {
    /// The position in [`Stage::ALL`] of the stage `id` marks, when it marks
    /// one.
    fn of<S>(id: &Id, ctx: &Context<'_, S>) -> Option<usize>
    where
        S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    {
        let metadata = ctx.metadata(id)?;
        if metadata.target() != TARGET {
            return None;
        }
        let stage = Stage::named(metadata.name())?;
        Stage::ALL.iter().position(|s| *s == stage)
    }
}

impl<S> tracing_subscriber::Layer<S> for Stages
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    // NOTE: our own design (#6321), only the stage spans are enabled, so the
    // figures hold no telemetry a server's console filter would leave out.
    fn enabled(&self, metadata: &tracing::Metadata<'_>, _: Context<'_, S>) -> bool {
        metadata.target() == TARGET
    }

    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        if let Some(stage) = Self::of(id, &ctx) {
            LEDGER.with_borrow_mut(|ledger| ledger.enter(stage));
        }
    }

    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        if Self::of(id, &ctx).is_some() {
            LEDGER.with_borrow_mut(Ledger::exit);
        }
    }
}

/// The CPU of one request: its polls, and its stages inside them and
/// outside them, in nanoseconds.
#[derive(Default, Clone, Copy)]
struct Charged {
    polls: u64,
    inside: [u64; Stage::ALL.len()],
    outside: [u64; Stage::ALL.len()],
    reads: u64,
}

impl Charged {
    /// The request's CPU: its polls and what ran for it outside them.
    fn total(&self) -> u64 {
        self.polls + self.outside.iter().sum::<u64>()
    }

    /// `stage`'s CPU, inside the polls and outside them.
    fn stage(&self, at: usize) -> u64 {
        self.inside[at] + self.outside[at]
    }

    /// The CPU of the polls outside every stage.
    fn rest(&self) -> u64 {
        self.polls.saturating_sub(self.inside.iter().sum())
    }
}

/// A future whose polls are timed with the polling thread's CPU clock.
struct Timed<'a, F> {
    inner: Pin<Box<F>>,
    polls: &'a mut u64,
}

impl<F: Future> Future for Timed<'_, F> {
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<F::Output> {
        let this = &mut *self;
        LEDGER.with_borrow_mut(|ledger| ledger.polling = true);
        let started = thread_cpu();
        let out = this.inner.as_mut().poll(cx);
        *this.polls += thread_cpu().saturating_sub(started);
        LEDGER.with_borrow_mut(|ledger| {
            ledger.polling = false;
            ledger.reads += 2;
        });
        out
    }
}

/// The lower-quartile of `figures`, the third smallest of ten.
fn low(figures: impl Iterator<Item = u64>) -> u64 {
    let mut figures: Vec<u64> = figures.collect();
    figures.sort_unstable();
    figures
        .get(figures.len().div_euclid(4))
        .copied()
        .unwrap_or(0)
}

/// The lower-quartile of the nanosecond `figures`, in microseconds.
fn micros(figures: impl Iterator<Item = u64>) -> u64 {
    low(figures).div_euclid(1000)
}

/// The CPU nanoseconds one read of the thread CPU clock takes, the median of
/// a hundred runs of a thousand reads.
fn clock_read() -> u64 {
    let mut runs: Vec<u64> = (0..100)
        .map(|_| {
            let started = thread_cpu();
            for _ in 0..1000 {
                std::hint::black_box(thread_cpu());
            }
            thread_cpu().saturating_sub(started).div_euclid(1000)
        })
        .collect();
    runs.sort_unstable();
    runs[runs.len().div_euclid(2)]
}

/// The reference work: one serde round trip of the IPS composition's
/// canonical JSON (128 KB), which is CPU-bound and allocates as the paths do.
struct Reference {
    text: String,
}

impl Reference {
    fn new() -> Self {
        Self {
            text: crate::common::ips_canonical_composition().to_string(),
        }
    }

    /// The CPU nanoseconds of the fastest of [`REFERENCE_ROUNDS`] round trips.
    fn time(&self) -> u64 {
        (0..REFERENCE_ROUNDS)
            .map(|_| {
                let started = thread_cpu();
                let value: serde_json::Value = serde_json::from_str(&self.text).unwrap();
                std::hint::black_box(value.to_string());
                thread_cpu().saturating_sub(started)
            })
            .min()
            .unwrap()
    }
}

/// One measured request with the [`Reference`] timed right after it.
struct Sample {
    charged: Charged,
    reference: u64,
}

/// `nanos` in thousandths of `reference`.
fn share(nanos: u64, reference: u64) -> u64 {
    (nanos * 1000).div_euclid(reference.max(1))
}

/// Sends the [`REQUESTS`] measured requests of `path`, after its warm ones,
/// each followed by its [`Reference`].
async fn measure(
    app: &axum::Router,
    pool: &sqlx::PgPool,
    world: &World,
    reference: &Reference,
    path: Path,
) -> Vec<Sample> {
    world.warm(app, path).await;
    drained(pool).await;
    LEDGER.with_borrow_mut(Ledger::take);
    let warm = usize::try_from(super::paths::WARM).unwrap();
    let mut each = Vec::new();
    for n in 0..usize::try_from(REQUESTS).unwrap() {
        let n = warm + n;
        let call = world.call(path, n);
        let mut polls = 0;
        let (status, headers, body) = Timed {
            inner: Box::pin(exchange(app, call.request())),
            polls: &mut polls,
        }
        .await;
        assert!(
            status.is_success(),
            "{}: {status} {}",
            path.name(),
            String::from_utf8_lossy(&body)
        );
        world.observe(path, n, &headers);
        let mut charged = LEDGER.with_borrow_mut(Ledger::take);
        charged.polls = polls;
        each.push(Sample {
            charged,
            reference: reference.time(),
        });
        drained(pool).await;
        LEDGER.with_borrow_mut(Ledger::take);
    }
    each
}

/// Every hot path stays within its CPU budget, and the report names the CPU
/// of each stage.
#[tokio::test]
async fn every_hot_path_stays_within_its_cpu_budget() {
    let _guard = tracing::subscriber::set_default(tracing_subscriber::registry().with(Stages));
    ferroehr::aql::sql::without_reference_cells();
    let (_db, pool, svc, app) = Box::pin(super::served()).await;
    let world = World::build(&app).await;
    let read = clock_read();
    let reference = Reference::new();
    let mut measured = Vec::new();
    for path in order() {
        world.prepare(&app, &svc, path).await;
        let samples = Box::pin(measure(&app, &pool, &world, &reference, path)).await;
        let window = low(samples.iter().map(|s| s.reference));
        let each: Vec<Charged> = samples.iter().map(|s| s.charged).collect();
        let ratio = low(samples
            .iter()
            .map(|s| share(s.charged.total(), s.reference)));
        measured.push((path, window, ratio, each));
    }
    let quietest = measured
        .iter()
        .map(|(_, window, _, _)| *window)
        .min()
        .unwrap();
    let mut report = String::from("path | share ‰ | reference ‰ | total µs | ");
    for stage in Stage::ALL {
        write!(report, "{} | ", stage.name()).unwrap();
    }
    report.push_str("rest | clock reads\n");
    let mut over = Vec::new();
    let mut contended = Vec::new();
    for (path, window, ratio, each) in &measured {
        let (path, ratio) = (*path, *ratio);
        let total = low(each.iter().map(Charged::total));
        let moved = share(*window, quietest);
        let ceiling = CPU_BUDGETS
            .iter()
            .find(|(of, _)| *of == path)
            .map(|(_, ceiling)| *ceiling)
            .expect("every path has its CPU budget");
        if moved > REFERENCE_BAND {
            contended.push(format!("{}: {moved} ‰", path.name()));
        } else if JUDGED && ratio > ceiling {
            over.push(format!("{}: {ratio} ‰ (<= {ceiling})", path.name()));
        }
        write!(
            report,
            "{} | {ratio} (<= {ceiling}) | {moved} | {} | ",
            path.name(),
            total.div_euclid(1000)
        )
        .unwrap();
        for at in 0..Stage::ALL.len() {
            write!(report, "{} | ", micros(each.iter().map(|c| c.stage(at)))).unwrap();
        }
        let reads = low(each.iter().map(|c| c.reads));
        writeln!(
            report,
            "{} | {reads}",
            micros(each.iter().map(Charged::rest))
        )
        .unwrap();
    }
    let judged = if JUDGED {
        "judged"
    } else {
        "a debug build, reported and not judged"
    };
    eprintln!(
        "thread CPU per request by stage, the lower-quartile request of {REQUESTS} ({judged}); \
         one clock read costs {read} ns; the quietest path's reference took {} µs, and each \
         path's reference is in thousandths of it\n{report}",
        quietest.div_euclid(1000),
    );
    assert!(
        over.is_empty(),
        "a hot path exceeds its CPU budget: {over:?}\n{report}"
    );
    assert!(
        !JUDGED || contended.is_empty(),
        "the runner was too noisy to judge: these paths' references read above the quietest \
         path's by more than the band of {REFERENCE_BAND} ‰: {contended:?}\n{report}"
    );
    assert_eq!(
        CPU_BUDGETS.len(),
        Path::ALL.len(),
        "every path has its CPU budget"
    );
}
