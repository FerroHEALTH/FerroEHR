---
name: no-cnf-perf-runs-during-rewrite
description: Owner ruling 2026-09-15 — no CNF measured performance runs while the storage rewrite is in progress; the wire-level before/after runs once at the end against the published 4.3.0 image. Today the measured lane is `scripts/conformance.sh perf` / `perf-aa`
metadata:
  type: feedback
---

While the storage rewrite (#3337) is in progress, no measured run is started (then a `CONF_PERF_CLASS` class run; since Veredictum 0.3.3 the `perf` and `perf-aa` stages of `scripts/conformance.sh`, #4452); the owner stopped one mid-seed ("we are not doing this CNF one hour thing, not in this rewrite, at the end when we are fully finished"). The storage benchmark harness record (#3389) is the pre-rewrite baseline for the hypotheses; the wire-level comparison is run ONCE at the end, before and after in one session, with the published `ghcr.io/rubentalstra/ferroehr:4.3.0` image as the pre-rewrite SUT (`SKIP_BUILD`, images pinned). Class S is not measurable on this box anyway (10 M compositions to seed; the committed record only earned POC).

**Why:** the hour-plus exclusive runs block the checkout and the box for the rewrite's critical path, and the 4.3.0 image preserves the "before" side indefinitely.

**How to apply:** file wire-level measurement under the after-rewrite comparison (#3350), never as a prerequisite of an implementation issue; keep the box free for the workers. Related: [[rewrite-needs-baseline-and-harness]], [[measurement-environment-discipline]].


**Update 2026-09-16:** the owner ruled the hour-long wire runs out of the v4.3.1 cut as well ("we will not run the hour long test, that should move out of this milestone"); they live in #3452 (v4.3.2). Never schedule a measured session (`bash scripts/conformance.sh perf` or `perf-aa`) in a milestone the owner has not opened for it.
