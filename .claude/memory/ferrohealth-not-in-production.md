---
name: ferrohealth-not-in-production
description: no FerroHEALTH product runs in production, so a wrong design is refactored or rewritten outright, with no compatibility layer or migration path (owner, 2026-11-19)
metadata:
  type: project
---

No one runs a FerroHEALTH product in production. When a design turns out
wrong or awkward, it is refactored or rewritten outright, correct from the
start: no compatibility layer, no migration path for old behaviour, and the
breaking change taken with the version bump. Owner, 2026-11-19: "major
refactor and change and rewrite because nobody is using it in production, so
we can do it correct from the start".

**Why:** with no installation to protect, a compatibility layer only keeps a
known-wrong design alive.

**How to apply:** brief workers that they may replace a model, schema or
configuration shape wholesale. Tests are still never weakened and the
specifications stay the oracle. Related: [[no-compat-shims]],
[[no-half-solutions]], [[new-design-no-legacy]].
