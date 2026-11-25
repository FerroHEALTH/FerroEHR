---
name: ci-under-20-minutes
description: "Owner 2026-11-09: every pull-request and merge-group run of ci.yml under 20 minutes, queue included; owner 2026-11-25: a merge-group run is light (build, guards, a smoke set), the full suite runs on the pull request"
metadata:
  type: feedback
---

The family target (owner, 2026-11-09): a pull-request run and a merge-group
run of `ci.yml` each finish in under 20 minutes, from the run's creation to
its conclusion, runner queue included.

The merge queue proves the merged tree builds, never the whole suite again
(owner ruling 2026-11-25): a `merge_group` run is light, with change
detection, rustfmt, lint, the guards, cargo-deny, the bin link, the test
archive and a smoke set of the unit tests and the server's wiring tests, and
`conclusion` judges the group on that set. The full test shards, perf, the
browser lanes, conformance, the deployment harness and chart boot run on the
pull request; a push to `main` the queue verified writes caches and
publishes (#6515).

**Why:** hosted runners are shared across the organisation, so every job a
run starts waits for a runner, and the queue tests one group after another.

**How to apply:** no gate is weakened; only the event it runs on changes.
Fewer, fatter jobs beat many tiny ones. A new long non-functional lane takes
the merge-group skip in the change that adds it.
