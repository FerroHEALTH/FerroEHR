---
name: efficiency-first-rust
description: every product is stable, fast and low on storage, CPU and RAM; idiomatic Rust with no needless clones, allocations or layers of forwarding functions (owner, 2026-11-16)
metadata:
  type: feedback
---

The owner's bar for every product: stable, fast, small on disk, and low on
CPU and RAM. The code uses what Rust offers: no needless copying (`clone`,
`to_owned`, `collect` into temporaries), borrowing where lifetimes allow, no
layers of functions that only call functions, and no per-request allocation
that a reused buffer or a borrowed view avoids. Owner, 2026-11-16: "we need
stable, speed, and also low storage usages. but also low CPU and low RAM
usages. because we use RUST so we need to make sure that we do it as
efficient as possible".

**Why:** efficiency is a product property the owner measures, beside
correctness.

**How to apply:** every performance change reports CPU per request, RSS and
storage beside latency. Reviews look for copies and allocations on hot paths.
External suites (Cadasto aql-conformance, cdr-benchmark) are an outside
measure where they apply ([[spec-over-test-suites]]).
