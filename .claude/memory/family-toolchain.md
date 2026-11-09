---
name: family-toolchain
description: Every family Rust repository pins the newest stable Rust as its toolchain and declares the toolchain minus one minor as its MSRV; an MSRV is only ever raised (owner 2026-10-21)
metadata:
  type: feedback
---

Every FerroHEALTH Rust repository pins the newest stable Rust release in `rust-toolchain.toml` and declares the toolchain minus one minor as its workspace `rust-version`. On 2026-10-21 that was toolchain 1.99.1 and MSRV 1.98.

**Why:** on 2026-10-21 the orchestrator started lowering FerroBRIDGE's MSRV to fit a sibling's msrv lane, working from a stale idea of the newest release; Rust 1.99.1 had shipped on 2026-10-15, and the owner ordered the versions kept.

**How to apply:** check the newest stable release on <https://blog.rust-lang.org/> before any toolchain or MSRV decision. When a crate's `rust-version` is above a consumer's MSRV, raise the consumer. Never lower a published crate's MSRV.
