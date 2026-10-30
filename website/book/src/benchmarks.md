# Benchmarks

FerroEHR measures performance with the **same instrument family that measures
conformance**: [Veredictum](https://github.com/Vernum-Projecten/Veredictum),
the separate CNF 2.0 runner described on [Conformance](conformance.md). There is no separate benchmark harness:
every published number is measured by a committed, re-runnable instrument,
reported in both directions, and regenerated from committed artefacts. There is no
chart in this project that you cannot regenerate yourself with one command.

<!-- toc -->

## The three instruments

- **Measured class runs** (the runner's `perf` subcommand): conformance by
  measurement. The open-loop
  hospital-simulation workload holds a population-anchored offered-load floor for
  the normative hour (or an extended hold up to twelve hours), and the volumetric
  deployment class is **earned or not** from the committed record. Every
  measurement embeds re-checkable HDR histograms and per-container resource
  telemetry. See [Performance](performance.md).
- **The step-load stress ladder** (the `stress` subcommand): exploration. The
  same workload at geometrically climbing rates until the system leaves the
  envelope, then bisection to the **maximum sustainable throughput** (the knee of
  the latency-throughput curve). Each rung embeds its own histograms and resource
  telemetry; a rung where the load *generator* fell behind is flagged
  generator-bound, never counted against the server. A stress report earns no
  class and never touches the conformance record.
- **The AQL probe** (the `aql-probe` subcommand): diagnosis. The instrument's AQL
  set fired repeatedly against a freshly seeded corpus, with wire-latency
  percentiles and the database-side cost attributed per SQL statement. The
  optimization loop's entry point; exploration evidence only.

## What the workload simulates

All three instruments drive the same **hospital simulation**: clinical journeys
(admissions, shift vitals, medication rounds, laboratory results arriving
asynchronously, chart reviews, AQL ward dashboards, corrections, discharges)
expanded from a committed journey catalogue onto an open-loop arrival schedule,
with payloads built from **official openEHR CKM templates** vendored with
provenance. Every stage is its own planned arrival instant, so latency is measured
from the *planned* time (coordinated-omission-corrected) and a stalled server
cannot hide. The full workload story lives in
[Performance](performance.md#the-hospital-simulation).

## Running the instruments

```bash
# the measured class run
veredictum perf --root <perf-root> --ixit <ixit.json> --class POC --results <measured.json>

# the step-load stress ladder (fresh compose + seed, then the climb)
veredictum stress --root <catalogue-root> --ixit <ixit.json> --out <stress.json>

# the AQL optimization probe
veredictum aql-probe --root <catalogue-root> --ixit <ixit.json> --out <aql-probe.json>
```

Every instrument seeds a freshly composed, empty server through the public API and
the stack is torn down afterwards; there is no seed reuse, so no run ever
measures another run's leftovers. Committed records land under
`docs/conformance/ferroehr/`; the published charts regenerate from them
(`scripts/render/perf-assets.sh`) and are diff-guarded in CI.

## Measurement rules

The measurement methodology is enforced by construction:

- **The same runner measures every run** against the same committed catalogue
  and the same ladder, each on its own freshly composed stack with the committed
  party statement.
- **Database maintenance is settled deterministically before every measured
  window**, so no run pays another's maintenance debt or trips an autovacuum
  mid-measurement.
- **Rate limiting is off for measurement runs**, because the instruments
  deliberately offer load past the knee; both instruments refuse to write a
  record if the server answered any throttled request.
- **Configuration differences are explicit and labelled**, never quiet. Any
  setting raised or lowered for a measurement is a visible part of the composed
  posture, which is why the postures live in committed compose files rather than
  in a shell variable someone may or may not have exported.
