---
name: book-numbers-are-generated-includes
description: Any latency/throughput/conformance number on the website comes from a generator over a committed record into website/book/generated (gitignored) via {{#include}}; the stale-numbers gate refuses a hand-typed "543 ms" in the sources
metadata:
  type: feedback
---

The docs `build` job runs `scripts/checks/conformance-numbers.sh`, which fails
on any digit followed by `ms`/`/s`/`MB`/`GiB` or conformance-count vocabulary
in `website/landing`, `website/book/src` and `README.md`. A measured table
therefore never goes into a page by hand: write a small renderer under
`scripts/render/` (jq over the committed record under `docs/conformance/`),
have it write `website/book/generated/<name>.md` (render output, gitignored),
include it with `{{#include ../generated/<name>.md}}`, and register the
renderer in BOTH `scripts/site/build.sh` and `.github/workflows/docs-build.yml`
beside `perf-assets.sh`.

**Why:** the #3276 cohort-benchmark table turned the docs build red the first
time (2026-09-12); `perf-summary.md`, `conformance-stats.md` and the comparison
tables all follow this pattern, and a number typed into a page goes stale the
moment the record is re-measured.

**How to apply:** prose may say "under a second" or "100 000 parties" (no unit
suffix), never "2.6 s"; issue and PR comments are not gated. Run
`bash scripts/checks/conformance-numbers.sh` locally before pushing a docs
change that carries measurements. See [[gate-parity-and-caller-sweeps]].
