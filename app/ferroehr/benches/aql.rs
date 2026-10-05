// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Criterion benches over the AQL front half (parse → plan → SQL build), with
//! per-bench CPU flamegraphs.
//!
//! Plain measurement: `cargo bench -p ferroehr --bench aql`.
//! Flamegraphs: `cargo bench -p ferroehr --bench aql -- --profile-time 10`
//! writes `flamegraph.svg` per bench under
//! `target/criterion/<bench>/profile/` (criterion runs the registered
//! profiler only under `--profile-time` —
//! <https://docs.rs/criterion/latest/criterion/profiler/trait.Profiler.html>).
//!
//! NOTE: the profiler glue below implements criterion's `Profiler` trait over
//! the `pprof` sampler directly, because pprof's own `criterion` feature is
//! pinned to criterion ^0.5 (verified on crates.io 2026-08-04) and this
//! workspace is on criterion 0.8. The sampling and the flamegraph rendering
//! are entirely pprof + inferno; only this trait impl is ours.

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
        subject_scope: None,
        ehr_access: ferroehr::aql::sql::EhrAccessScope::Unrestricted,
        limit: None,
        offset: None,
        archetype_lineage: Arc::new(ArchetypeLineage::default()),
    }
}

fn aql_front_half(c: &mut Criterion) {
    let params = Params::new();
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
    targets = aql_front_half
}
criterion_main!(benches);
