---
name: flamegraph
description: >
  Generates a CPU flamegraph to find where the program spends its time —
  against the running server (the GET /management/flamegraph endpoint), a
  code path in isolation (the criterion benches under --profile-time), async
  span attribution (the tracing-flame layer via management.flame_file), or a
  whole local binary run (cargo flamegraph). Use when the user asks to
  profile, find hotspots, see "where the time goes", or before any
  optimization work (profile-first is the standing perf rule).
allowed-tools: [Read, Bash]
argument-hint: "[server | bench <name> | binary] [seconds] [frequency-hz]"
---

# /flamegraph

Four flamegraph instruments, all built on established crates (`pprof`
sampling, `tracing-flame` span capture, `inferno` rendering — never
hand-rolled). Pick by what you are profiling; every route ends in an SVG
whose **wide frames are the hotspots**.

No openEHR spec governs profiling — our own operational tooling (issue
#1861). User docs: `website/book/src/operations.md` §Profiling (operators)
and `website/book/src/contributing.md` §Profiling (developers).

## 1. The running server — `GET /management/flamegraph`

The production/staging/composed-stack instrument. Opt-in like the whole
management surface (`ferroehr::config::management`, default off), endpoint
key `management.endpoints.flamegraph`, caps under `[management.profiling]`
(`max_seconds` 30, `max_frequency` 999).

```bash
# enable on a dev/composed server (env grammar: FERROEHR__ + TOML path):
#   FERROEHR__MANAGEMENT__ENABLED=true
#   FERROEHR__MANAGEMENT__ENDPOINTS__FLAMEGRAPH=public   # dev only; admin_only in prod
curl -o /tmp/flamegraph.svg \
  "http://localhost:8080/management/flamegraph?seconds=10&frequency=99"
```

Wire semantics (pinned by `app/ferroehr-rest/tests/it/management.rs`):
`400` for a parameter beyond a cap (refused, never clamped) · `409` while
another sample window runs · `404` when not opted in. **Load matters**: an
idle server flamegraphs its own idling — drive the workload you are
diagnosing while the window samples (e.g. run the CNF stress instrument
concurrently).

Implementation seam, when extending: sampling logic in
`app/ferroehr-rest/src/extensions/management/flamegraph.rs`, handler +
`#[utoipa::path]` in `.../management/mod.rs`, config in
`app/ferroehr/src/config/management.rs` (+ the annotated template
`app/ferroehr/assets/ferroehr.default.toml`, the book pages, and the Helm
`values.yaml` — same-PR, per the configuration rule).

## 2. A code path in isolation — criterion benches

Every criterion bench emits a per-bench flamegraph under `--profile-time`
(the profiler only runs under that flag):

```bash
cargo bench -p ferroehr --bench aql -- --profile-time 10
# → target/criterion/<bench-name>/profile/flamegraph.svg
```

The pattern lives in `app/ferroehr/benches/aql.rs`: a small
`criterion::profiler::Profiler` impl over `pprof`. **Copy that impl into new
bench files** — do NOT enable pprof's own `criterion` feature: it is pinned
to criterion ^0.5 (verified on crates.io 2026-08-04) and type-mismatches our
criterion 0.8. New benches: `[[bench]] name = "…" harness = false` in the
crate's `Cargo.toml`, `criterion.workspace = true` + `pprof.workspace =
true` as dev-deps.

## 3. Async span attribution — the `tracing-flame` layer

A sampled stack under tokio often blames the executor's poll loop; a span
flame attributes wall time to the instrumented operation. Set
`management.flame_file` (config key, `[management]` section; env
`FERROEHR__MANAGEMENT__FLAME_FILE`) — the layer is installed only when set
(issue #1862; `ferroehr::telemetry::init`). Folded stacks render offline:

```bash
cargo install inferno
inferno-flamegraph < /tmp/ferroehr.folded > span-flame.svg
```

Diagnostic sessions only — the folded file grows with span traffic. The
flush guard rides `TelemetryGuard` (flushes at shutdown).

## 4. A whole local run — `cargo flamegraph`

Zero code changes; a dev TOOL (`cargo install flamegraph`), never a
dependency:

```bash
cargo flamegraph --bin ferroehr          # Linux (perf)
sudo cargo flamegraph --bin ferroehr     # macOS (dtrace needs elevation)
```

## Ground rules

- **Profile first, then optimize** — a perf change without a profile naming
  the hotspot is speculation (the standing perf rule: profile-first, one
  change per ladder, re-measured per release).
- pprof is Linux + macOS only (signal-based sampling) — matches our targets.
- A flamegraph is an exploration artifact, NEVER a conformance/perf record:
  committed performance claims come only from the CNF runner's measured
  instruments (`docs/conformance/`).
- The endpoint samples the WHOLE process; keep it `admin_only` outside dev
  and prefer the separate management port.
