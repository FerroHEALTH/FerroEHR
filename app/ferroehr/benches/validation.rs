// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Criterion benches over the CPU a write path spends inline on its worker,
//! with per-bench flamegraphs: the commit-validation passes, and the operational
//! template ingestion a template upload runs before either of them can.
//!
//! `FerroEhrService::validate_composition_for_commit` runs two CPU-bound passes
//! on the tokio worker before a composition reaches storage: the RM +
//! terminology pass over the canonical instance, then the archetype-conformance
//! pass against the template's WebTemplate. Both are pure functions of the
//! composition and the template, so they bench without a database. The fixtures
//! are the corpus spread: a short medicines list, the International Patient
//! Summary the issue names (a ~200 kB instance against a ~1.9 MB operational
//! template), and the congenital-syphilis case investigation, which is the
//! largest form the CKM publishes and this repository vendors as its
//! large-payload scale probe (a ~539 kB instance).
//!
//! Plain measurement: `cargo bench -p ferroehr --bench validation`.
//! Flamegraphs: `cargo bench -p ferroehr --bench validation -- --profile-time 10`
//! writes `flamegraph.svg` per bench under
//! `target/criterion/<bench>/profile/` (criterion runs the registered
//! profiler only under `--profile-time` —
//! <https://docs.rs/criterion/latest/criterion/profiler/trait.Profiler.html>).
//!
//! NOTE: the profiler glue below implements criterion's `Profiler` trait over
//! the `pprof` sampler directly, because pprof's own `criterion` feature is
//! pinned to criterion ^0.5 (verified on crates.io 2026-08-04) and this
//! workspace is on criterion 0.8.

#![expect(
    clippy::expect_used,
    reason = "bench fixture setup and profiler I/O: a malformed fixture or an \
              unwritable profile directory must abort the bench loudly (there \
              is no caller to return an error to)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the bench feeds the validators \
              the same canonical fragment the commit seam carries"
)]
#![expect(
    clippy::print_stderr,
    reason = "the bench harness is a dev binary; profiler lifecycle notes go \
              to the operator on stderr (no tracing subscriber is installed)"
)]

use std::fs::File;
use std::hint::black_box;
use std::path::{Path, PathBuf};

use criterion::profiler::Profiler;
use criterion::{Criterion, criterion_group, criterion_main};
use openehr_sdt::flat::webtemplate::model::WebTemplate;
use serde_json::Value;

/// The vendored CKM corpus directory, anchored at the crate manifest so the
/// bench runs from any working directory.
fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/templates/ckm")
}

/// Load one corpus template's example composition and its WebTemplate.
fn fixture(stem: &str) -> (Value, WebTemplate) {
    let dir = corpus_dir();
    let example = std::fs::read_to_string(dir.join(format!("{stem}.example.json")))
        .expect("the corpus example composition must be readable");
    let composition: Value =
        serde_json::from_str(&example).expect("the corpus example must be canonical JSON");
    let xml = std::fs::read_to_string(dir.join(format!("{stem}.opt")))
        .expect("the corpus operational template must be readable");
    let opt = openehr_its::opt14::from_xml(&xml).expect("the corpus OPT must parse");
    let web_template = openehr_sdt::flat::webtemplate::builder::build_web_template(&opt)
        .expect("the corpus OPT must build into a WebTemplate");
    (composition, web_template)
}

/// The two inline passes, per fixture size, plus their sum — the CPU the
/// commit path spends on the worker before it touches the pool.
fn commit_validation(c: &mut Criterion) {
    for stem in [
        "medicines-list",
        "international-patient-summary",
        "congenital-syphilis-case-investigation",
    ] {
        let (composition, web_template) = fixture(stem);
        c.bench_function(&format!("validate_rm_terminology/{stem}"), |b| {
            b.iter(|| {
                openehr_sdt::rm_instance::validate_rm_and_terminology(black_box(&composition))
            });
        });
        c.bench_function(&format!("validate_archetype_conformance/{stem}"), |b| {
            b.iter(|| {
                openehr_sdt::flat::validation::validate_archetype_conformance(
                    black_box(&composition),
                    black_box(&web_template),
                )
            });
        });
        c.bench_function(&format!("validate_both_passes/{stem}"), |b| {
            b.iter(|| {
                let mut messages =
                    openehr_sdt::rm_instance::validate_rm_and_terminology(black_box(&composition));
                messages.extend(
                    openehr_sdt::flat::validation::validate_archetype_conformance(
                        black_box(&composition),
                        black_box(&web_template),
                    ),
                );
                messages
            });
        });
    }
}

/// Operational template ingestion: the XML parse and the WebTemplate build a
/// template upload runs, and the composition path runs on a cache miss.
fn template_ingestion(c: &mut Criterion) {
    let dir = corpus_dir();
    for stem in [
        "medicines-list",
        "international-patient-summary",
        "congenital-syphilis-case-investigation",
    ] {
        let xml = std::fs::read_to_string(dir.join(format!("{stem}.opt")))
            .expect("the corpus operational template must be readable");
        c.bench_function(&format!("opt_parse/{stem}"), |b| {
            b.iter(|| openehr_its::opt14::from_xml(black_box(&xml)).expect("the OPT must parse"));
        });
        let opt = openehr_its::opt14::from_xml(&xml).expect("the OPT must parse");
        c.bench_function(&format!("web_template_build/{stem}"), |b| {
            b.iter(|| {
                openehr_sdt::flat::webtemplate::builder::build_web_template(black_box(&opt))
                    .expect("the OPT must build into a WebTemplate")
            });
        });
    }
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
    targets = commit_validation, template_ingestion
}
criterion_main!(benches);
