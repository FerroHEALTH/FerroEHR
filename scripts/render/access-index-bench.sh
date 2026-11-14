#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Render the access-index measurement table for the book FROM the committed
# record, docs/benchmarks/access-index/record.json (#5588).
#
# The stale-numbers gate (scripts/checks/conformance-numbers.sh) forbids a
# hand-typed size or latency in the site sources, so the security page
# includes this output instead; website/book/generated/ is render output and
# gitignored. The before rows were measured at the commit the record names,
# the after rows with app/ferroehr/benches/access_classes.rs.
#
# Usage: scripts/render/access-index-bench.sh [record.json] [out.md]
set -euo pipefail
cd "$(dirname "$0")/../.."
command -v jq >/dev/null || { echo "error: jq is required" >&2; exit 1; }

RECORD="${1:-docs/benchmarks/access-index/record.json}"
OUT="${2:-website/book/generated/access-index-bench.md}"
[[ -f "$RECORD" ]] || { echo "access-index-bench: $RECORD missing" >&2; exit 1; }
mkdir -p "$(dirname "$OUT")"

jq -r '
  def n: tostring | if length > 3 then (.[:-3] | n) + " " + .[-3:] else . end;
  def size: if . >= 1000000000 then ((. / 100000000 | round) / 10 | tostring) + " GB"
            elif . >= 1000000 then ((. / 1000000 | round) | tostring) + " MB"
            else ((. / 1000 | round) | tostring) + " kB" end;
  def ms: if . >= 1000 then ((. / 100 | round) / 10 | tostring) + " s"
          else (round | tostring) + " ms" end;
  def us: ((. / 1000 | round) | tostring) + " µs";
  .before.decision_ns as $d |
  (.after.rows | map({key: (.ehrs | tostring), value: .}) | from_entries) as $after |
  "| EHRs with settings | Whole-set copy | Its reload | Its condition per query | Classes now | Class copy | Condition per query now |",
  "|---|---|---|---|---|---|---|",
  (.before.rows[] | . as $b | $after[$b.ehrs | tostring] as $a |
    "| \($b.ehrs | n) | \($b.index_bytes | size) | \($b.reload_parse_ms | ms) | \(($d * 2 * $b.ehrs / 1000000) | ms) | \($a.classes) | \($a.class_json_bytes | size) | \($a.condition_ns | us) |"),
  "",
  "The per-EHR copy holds at most \(.after.copy_capacity | n) EHRs, about \((.after.copy_capacity * .after.entry_bytes) | size), whatever the number of EHRs with settings. A commit spends about \(.after.rows[-1].classify_ns_per_commit | us) classifying its settings, and the facts take \(.after.rows[-1].fact_bytes_per_ehr) bytes per EHR in the database. The reload column is the parse and sort alone, after the settings text was read from the database. Measured on an \(.environment.hardware_class) (\(.environment.memory_gb) GB), \(.environment.build) build, \(.environment.memory_measure); the before rows at commit `\(.before.measured_at_commit[0:12])`."
' "$RECORD" > "$OUT"
echo "access-index-bench: wrote $OUT from $RECORD"
