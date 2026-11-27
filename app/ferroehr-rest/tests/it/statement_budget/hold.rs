// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! How long every hot path holds each pool connection against how long its
//! statements run on it (#5774), over the paths and fixtures of
//! [`super::paths`].
//!
//! The pool marks each hold ([`ferroehr::db::acquire::HOLD`]), sqlx times
//! each statement (`sqlx::query`), and the server marks the stages of a
//! request ([`ferroehr::telemetry::stage`]). A connection held while no
//! statement runs is charged to the stage the request is in then. No openEHR
//! spec governs connection handling: our own design.

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ferroehr::db::acquire::HOLD;
use ferroehr::telemetry::stage::{Stage, TARGET};
use tracing::Instrument as _;
use tracing::field::{Field, Visit};
use tracing::span::Id;
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::registry::LookupSpan;

use super::drained;
use super::paths::{REQUESTS, World, exchange, order};

/// The span each measured request runs in.
const REQUEST: &str = "hold_ledger_request";

/// One mark of a request's timeline.
#[derive(Clone, Debug)]
enum Mark {
    Taken(&'static str),
    Back(&'static str),
    /// A statement ended, having run this long.
    Statement(Duration, String),
    Enter(usize),
    Exit(usize),
}

/// The marks of the request in flight.
#[derive(Clone, Default)]
struct Ledger(Arc<Mutex<Vec<(Instant, Mark)>>>);

/// Reads the `pool` field of a hold event and the `elapsed_secs` of a
/// statement.
#[derive(Default)]
struct Fields {
    pool: Option<&'static str>,
    elapsed: f64,
    taken: bool,
    statement: String,
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "db.statement"
            || (field.name() == "summary" && self.statement.is_empty())
        {
            self.statement = value
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(70)
                .collect();
        }
        if field.name() == "pool" {
            self.pool = ["clinical", "party", "audit"]
                .into_iter()
                .find(|known| *known == value)
                .or(Some("other"));
        }
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        if field.name() == "elapsed_secs" {
            self.elapsed = value;
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.taken = format!("{value:?}") == "taken";
        }
    }
}

/// Whether the span `id` runs inside a measured request.
fn in_request<S>(ctx: &Context<'_, S>, id: &Id) -> bool
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    ctx.span(id)
        .is_some_and(|span| span.scope().any(|span| span.name() == REQUEST))
}

impl<S> tracing_subscriber::Layer<S> for Ledger
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        if span.metadata().target() != TARGET || !in_request(&ctx, id) {
            return;
        }
        if let Some(at) = Stage::ALL.iter().position(|s| s.name() == span.name()) {
            self.0
                .lock()
                .unwrap()
                .push((Instant::now(), Mark::Enter(at)));
        }
    }

    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        if span.metadata().target() != TARGET || !in_request(&ctx, id) {
            return;
        }
        if let Some(at) = Stage::ALL.iter().position(|s| s.name() == span.name()) {
            self.0
                .lock()
                .unwrap()
                .push((Instant::now(), Mark::Exit(at)));
        }
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        let target = event.metadata().target();
        if target != HOLD && target != "sqlx::query" {
            return;
        }
        let Some(scope) = ctx.event_scope(event) else {
            return;
        };
        let names: Vec<&str> = scope.map(|span| span.name()).collect();
        if !names.contains(&REQUEST) || names.contains(&ferroehr::db::pool::SESSION_SETUP_SPAN) {
            return;
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        let mark = if target == HOLD {
            let pool = fields.pool.unwrap_or("other");
            if fields.taken {
                Mark::Taken(pool)
            } else {
                Mark::Back(pool)
            }
        } else {
            Mark::Statement(Duration::from_secs_f64(fields.elapsed), fields.statement)
        };
        self.0.lock().unwrap().push((Instant::now(), mark));
    }
}

/// What one request held: per pool the hold, the statements run while any
/// connection was held, and the idle hold charged to each stage.
#[derive(Clone, Debug, Default)]
struct Held {
    holds: Vec<(&'static str, Duration)>,
    statements: Duration,
    /// Indexed like [`Stage::ALL`], then one for no stage entered.
    idle: [Duration; Stage::ALL.len() + 1],
}

impl Held {
    fn hold(&self) -> Duration {
        self.holds.iter().map(|(_, d)| *d).sum()
    }

