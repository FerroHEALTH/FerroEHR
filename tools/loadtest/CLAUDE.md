# `loadtest`: the performance guard's load test (tooling, not part of the app)

One binary that drives a running FerroEHR over ITS-REST with a fixed workload
and writes one JSON report: throughput, p50 and p95 over the measured phase,
and the same figures per operation. `scripts/perf/ab.sh` runs it against the
release candidate and the previous release, and `scripts/perf/compare.sh`
turns the medians into the verdict (#4081).

- **HTTP only.** It never depends on `app/*`: the same binary has to measure
  a published image of an earlier release.
- **A fixed count, never a duration.** Both sides finish holding the same
  data, so a faster server is not penalised by a larger store.
- **An error fails the run.** A refused or failed operation stops the run
  with the status and the start of the body; it is never timed as a fast
  operation.
- **The workload changes deliberately.** Changing the mix, the queries or the
  seed changes what both sides measure, so the previous release is measured
  with the new workload in the same job; nothing compares across workload
  versions.
- **`--audit` measures the access log too** (EHDS Annex II 1.1, #4293): the
  measured window's records counted through ITI-81 per operation id, and
  paged retrieval timings. `scripts/perf/audit.sh` runs it with the log off
  and on, and `scripts/perf/audit-verdict.sh` holds the medians to the claims
  on the book's logging-performance page; change a budget there and in the
  script together.
- **Gates:** `cargo clippy -p loadtest --all-targets -- -D warnings`,
  `cargo nextest run -p loadtest`, `bash scripts/perf/compare.sh --self-test`, `bash scripts/perf/audit-verdict.sh --self-test`.
