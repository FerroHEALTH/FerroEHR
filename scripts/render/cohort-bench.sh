#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Render the cohort-query benchmark table for the book FROM the committed
# record, docs/benchmarks/cohort/record.json (#3159).
#
# The stale-numbers gate (scripts/checks/conformance-numbers.sh) forbids a
# hand-typed latency in the site sources, so the querying page includes this
# output instead; website/book/generated/ is render output and gitignored. The
# record is a pure function of one benchmark run and names the commit it was
# measured at, so this table is too.
#
# Usage: scripts/render/cohort-bench.sh [record.json] [out.md]
set -euo pipefail
cd "$(dirname "$0")/../.."
command -v jq >/dev/null || { echo "error: jq is required" >&2; exit 1; }

RECORD="${1:-docs/benchmarks/cohort/record.json}"
OUT="${2:-website/book/generated/cohort-bench.md}"
[[ -f "$RECORD" ]] || { echo "cohort-bench: $RECORD missing — run the ignored cohort_bench test first" >&2; exit 1; }
mkdir -p "$(dirname "$OUT")"

# One row per cohort; a latency renders in milliseconds below one second and
# in seconds above, with one decimal, so the column reads at a glance.
jq -r '
  def ms: if . >= 1000 then ((. / 100 | round) / 10 | tostring) + " s"
          else ((. | round) | tostring) + " ms" end;
  def n: tostring | if length > 3 then (.[:-3] | n) + " " + .[-3:] else . end;
  "| Cohort | p50 | p95 | Demographic statement | Clinical statement |",
  "|---|---|---|---|---|",
  (.cohorts[] | "| \(.size | n) EHRs | \(.p50_ms | ms) | \(.p95_ms | ms) | \(.predicate_plan.actual_total_ms | ms) | \(.clinical_plan.actual_total_ms | ms) |"),
  "",
  "Measured over \(.corpus.parties | n) parties, \(.corpus.ehrs | n) EHRs and \(.corpus.compositions | n) compositions at commit `\(.measured_at_commit[0:12])` on a \(.environment.hardware_class) (\(.environment.cores) cores, \(.environment.memory_gb) GB, \(.environment.storage_class)); the two statement columns are `EXPLAIN (ANALYZE)` times."
' "$RECORD" > "$OUT"
echo "cohort-bench: wrote $OUT from $RECORD"
