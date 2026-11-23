// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Criterion benches over the AQL front half (parse → plan → SQL build), and
//! a statement built against one bound from its shape, with per-bench CPU
//! flamegraphs.
//!
//! Plain measurement: `cargo bench -p ferroehr --bench aql`.
//! Flamegraphs: `cargo bench -p ferroehr --bench aql -- --profile-time 10`
//! writes `flamegraph.svg` per bench under
//! `target/criterion/<bench>/profile/` (criterion runs the registered
//! profiler only under `--profile-time` —
//! <https://docs.rs/criterion/latest/criterion/profiler/trait.Profiler.html>).

#![expect(
    clippy::expect_used,
    reason = "bench fixture setup and profiler I/O: a malformed fixture query \
              or an unwritable profile directory must abort the bench loudly \
              (there is no caller to return an error to)"
)]
#![expect(
    clippy::print_stderr,
    reason = "the bench harness is a dev binary; profiler lifecycle notes go \
              to the operator on stderr (no tracing subscriber is installed)"
)]

use std::fs::File;
use std::hint::black_box;
use std::path::Path;
use std::sync::Arc;

use criterion::profiler::Profiler;
use criterion::{Criterion, criterion_group, criterion_main};

use ferroehr::aql::ir::Params;
use ferroehr::aql::lineage::ArchetypeLineage;
use ferroehr::aql::sql::SqlCtx;
use openehr_query::parser;

/// A representative clinical query: EHR → COMPOSITION → archetyped
/// OBSERVATION chain, a typed magnitude predicate, ordering, and paging — it
/// exercises the CONTAINS planner, path typing, and the ORDER BY/limit
/// lowering in one shot.
const QUERY: &str = "SELECT e/ehr_id/value, c/uid/value, \
     o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude \
     FROM EHR e CONTAINS COMPOSITION c \
     CONTAINS OBSERVATION o [openEHR-EHR-OBSERVATION.blood_pressure.v2] \
     WHERE o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude > 120.0 \
     ORDER BY c/context/start_time/value DESC \
     LIMIT 50";

fn bench_ctx() -> SqlCtx {
    SqlCtx {
        system_id: "bench.ferroehr.org".to_owned(),
        ehr_ids: Vec::new(),
        ehr_scope_kind: ferroehr::aql::sql::EhrScopeKind::Named,
        ehr_access: ferroehr::access_eval::condition::AccessCondition::default(),
        limit: None,
        offset: None,
        archetype_lineage: Arc::new(ArchetypeLineage::default()),
        policy_parties: ferroehr::aql::sql::readers::PolicyParties::Served,
        access_settings: ferroehr::aql::sql::readers::EhrAccessReaders::Every,
        own_records: None,
        caseload: None,
    }
}

fn aql_front_half(c: &mut Criterion) {
    let params = Params::default();
    let ctx = bench_ctx();
    let ast = parser::parse_str(QUERY).expect("the fixture query must parse");
    let ir = ferroehr::aql::plan(
        &ast,
        &params,
        ferroehr::config::profile::SpecProfile::default(),
    )
    .expect("the fixture query must plan");

    c.bench_function("aql_parse", |b| {
        b.iter(|| parser::parse_str(black_box(QUERY)).expect("the fixture query must parse"));
    });
    c.bench_function("aql_plan", |b| {
        b.iter(|| {
            ferroehr::aql::plan(
                black_box(&ast),
                &params,
                ferroehr::config::profile::SpecProfile::default(),
            )
            .expect("the fixture query must plan")
        });
    });
    c.bench_function("aql_sql_build", |b| {
        b.iter(|| {
            ferroehr::aql::sql::build(black_box(&ir), &params, &ctx)
                .expect("the fixture query must lower to SQL")
        });
    });
    c.bench_function("aql_parse_plan_sql", |b| {
        b.iter(|| {
            let ast = parser::parse_str(black_box(QUERY)).expect("the fixture query must parse");
            let ir = ferroehr::aql::plan(
                &ast,
                &params,
                ferroehr::config::profile::SpecProfile::default(),
            )
            .expect("the fixture query must plan");
            ferroehr::aql::sql::build(&ir, &params, &ctx)
                .expect("the fixture query must lower to SQL")
        });
    });
}

/// The capacity test's single-EHR query (`scripts/perf/gate.sh`, `aql`): one
/// EHR named by `$ehr`, lab results ordered by time.
const PINNED: &str = "SELECT o/data[at0001]/events[at0002]/time/value, a/items[at0001]/value \
     FROM EHR e CONTAINS COMPOSITION c \
     CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.laboratory_test_result.v1] \
     CONTAINS CLUSTER a[openEHR-EHR-CLUSTER.laboratory_test_analyte.v1] \
     WHERE e/ehr_id/value = $ehr \
     ORDER BY o/data[at0001]/events[at0002]/time/value DESC";

