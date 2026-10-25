---
name: slim-feature-lanes-gate-helpers
description: A pub(crate) helper consumed only from a feature-gated module (ferroehr-rest's `fhir`, `events`, `multimedia`) must carry the same `#[cfg(feature)]`, or the CI slim-server clippy lane fails on dead code; run the three `--no-default-features` combos locally before pushing rest/server changes
metadata:
  type: feedback
---

`ferroehr-rest` gates `extensions::fhir` (and `events`, `multimedia`) behind
cargo features, and CI's "clippy (slim server feature combos)" lane builds
`ferroehr-server` with `--no-default-features` (plus each feature alone) under
`-D warnings`. A `pub(crate)` helper added for a feature-gated consumer is
dead code in the slim build and fails that lane (hit 2026-09-11 on #3263: the
RBAC `holds_subject_audit_role` accessor, used only by the ITI-81 handler in
the `fhir` module).

The ITI-81 retrieval route and its local Audit Record Repository are an IHE
profile FerroEHR does not own under [[family-architecture]]: retrieval
belongs to the shared `ihe-atna` crate. Never extend that route; its removal
is tracker work.

**Why:** the full `--all-features` lane I run locally never sees the slim
shape, so the failure only surfaces in CI, one cycle later.

**How to apply:** when a change in `ferroehr-rest`/`ferroehr-server` adds an
item consumed from a gated module, put `#[cfg(feature = "…")]` on it (or use it
from an ungated path), and before pushing run the three slim combos:
`cargo clippy --locked -p ferroehr-server --no-default-features --all-targets -- -D warnings`
and the same with `--features events` and `--features fhir`. Related:
[[gate-parity-and-caller-sweeps]].
