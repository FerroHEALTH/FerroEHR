#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Render the care caseload figures for the book FROM the committed record,
# docs/benchmarks/care-caseload/record.json (#6418).
#
# Usage: scripts/render/care-caseload-bench.sh [record.json] [out.md]
set -euo pipefail
cd "$(dirname "$0")/../.."
command -v jq >/dev/null || { echo "error: jq is required" >&2; exit 1; }

RECORD="${1:-docs/benchmarks/care-caseload/record.json}"
OUT="${2:-website/book/generated/care-caseload-bench.md}"
[[ -f "$RECORD" ]] || { echo "care-caseload-bench: $RECORD missing" >&2; exit 1; }
mkdir -p "$(dirname "$OUT")"

jq -r '
  def n: tostring | if length > 3 then (.[:-3] | n) + "," + .[-3:] else . end;
  def ms: if . >= 1000 then ((. / 100 | round) / 10 | tostring) + " s"
          elif . >= 10 then (round | tostring) + " ms"
          elif . >= 1 then ((. * 10 | round) / 10 | tostring) + " ms"
          else ((. * 100 | round) / 100 | tostring) + " ms" end;
  "Measured by `\(.instrument)` on \(.environment), at commit `\(.measured_at_commit[0:12])`, in a store of \(.store.ehrs | n) EHRs with \(.store.compositions_per_ehr) compositions each.",
  "",
  "| Caseload | Check of one EHR | Caseload page of 50 | `COUNT` of the caseload'"'"'s compositions | Page under an episode | `COUNT` under an episode |",
  "| ---: | ---: | ---: | ---: | ---: | ---: |",
  (.rows[] | "| \(.patients | n) | \(.check_ms | ms) | \(.page_ms | ms) | \(.count_ms | ms) | \(.episode_page_ms | ms) | \(.episode_count_ms | ms) |")
' "$RECORD" > "$OUT"
echo "care-caseload-bench: wrote $OUT"