    fn idle(&self) -> Duration {
        self.idle.iter().sum()
    }
}

/// Folds a request's `marks` into what it [`Held`].
fn fold(marks: &[(Instant, Mark)]) -> Held {
    let mut held = Held::default();
    let mut points: Vec<Instant> = Vec::new();
    let mut statements: Vec<(Instant, Instant)> = Vec::new();
    let mut open: Vec<(&'static str, Instant)> = Vec::new();
    let mut windows: Vec<(Instant, Instant)> = Vec::new();
    for (at, mark) in marks {
        points.push(*at);
        match mark {
            Mark::Taken(pool) => open.push((pool, *at)),
            Mark::Back(pool) => {
                if let Some(i) = open.iter().position(|(p, _)| p == pool) {
                    let (_, since) = open.swap_remove(i);
                    held.holds.push((pool, at.duration_since(since)));
                    windows.push((since, *at));
                }
            }
            Mark::Statement(elapsed, _) => {
                let start = at.checked_sub(*elapsed).unwrap_or(*at);
                statements.push((start, *at));
                points.push(start);
            }
            Mark::Enter(_) | Mark::Exit(_) => {}
        }
    }
    points.sort_unstable();
    let mut stages: Vec<usize> = Vec::new();
    let mut marks_at = marks.iter().peekable();
    for pair in points.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        while let Some((at, mark)) = marks_at.peek() {
            if *at > from {
                break;
            }
            match mark {
                Mark::Enter(stage) => stages.push(*stage),
                Mark::Exit(stage) => {
                    if let Some(i) = stages.iter().rposition(|s| s == stage) {
                        stages.remove(i);
                    }
                }
                _ => {}
            }
            marks_at.next();
        }
        let mid = from + to.duration_since(from) / 2;
        let holding = windows.iter().any(|(s, e)| *s <= mid && mid < *e);
        if !holding {
            continue;
        }
        let running = statements.iter().any(|(s, e)| *s <= mid && mid < *e);
        let span = to.duration_since(from);
        if running {
            held.statements += span;
        } else {
            let at = stages.last().copied().unwrap_or(Stage::ALL.len());
            held.idle[at] += span;
        }
    }
    held
}

/// The idle stretches over `min` of a request's holds, each with the stage
/// and the statements around it.
fn gaps(marks: &[(Instant, Mark)], min: Duration) -> Vec<String> {
    let mut out = Vec::new();
    let mut last: Option<(Instant, String)> = None;
    let mut held: Vec<&'static str> = Vec::new();
    let mut stages: Vec<usize> = Vec::new();
    let mut prev = String::new();
    let mut pending: Vec<usize> = Vec::new();
    for (at, mark) in marks {
        if let Mark::Statement(_, text) = mark {
            for i in pending.drain(..) {
                let line: &mut String = &mut out[i];
                write!(line, " | next stmt `{text}`").unwrap();
            }
        }
        let (start, what) = match mark {
            Mark::Statement(e, text) => (at.checked_sub(*e).unwrap_or(*at), text.clone()),
            Mark::Back(p) => (*at, format!("[give back {p}]")),
            Mark::Taken(p) => (*at, format!("[take {p}]")),
            Mark::Enter(s) => (*at, format!("<{}>", Stage::ALL[*s].name())),
            Mark::Exit(s) => (*at, format!("</{}>", Stage::ALL[*s].name())),
        };
        if let Some((end, before)) = &last
            && !held.is_empty()
            && start > *end
            && start.duration_since(*end) >= min
        {
            let stage = stages.last().map_or("none", |s| Stage::ALL[*s].name());
            out.push(format!(
                "    {} µs idle holding {:?} in {stage}: after `{before}` before `{what}` | prev stmt `{prev}`",
                start.duration_since(*end).as_micros(),
                held
            ));
            pending.push(out.len() - 1);
        }
        match mark {
            Mark::Taken(p) => held.push(p),
            Mark::Back(p) => {
                if let Some(i) = held.iter().position(|h| h == p) {
                    held.remove(i);
                }
            }
            Mark::Enter(s) => stages.push(*s),
            Mark::Exit(s) => {
                if let Some(i) = stages.iter().rposition(|x| x == s) {
                    stages.remove(i);
                }
            }
            Mark::Statement(_, text) => prev.clone_from(text),
        }
        last = Some((*at, what));
    }
    out
}

/// The median of `figures`, in microseconds.
fn median(figures: impl Iterator<Item = Duration>) -> u128 {
    let mut figures: Vec<u128> = figures.map(|d| d.as_micros()).collect();
    figures.sort_unstable();
    figures
        .get(figures.len().div_euclid(2))
        .copied()
        .unwrap_or(0)
}

/// No hot path keeps a connection while it decodes its body or encodes its
/// answer, and the report gives each path's holds against its statements, by
/// stage.
#[tokio::test]
async fn no_hot_path_keeps_a_connection_through_its_decode_or_encode() {
    let ledger = Ledger::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(ledger.clone()));
    let (_db, pool, svc, app, _pools) = Box::pin(super::steady()).await;
    let world = World::build(&app).await;
    let mut report = String::from("path | hold µs | statements µs | idle µs | ");
    for stage in Stage::ALL {
        write!(report, "{} | ", stage.name()).unwrap();
    }
    report.push_str("none | holds\n");
    let mut detail = String::new();
    for path in order() {
        world.prepare(&app, &svc, path).await;
        world.warm(&app, path).await;
        drained(&pool).await;
        let warm = usize::try_from(super::paths::WARM).unwrap();
        let mut each = Vec::new();
        for n in 0..usize::try_from(REQUESTS).unwrap() {
            let n = warm + n;
            let call = world.call(path, n);
            ledger.0.lock().unwrap().clear();
            let (status, headers, body) = exchange(&app, call.request())
                .instrument(tracing::info_span!(REQUEST))
                .await;
            assert!(
                status.is_success(),
                "{}: {status} {}",
                path.name(),
                String::from_utf8_lossy(&body)
            );
            world.observe(path, n, &headers);
            let marks = std::mem::take(&mut *ledger.0.lock().unwrap());
            let held = fold(&marks);
            for stage in [Stage::Decode, Stage::Encode] {
                let at = Stage::ALL.iter().position(|s| *s == stage).unwrap();
                assert!(
                    held.idle[at].is_zero(),
                    "{}: a connection was kept through the {} stage: {:?}",
                    path.name(),
                    stage.name(),
                    gaps(&marks, Duration::ZERO)
                );
            }
            each.push(held);
            if n == warm + 5 {
                for gap in gaps(&marks, Duration::from_micros(300)) {
                    writeln!(detail, "{}: {gap}", path.name()).unwrap();
                }
            }
            drained(&pool).await;
        }
        write!(
            report,
            "{} | {} | {} | {} | ",
            path.name(),
            median(each.iter().map(Held::hold)),
            median(each.iter().map(|h| h.statements)),
            median(each.iter().map(Held::idle)),
        )
        .unwrap();
        for at in 0..=Stage::ALL.len() {
            write!(report, "{} | ", median(each.iter().map(|h| h.idle[at]))).unwrap();
        }
        let holds: Vec<String> = each[each.len().div_euclid(2)]
            .holds
            .iter()
            .map(|(p, d)| format!("{p} {}", d.as_micros()))
            .collect();
        writeln!(report, "{}", holds.join(", ")).unwrap();
    }
    eprintln!("connection holds per request, the median of {REQUESTS}, in µs\n{report}\n{detail}");
}

