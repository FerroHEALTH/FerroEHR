---
name: spec-over-test-suites
description: external conformance suites are regression signals, never the oracle; the specification decides (owner, 2026-11-16)
metadata:
  type: feedback
---

Never tell a worker to keep an external conformance suite (Cadasto
aql-conformance, cdr-benchmark checks, any third-party corpus) at 100%. The
specification is the oracle. A failing case is judged against the spec
section: if the suite is wrong, keep the spec behaviour, record the case with
its citation, and draft an upstream report. Owner, 2026-11-16: "it does not
need to stay at 100% if the AQL conformance is wrong".

**Why:** chasing a pass rate encodes another party's mistakes.

**How to apply:** phrase a suite in every brief as "no regression without a
spec-cited reason". Related: [[verify-before-implementing-old-issues]].
