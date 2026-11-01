# Benchmarks

FerroEHR measures performance with the **same instrument family that measures
conformance**: [Veredictum](https://github.com/Vernum-Projecten/Veredictum),
the separate CNF 2.0 runner described on [Conformance](conformance.md). There is no separate benchmark harness:
every published number is measured by a committed, re-runnable instrument,
reported in both directions, and regenerated from committed artefacts. There is no
chart in this project that you cannot regenerate yourself with one command.

<!-- toc -->

## The instruments

- **`perf`:** one pinned workload offered to a running server as open-loop
  Poisson arrivals, a scout ramp to estimate the knee, and a ladder of
  plateaus around it. The record gives the knee bracket, the peak throughput,
  the percentiles per operation with their resolvability, and server-side
  readings, and it earns no class. See [Performance](performance.md).
- **`perf-judge`:** re-derives every number of a record from its recorded
  histograms and fails when one byte differs.
- **`perf-compare`:** lines committed records up and names every way in which
  they describe different things, or runs two servers (`--ab`) or one server
  against itself (`--aa`) interleaved on one host. An A/A run measures the
  between-run noise floor a difference has to clear.
- **`perf-assets`:** draws the published charts from a committed record.

## What the workload simulates

The published record offers `clinical-mix`, Veredictum's reference workload:
chart review as short sessions on one EHR, ward boards, problem lists, cohort
counts, AQL over ad-hoc and stored queries, and the order, result, correction,
folder, tag, definition, demographic and bootstrap traffic around them, with
payloads built from published openEHR CKM templates. Every share names its
derivation. See [Performance](performance.md#the-workload).

## Running the instruments

```bash
# compose the measurement deployment, measure, re-derive, redraw the charts
bash scripts/conformance.sh perf

# the same, measured against itself interleaved: the record carries its noise floor
bash scripts/conformance.sh perf-aa

# two records side by side
source scripts/lib/veredictum.sh
"$(veredictum_bin)" perf-compare --result <a.json> --result <b.json>
```

Every stage seeds a freshly composed, empty server through the public API and
the stack is torn down afterwards. The committed record lands at
`docs/conformance/ferroehr/perf-result.json`; the published charts regenerate
from it (`scripts/render/perf-assets.sh`) and are diff-guarded in CI.

## Measurement rules

The measurement methodology is enforced by construction:

- **The same instrument measures every run** with the same pinned workload,
  each on its own freshly composed stack with the committed party statement and
  IXIT.
- **Database maintenance is settled deterministically before every measured
  window**, so no run pays another's maintenance debt or trips an autovacuum
  mid-measurement.
- **Rate limiting and response compression are off for measurement runs.**
  The scout ramp offers load past the knee on purpose, and `perf` writes no
  record if the server answered any throttled request; every posture profile
  declares compression off, and the posture canaries refuse a run that
  contradicts it.
- **The posture is checked, not assumed.** Canaries before and after the
  measured window check version signing, commit validation and authentication
  against the declared posture.
- **Configuration differences are explicit and labelled**, never quiet. Any
  setting raised or lowered for a measurement is a visible part of the composed
  posture, which is why the postures live in committed compose files rather than
  in a shell variable someone may or may not have exported.