/// The holds of every request of a burst, keyed by the request's span and
/// the pool, from the moment it took a connection to the moment it gave it
/// back.
#[derive(Clone, Default)]
struct Burst(Arc<Mutex<BurstHolds>>);

#[derive(Default)]
struct BurstHolds {
    open: Vec<(u64, &'static str, Instant)>,
    done: Vec<(&'static str, Duration)>,
}

impl<S> tracing_subscriber::Layer<S> for Burst
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        if event.metadata().target() != HOLD {
            return;
        }
        let Some(request) = ctx
            .event_scope(event)
            .and_then(|mut scope| scope.find(|span| span.name() == REQUEST))
        else {
            return;
        };
        let key = request.id().into_u64();
        let mut fields = Fields::default();
        event.record(&mut fields);
        let pool = fields.pool.unwrap_or("other");
        let now = Instant::now();
        let mut holds = self.0.lock().unwrap();
        if fields.taken {
            holds.open.push((key, pool, now));
        } else if let Some(at) = holds
            .open
            .iter()
            .position(|(k, p, _)| *k == key && *p == pool)
        {
            let (_, _, since) = holds.open.swap_remove(at);
            holds.done.push((pool, now.duration_since(since)));
        }
    }
}

/// The paths of the burst: the reads, queries and commits of a benchmark mix
/// whose requests can repeat.
const BURST_PATHS: [super::paths::Path; 7] = [
    super::paths::Path::ReadJson,
    super::paths::Path::EhrRead,
    super::paths::Path::ReadStatus,
    super::paths::Path::AqlSingle,
    super::paths::Path::AqlPopulation,
    super::paths::Path::PartyRead,
    super::paths::Path::CommitJson,
];

