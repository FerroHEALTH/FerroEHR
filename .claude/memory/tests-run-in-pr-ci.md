---
name: tests-run-in-pr-ci
description: "Owner 2026-10-06 — never run the test suites (nextest/cargo test) locally; the machine lacks RAM; tests run in the PR's CI, locally only clippy/check/fmt/doc"
metadata:
  node_type: memory
  type: feedback
  originSessionId: 7514b6fd-a08b-4154-8327-9b3929e46f2e
  modified: 2026-10-06T14:17:46.049Z
---

Owner, 2026-10-06: "we test in the PR CI because our system does not have enough ram ... they need to stop these tests locally". Applies to the orchestrator and every subagent.

**Why:** the workspace test suites exhaust this machine's memory, especially with parallel workers and Docker running.
**How to apply:** locally run fmt, clippy (all CI lanes, `--all-targets` so test code compiles), `cargo check` and rustdoc only; push and let the PR's CI run nextest; read a red CI run's log and fix from it. Tell every implementation worker this in its brief. Supersedes the local nextest step in [[merge-on-local-gates]] and the gate list in CLAUDE.md for this machine.
