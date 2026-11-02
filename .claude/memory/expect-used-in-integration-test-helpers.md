---
name: expect-used-in-integration-test-helpers
description: The test-only lint relaxations live once per test crate in tests/it/main.rs; a new tests/it file adds no header of its own
metadata:
  type: feedback
---

`clippy.toml`'s `allow-*-in-tests` reaches only `#[test]`-annotated
functions, so the helpers and async bodies of an integration suite under
`app/*/tests/it/` still trip `clippy::expect_used`, `panic`,
`indexing_slicing` and the rest.

**Why:** the owner approved (2026-11-02, #4767) one crate-level
`#![expect(…)]` per test binary, in `tests/it/main.rs`, with its reason, in
place of a header in every file. Library and app code keep every lint at its
tier.

**How to apply:** a new `tests/it/*.rs` file carries no `#![expect]` or
`#![allow]` header. A lint the main block does not list yet is added there,
once, and only when the test genuinely needs it. A lint that stops firing
anywhere in the binary fails `unfulfilled_lint_expectations` and comes out of
the block. Run the `--all-targets` clippy lane before nextest, since nextest
alone compiles fine. See [[slim-feature-lanes-gate-helpers]] and
[[gate-parity-and-caller-sweeps]].