/// The request of [`PINNED`] for `ehr`: its parameter, and the context of a
/// caller whose verdict on the EHR leaves one template out.
fn pinned_request(ehr: uuid::Uuid, lineage: &Arc<ArchetypeLineage>) -> (Params, SqlCtx) {
    use ferroehr::access_eval::condition::AccessCondition;
    use ferroehr::access_eval::condition::pinned::PinnedEhr;
    use ferroehr::access_eval::scope::{DataMatch, Polarity, Pred};
    let admitted = Pred::Not(Box::new(Pred::Data(
        Arc::new(DataMatch {
            templates: Some(vec!["hiv results".to_owned()]),
            ..DataMatch::default()
        }),
        Polarity::Deny,
    )));
    let condition = AccessCondition::of_parts(
        None,
        Some(Arc::new(PinnedEhr {
            ehr,
            admitted,
            settled: true,
            revision: ferroehr::storage::access_projection::Revision::default(),
        })),
    );
    let params = Params::default().with("ehr", ferroehr::aql::ir::ParamValue::Str(ehr.to_string()));
    let ctx = SqlCtx {
        ehr_access: condition,
        archetype_lineage: Arc::clone(lineage),
        ..bench_ctx()
    };
    (params, ctx)
}

/// The single-EHR statement built from its plan against the same statement
/// bound from its shape (#5783), each for a new EHR per iteration.
fn aql_statement_shape(c: &mut Criterion) {
    let ast = parser::parse_str(PINNED).expect("the pinned query must parse");
    let ir = ferroehr::aql::lower_query(&ast, ferroehr::config::profile::SpecProfile::default())
        .expect("the pinned query must lower");
    let lineage = Arc::new(ArchetypeLineage::default());
    let shapes = ferroehr::aql::sql::shape::Shapes::new(&ir);
    for _ in 0..2 {
        let (params, ctx) = pinned_request(uuid::Uuid::now_v7(), &lineage);
        shapes
            .prepare(&ir, &params, &ctx)
            .expect("the pinned query must prepare");
    }
    c.bench_function("aql_sql_build_pinned", |b| {
        b.iter_batched(
            || pinned_request(uuid::Uuid::now_v7(), &lineage),
            |(params, ctx)| {
                ferroehr::aql::sql::build(black_box(&ir), &params, &ctx)
                    .expect("the pinned query must lower to SQL")
            },
            criterion::BatchSize::SmallInput,
        );
    });
    c.bench_function("aql_sql_bind_pinned", |b| {
        b.iter_batched(
            || pinned_request(uuid::Uuid::now_v7(), &lineage),
            |(params, ctx)| {
                shapes
                    .prepare(black_box(&ir), &params, &ctx)
                    .expect("the pinned query must bind")
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

/// Criterion `Profiler` glue over the `pprof` sampler: start a guard when
/// criterion enters profile mode, render `flamegraph.svg` into the bench's
/// profile directory when it leaves.
struct FlamegraphProfiler {
    frequency: i32,
    guard: Option<pprof::ProfilerGuard<'static>>,
}

impl FlamegraphProfiler {
    const fn new(frequency: i32) -> Self {
        Self {
            frequency,
            guard: None,
        }
    }
}

impl Profiler for FlamegraphProfiler {
    fn start_profiling(&mut self, _benchmark_id: &str, _benchmark_dir: &Path) {
        self.guard = Some(
            pprof::ProfilerGuardBuilder::default()
                .frequency(self.frequency)
                // The unwind-hazard blocklist pprof's docs recommend
                // (<https://docs.rs/pprof/latest/pprof/>).
                .blocklist(&["libc", "libgcc", "pthread", "vdso"])
                .build()
                .expect("the pprof sampler must start"),
        );
    }

    fn stop_profiling(&mut self, benchmark_id: &str, benchmark_dir: &Path) {
        let Some(guard) = self.guard.take() else {
            return;
        };
        let report = guard.report().build().expect("the pprof report must build");
        std::fs::create_dir_all(benchmark_dir).expect("the profile directory must be creatable");
        let path = benchmark_dir.join("flamegraph.svg");
        let file = File::create(&path).expect("the flamegraph file must be creatable");
        // NOTE: #2406 — pprof's `flamegraph` feature pins inferno ^0.11
        // (quick-xml 0.26, RUSTSEC-2026-0194/0195); this is pprof 0.15's own
        // fold, rendered through the direct inferno 0.12 dependency instead.
        let lines: Vec<String> = report
            .data
            .iter()
            .map(|(frames, count)| {
                let mut segments = vec![frames.thread_name_or_id()];
                for frame in frames.frames.iter().rev() {
                    for symbol in frame.iter().rev() {
                        segments.push(symbol.to_string());
                    }
                }
                format!("{} {count}", segments.join(";"))
            })
            .collect();
        if !lines.is_empty() {
            inferno::flamegraph::from_lines(
                &mut inferno::flamegraph::Options::default(),
                lines.iter().map(String::as_str),
                file,
            )
            .expect("the flamegraph must render");
        }
        eprintln!("profiled {benchmark_id}: {}", path.display());
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default().with_profiler(FlamegraphProfiler::new(999));
    targets = aql_front_half, aql_statement_shape
}
criterion_main!(benches);
