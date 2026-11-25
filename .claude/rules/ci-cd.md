---
paths:
  - .github/workflows/**
  - .github/actions/**
  - scripts/lib/change-lanes.sh
  - scripts/lib/queue-runs.sh
---
# CI and CD: the standing rules

The pipeline and its tiers are described in `docs/ci-cd.md`. These are the
rules a workflow change keeps (owner decisions D-4 and D-5, 2026-11-24).

- **A required check is exact.** `conclusion` in `ci.yml` is the one required
  check. Every job it needs counts something a runner cannot vary: a test
  result, statements, WAL bytes, allocations, instructions, a diff. A job
  that reads a clock (thread CPU, wall time, latency, perf A/B judged on a
  hosted runner) never joins `conclusion`; it runs in `nightly.yml`.
- **The nightly tier is not a lesser gate.** Moving a lane there changes the
  event it runs on and nothing else: no threshold, ceiling, corpus or test
  changes with the move. A red scheduled run files an issue through
  `nightly-report.yml`, and that issue is worked like any other.
- **The merge queue proves `main`.** A push the queue verified writes caches
  and publishes; it re-runs no check. A new lane decides its tier in the
  change that adds it.
- **Concurrency matches the queue.** A merge-group run is never cancelled by
  a workflow's own group; only the queue cancels it. A pull request's newer
  push cancels its older run.
- **Labels switch heavy lanes on a pull request:** `ci:perf` for the timing
  lanes, `ci:heavy` for every nightly lane. A label never changes what
  `conclusion` needs.
