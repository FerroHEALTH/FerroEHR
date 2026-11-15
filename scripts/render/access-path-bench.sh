#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Render the access-path measurements for the book FROM the committed record,
# docs/benchmarks/access-path/record.json (#5590).
#
# The stale-numbers gate (scripts/checks/conformance-numbers.sh) forbids a
# hand-typed size or latency in the site sources, so the performance page
# includes this output instead; website/book/generated/ is render output and
# gitignored. The record names the instrument of each table: the performance
# gate against the previous release (scripts/perf/gate.sh ab), the evaluator
# micro-benchmark (app/ferroehr/benches/access_decision.rs), the replica copy
# (scaleseed replica), the access log's storage and the reload load of several
# instances (scripts/perf/gate.sh replicas).
#
# Usage: scripts/render/access-path-bench.sh [record.json] [out.md]
set -euo pipefail
cd "$(dirname "$0")/../.."
command -v jq >/dev/null || { echo "error: jq is required" >&2; exit 1; }

RECORD="${1:-docs/benchmarks/access-path/record.json}"
OUT="${2:-website/book/generated/access-path-bench.md}"
[[ -f "$RECORD" ]] || { echo "access-path-bench: $RECORD missing" >&2; exit 1; }
mkdir -p "$(dirname "$OUT")"