/// The requests of each path one wave of the burst sends at once.
const BURST_EACH: usize = 8;

/// The waves of the burst.
const BURST_WAVES: usize = 4;

/// The connections of each pool under the burst.
const BURST_POOL: u32 = 8;

/// A burst of the mix over pools smaller than it: every request answers, and
/// the report gives the distribution of how long each pool's connections
/// were held.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_burst_of_the_mix_answers_and_reports_its_holds() {
    let burst = Burst::default();
    tracing::subscriber::set_global_default(tracing_subscriber::registry().with(burst.clone()))
        .expect("the burst's subscriber is the process's only one");
    let (db, pool) = crate::common::migrated_pool().await;
    let settings = ferroehr::db::DbConfig {
        max_connections: Some(BURST_POOL),
        min_connections: Some(BURST_POOL),
        connection_budget: BURST_POOL * 3 + 8,
        acquire_timeout: ferrohealth_config::units::Duration::from_secs(30),
        ..ferroehr::db::DbConfig::new(db.url())
    };
    let pools = ferroehr::db::connect_domains(&settings)
        .await
        .expect("the domain pools connect");
    let (db, pool, svc, app) = Box::pin(crate::access_evaluator_http::app_over_pools(
        crate::party_binding_http::mapping(),
        db,
        pool,
        &pools,
    ))
    .await;
    super::listening(&db, &svc).await;
    let world = World::build(&app).await;
    for path in BURST_PATHS {
        world.prepare(&app, &svc, path).await;
        world.warm(&app, path).await;
    }
    drained(&pool).await;
    burst.0.lock().unwrap().done.clear();
    let started = Instant::now();
    for _ in 0..BURST_WAVES {
        let mut wave = tokio::task::JoinSet::new();
        for path in BURST_PATHS {
            for n in 0..BURST_EACH {
                let (app, call) = (app.clone(), world.call(path, n));
                wave.spawn(
                    async move {
                        let (status, _, body) = exchange(&app, call.request()).await;
                        (path, status, body)
                    }
                    .instrument(tracing::info_span!(REQUEST)),
                );
            }
        }
        while let Some(answered) = wave.join_next().await {
            let (path, status, body) = answered.expect("the request task");
            assert!(
                status.is_success(),
                "{}: {status} {}",
                path.name(),
                String::from_utf8_lossy(&body)
            );
        }
    }
    let elapsed = started.elapsed();
    let done = std::mem::take(&mut burst.0.lock().unwrap().done);
    let mut report = format!(
        "{} requests in {} waves over pools of {BURST_POOL} took {} ms\n\
         pool | holds | p50 µs | p90 µs | p99 µs | max µs | sum ms\n",
        BURST_PATHS.len() * BURST_EACH * BURST_WAVES,
        BURST_WAVES,
        elapsed.as_millis()
    );
    for name in ["audit", "clinical", "party", "other"] {
        let mut held: Vec<u128> = done
            .iter()
            .filter(|(pool, _)| *pool == name)
            .map(|(_, d)| d.as_micros())
            .collect();
        held.sort_unstable();
        let at = |q: usize| {
            held.get(
                (held.len() * q)
                    .div_euclid(100)
                    .min(held.len().saturating_sub(1)),
            )
        };
        writeln!(
            report,
            "{name} | {} | {} | {} | {} | {} | {}",
            held.len(),
            at(50).copied().unwrap_or(0),
            at(90).copied().unwrap_or(0),
            at(99).copied().unwrap_or(0),
            held.last().copied().unwrap_or(0),
            held.iter().sum::<u128>().div_euclid(1000)
        )
        .unwrap();
    }
    eprintln!("{report}");
}
