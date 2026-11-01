---
name: measurement-environment-discipline
description: Measured runs need an idle box + compose limits matching the ixit envelope; lane renames must update ixit containers blocks
metadata: 
  node_type: memory
  type: project
  originSessionId: 3369ea8f-21b3-4a04-b632-fa9c6be28562
  modified: 2026-07-25T03:11:49.978Z
---

Two measurement-integrity traps hit on 2026-07-25 during the #261 ladder:

1. A stress knee halved (512→256/s) because (a) the docker-rework compose
   resource limits (2cpu/1Gi) contradicted the party ixit's declared 8-CPU/8GB
   measurement envelope, and (b) a subagent compiled Rust during the measured
   window. Compose core-service limits now default to the envelope
   (FERROEHR_CPUS/MEM overridable); production limits are Helm's concern.
   **Never run agents/builds concurrently with a measured perf window.**
2. Compose project-name changes rename containers: the ixit `containers`
   block (docs/conformance/party/ferroehr/ixit.json) pins the container names
   `veredictum perf` settles the database through (`docker exec vacuumdb`) and
   reads its server-side readings from. A rename leaves those readings absent.
   Check it after any lane or compose-project change.

**Why:** measured records are environment-bound and honest-publication gated;
a contaminated or env-shifted number would have been committed as a regression.

**How to apply:** before any `bash scripts/conformance.sh perf` or `perf-aa`
run (Veredictum 0.3.3's `perf` and `perf-compare --aa`; the 0.1 `stress` and
`aql-probe` instruments are gone): idle box (no agents, no builds), fresh
volumes (the stages compose them), ixit container names verified against
`docker ps`. Related: [[owner-work-style]].

**Update 2026-09-16:** the first generation-2 harness record was taken at a load average above 50 on the 8-core box (OneDrive File Provider, `fileproviderd` and the ESET scanner each at 50 to 70 % CPU after a `cargo clean` rebuilt 130 GB under `./target`), and read 10 of 12 operations as regressed by up to 360 %. Before any measurement: `uptime` 1-minute load under 2, `ps -Ao pcpu,comm -r | head` showing no sync or scanner at the top, the IDE quiet; otherwise wait or ask the owner to pause OneDrive and exclude `./target` from the scanner. Buffer counts from `EXPLAIN (ANALYZE, BUFFERS)` are the load-independent signal to compare first.
