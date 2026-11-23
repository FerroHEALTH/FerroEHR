---
name: capacity-targets
description: FerroEHR's capacity targets: 4,000 to 5,000 rps on the own harness within the p99 targets, cdr-benchmark v0.19.0 at 8,000 ops/s, low CPU, memory and data, with the access path and log on
metadata:
  type: project
---

FerroEHR's capacity targets, one instance on 8 vCPU with the access check and
the signed access log on, as production runs:

- the own load harness: 4,000 to 5,000 requests per second on the realistic
  mix, within the p99 targets of reads 15 ms, writes 30 ms, single-EHR AQL
  20 ms and population AQL 100 ms;
- Cadasto cdr-benchmark v0.19.0: 8,000 operations per second;
- low CPU, memory and stored data per request ([[efficiency-first-rust]]).

A full redesign is allowed when it deletes work; it never removes a feature.

**Why:** the owner wants numbers that hold up beside other CDRs, measured as
production runs.

**How to apply:** measure capacity (open-loop, stepped arrival rate, the knee
where errors stay 0 and p99 stays inside the target) after each performance
change, on a quiet host ([[quiet-host-perf-runs]]). Report the knee per
request group with the hardware stated and name the bottleneck at the knee.
Never disable the access path or the log to look faster.
