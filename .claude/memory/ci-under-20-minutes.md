---
name: ci-under-20-minutes
description: "Owner 2026-11-09: every pull-request and merge-group run of ci.yml under 20 minutes, queue included; owner 2026-11-24: a required check is exact, so perf and the CPU budget run nightly or on a label; a verified push to main only writes caches and publishes"
metadata:
  type: feedback
---

The family target (owner, 2026-11-09): a pull-request run and a merge-group
run of `ci.yml` each finish in under 20 minutes, from the run's creation to
its conclusion, runner queue included.

The merge queue proves the merged tree, never the performance. A required
check is exact (owner decision D-4, 2026-11-24, #6517): perf A/B and the CPU
budget read a clock, so they run in `nightly.yml` on `main` and on a pull
request labelled `ci:perf` or `ci:heavy`, never in `conclusion`. The browser
lanes run on the pull request when the viewer changed and skip `merge_group`.
A push to `main` the queue verified writes caches and publishes, nothing else
(#6515). Functional lanes (tests, clippy, codegen drift, conformance, the
deployment harness, chart boot) stay in the queue. `docs/ci-cd.md` holds the
tiers.

**Why:** hosted runners are shared across the organisation, so every job a
run starts waits for a runner, and the queue tests one group after another.

**How to apply:** no gate is weakened; only the event it runs on changes.
Fewer, fatter jobs beat many tiny ones. A new long non-functional lane takes
the merge-group skip in the change that adds it.
