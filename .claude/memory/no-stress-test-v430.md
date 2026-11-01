---
name: no-stress-test-v430
description: Owner ruling 2026-09-13 — no `veredictum stress` (or any load measurement) runs during the v4.3.0 cycle; #3098 (LTO) moved to v4.3.1 rather than landing without numbers
metadata:
  type: feedback
---

The owner ruled mid-milestone (2026-09-13): "we will not do in this whole
milestone a stress test". #3098's acceptance needs a before/after stress on
one machine, so it moved to v4.3.1 (a new milestone the owner named) untouched instead of landing the profile
change on the Cargo book's general claim.

**Why:** measurement needs an idle box this environment cannot be, and the
owner does not want a CI-runner stand-in for it this cycle.

**How to apply:** do not propose, scope or run stress/perf measurement in
v4.3.0; an issue whose acceptance needs one is re-milestoned with the ruling
recorded on it, never closed on unmeasured numbers. (`veredictum stress` was
the instrument of that cycle; the measured lane is now `bash
scripts/conformance.sh perf`, #4452.) See
[[measurement-environment-discipline]].