jq -r '
  def n: tostring | if length > 3 then (.[:-3] | n) + " " + .[-3:] else . end;
  def size: if . >= 1000000000000 then ((. / 100000000000 | round) / 10 | tostring) + " TB"
            elif . >= 1000000000 then ((. / 100000000 | round) / 10 | tostring) + " GB"
            elif . >= 1000000 then ((. / 100000 | round) / 10 | tostring) + " MB"
            elif . >= 1000 then ((. / 100 | round) / 10 | tostring) + " kB"
            else (round | tostring) + " B" end;
  def ms: if . >= 1000 then ((. / 100 | round) / 10 | tostring) + " s"
          elif . >= 10 then (round | tostring) + " ms"
          else ((. * 100 | round) / 100 | tostring) + " ms" end;
  def ns: if . >= 1000000 then (. / 1000000 | ms)
          elif . >= 1000 then ((. / 100 | round) / 10 | tostring) + " µs"
          else (round | tostring) + " ns" end;
  def x: ((. * 100 | round) / 100 | tostring) + "×";
  def verdict: if . then "pass" else "**breach**" end;
  .environment as $env |

  "### Against the previous release",
  "",
  "The gate (`scripts/perf/gate.sh ab`) over \(.gate.population | n) seeded EHRs, the head with every access-path feature in force, the base `\(.gate.base.image)` (commit `\(.gate.base.revision[0:12])`) under its own configuration. Head over base, each a median over the load phases:",
  "",
  "| Metric | \(.gate.runs | map(.label) | join(" | ")) |",
  "|---|\(.gate.runs | map("---:") | join("|"))|",
  ([.gate.runs[0].checks[].name] as $names | $names[] as $name |
    "| `\($name)` | \(.gate.runs | map(.checks[] | select(.name == $name) |
       "\(.head | if $name | test("_ms$") then ms elif $name | test("bytes") then size else ((. * 1000 | round) / 1000 | tostring) end) over \(.base | if $name | test("_ms$") then ms elif $name | test("bytes") then size else ((. * 1000 | round) / 1000 | tostring) end), \(.passed | verdict)") | join(" | ")) |"),
  "",
  "Each run signs every access-log link with the key its column names. No run adds a transaction per request. The breaches of the AQL groups and the access-log read are open as \(.gate.regressions | map("#\(.)") | join(" and ")), to be fixed at their cause; the RSA-3072 column shows the cost of an RSA key on every write. Against v5.7.1, whose binary refuses this configuration, each column is the whole cost of the access path and the signed access log, the accepted cost of the EHDS access log; the gate of a change compares both arms under one configuration. The run before the fix of the per-signature key derivation (#5616) measured \(.gate.before_fix.cpu_per_request_ms.head | ms) of CPU per request against \(.gate.before_fix.cpu_per_request_ms.base | ms) for the base, and an EHR read at \(.gate.before_fix.ehr_read_p50_ms.head | ms) against \(.gate.before_fix.ehr_read_p50_ms.base | ms).",
  "",

  "### The evaluator",
  "",
  "One decision of the fixed evaluator (`cargo bench -p ferroehr --bench access_decision`) for a professional under a policy that requires a care relationship and a declared purpose, median and 99th percentile of \(.evaluator.decisions | n) decisions:",
  "",
  "| Directives on the EHR | With a care relationship | Without one |",
  "|---|---|---|",
  (.evaluator.rows | group_by(.set) | sort_by(.[0].directives)[] |
    (map(select(.cared))[0]) as $c | (map(select(.cared | not))[0]) as $u |
    "| \($c.directives) (\($c.set)) | \($c.decision_ns_p50 | ns), p99 \($c.decision_ns_p99 | ns) | \($u.decision_ns_p50 | ns), p99 \($u.decision_ns_p99 | ns) |"),
  "",

  "### What a replica holds",
  "",
  "Patients under the care of \(.replica.teams) teams, with \(.replica.staff | n) professionals (`scaleseed replica`). The copy of the caller facts used to hold every patient; it now holds the professionals alone, and the care caches are bounded at \(.replica.subjects_cap | n) subjects and teams holding \(.replica.census_cap | n) patients together:",
  "",
  "| Patients in care | Copy before | Its reload | Team lookup before | Copy now | Its reload | Care caches filled to their bound | Lookup held / read |",
  "|---|---|---|---|---|---|---|---|",
  ((.replica.after.rows | map({key: (.patients | tostring), value: .}) | from_entries) as $after |
    .replica.before.rows[] | . as $b | $after[$b.patients | tostring] as $a |
    "| \($b.patients | n) | \($b.resident_bytes | size) | \($b.reload_ms | ms) | \($b.cared_for_by_ns | ns) | \($a.copy_bytes | size) | \($a.reload_ms | ms) | \($a.care_bytes | size) | \($a.care_hit_ns | ns) / \($a.care_miss_ns | ns) |"),
  "",
  "A held lookup touches no database; a lookup the caches miss is one indexed statement. The resident figures are the growth of a fresh process (\($env.memory_measure)); the before rows at commit `\(.replica.before.measured_at_commit[0:12])`.",
  "",

  "### The access log on disk",
  "",
  .access_log as $log |
  "With signed links, an entry is stored in \($log.entry_row_bytes | size) of row and \($log.stored_bytes_per_entry | size) with its indexes, and its tombstone in \($log.tombstone_row_bytes | size). The gate'"'"'s request set wrote \(($log.entries_per_request * 100 | round) / 100) entries per request. At \($log.rate_rps) requests a second that is \(($log.rate_rps * 86400 * $log.entries_per_request * $log.stored_bytes_per_entry) | size) a day of entries; a horizon of \($log.horizon_days | n) days holds \(($log.rate_rps * 86400 * $log.entries_per_request * $log.stored_bytes_per_entry * $log.horizon_days) | size) of them, and each entry past it leaves its tombstone, \(($log.rate_rps * 86400 * $log.entries_per_request * $log.tombstone_row_bytes) | size) a day, until the tombstones are removed too (#5619).",
  "",

  "### Several instances",
  "",
  "The reload load of one change on one PostgreSQL (`scripts/perf/gate.sh replicas`), counted over the \(.replicas.window_seconds) seconds after it, less the idle window:",
  "",
  "| Change | \(.replicas.rows | map(.instances) | unique | map("\(.) instance\(if . > 1 then "s" else "" end)") | join(" | ")) |",
  "|---|\(.replicas.rows | map(.instances) | unique | map("---") | join("|"))|",
  ((.replicas.rows | map(select(.change == "none")) | map({key: (.instances | tostring), value: .}) | from_entries) as $idle |
    (.replicas.rows | map(.change) | unique - ["none"])[] as $change |
    "| \($change) | \(.replicas.rows | map(select(.change == $change)) | sort_by(.instances) | map(. as $r | $idle[$r.instances | tostring] as $i |
      "\($r.transactions - $i.transactions) transactions, \($r.statements - $i.statements) statements\(if $r.refresh_ms then ", grants read in \($r.refresh_ms | ms)" else "" end)") | join(" | ")) |"),
  "",
  "Connections held: \(.replicas.rows | map(select(.change == "none")) | sort_by(.instances) | map("\(.connections) with \(.instances) instance\(if .instances > 1 then "s" else "" end)") | join(", ")).",
  "",
  "Measured on an \($env.hardware_class) (\($env.memory_gb) GB), \($env.containers), \($env.build) build."
' "$RECORD" > "$OUT"
echo "access-path-bench: wrote $OUT from $RECORD"
