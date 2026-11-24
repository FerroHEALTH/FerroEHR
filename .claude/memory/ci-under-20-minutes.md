---
name: ci-under-20-minutes
description: "Owner 2026-11-09: every pull-request and merge-group run of ci.yml under 20 minutes, queue included; the merge queue proves the merged tree, so perf and the browser lanes skip merge_group; a push to main the queue verified only writes caches and publishes (#6515)"
metadata:
  type: feedback
---

The family target (owner, 2026-11-09): a pull-request run and a merge-group
run of `ci.yml` each finish in under 20 minutes, from the run's creation to
its conclusion, runner queue included.

The merge queue proves the merged tree, never the performance. Long
non-functional lanes (perf, the viewer journeys, the docs screenshots, the
viewer login smoke, fuzz) skip a `merge_group` run and run on the pull
request (#5284). A push to `main` the queue verified writes the caches pull
requests read and publishes, and nothing else (owner 2026-11-24, #6515).
`conclusion` names them in its merge-group summary. Functional lanes (tests,
clippy, codegen drift, conformance, the deployment harness, chart boot)
stay in the queue.

**Why:** hosted runners are shared across the organisation, so every job a
run starts waits for a runner, and the queue tests one group after another.

**How to apply:** no gate is weakened; only the event it runs on changes.
Fewer, fatter jobs beat many tiny ones. A new long non-functional lane takes
the merge-group skip in the change that adds it.
