#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Compare a fresh storage-benchmark record against the committed one for the
# same schema generation, and say per operation whether it moved (#3370).
#
# The harness (`app/ferroehr/benches/storage.rs`) writes its record over
# `docs/benchmarks/storage/<generation>/record.json`, which is where the
# committed baseline lives, so the baseline is read from `HEAD` rather than
# from the working tree: after a run the file on disk IS the fresh record.
#
# EXPLORATION, never a conformance record. Conformance is the CNF 2.0 suite
# Veredictum runs, whose artifacts live under `docs/conformance/<sut>/`.
# Nothing here earns a performance class, and a verdict below is a signal to
# investigate on the machine it was measured on, not a published number.
#
# Two records only compare when they measured the same corpus: `bench_class`
# sizes the seed, so an `s` baseline against a `poc` run is a different
# question, not a regression. When the classes differ every row reads
# `not compared` and the run stays green, with the reason in the summary.
#
# Usage:
#   scripts/render/storage-bench-compare.sh [options]
#     --measured <file>    the fresh record (default: the newest
#                          docs/benchmarks/storage/*/record.json on disk)
#     --baseline <file>    the record to compare against (default: the
#                          measured record's generation, read from HEAD)
#     --tolerance <pct>    how far p50 or p99 may rise before the run fails
#                          (default 15)
#     --summary <file>     where the markdown table is written (default
#                          $GITHUB_STEP_SUMMARY, else stdout)
#     --runner <text>      the machine this run measured on, named in the
#                          summary so a red verdict is attributable
#   scripts/render/storage-bench-compare.sh --self-test
#
# Exit: 0 when nothing regressed beyond the tolerance (and 0 when there is no
# committed record to compare against), 1 when something did, 2 on a usage or
# input error.
set -euo pipefail

usage() {
  echo "usage: scripts/render/storage-bench-compare.sh [--measured <file>] [--baseline <file>] [--tolerance <pct>] [--summary <file>] [--runner <text>] | --self-test" >&2
  exit 2
}

MEASURED=""
BASELINE=""
TOLERANCE="15"
SUMMARY="${GITHUB_STEP_SUMMARY:-}"
RUNNER=""
SELF_TEST=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --measured) MEASURED="${2:-}"; shift 2 || usage ;;
    --baseline) BASELINE="${2:-}"; shift 2 || usage ;;
    --tolerance) TOLERANCE="${2:-}"; shift 2 || usage ;;
    --summary) SUMMARY="${2:-}"; shift 2 || usage ;;
    --runner) RUNNER="${2:-}"; shift 2 || usage ;;
    --self-test) SELF_TEST=1; shift ;;
    *) usage ;;
  esac
done

command -v jq >/dev/null || { echo "error: jq is required" >&2; exit 2; }

# The formatting and verdict program, shared by every invocation below.
#
# `t` renders nanoseconds the way the book's storage section does; `pc` renders
# a signed percentage; the verdict turns on the WORSE of the p50 and p99 rises,
# and the row names which percentile that was, so a red row points straight at
# the number it failed on.
# shellcheck disable=SC2016 # every $name below is a jq variable, never a shell expansion
readonly COMPARE_JQ='
def t: if . == null then "—"
       elif . >= 1000000000 then ((. / 100000000 | round) / 10 | tostring) + " s"
       elif . >= 1000000 then ((. / 100000 | round) / 10 | tostring) + " ms"
       else ((. / 100 | round) / 10 | tostring) + " µs" end;
def pc: if . == null then "—"
        else (if . >= 0 then "+" else "" end) + ((. * 10 | round) / 10 | tostring) + "%" end;
def rise($base; $meas): if ($base == null or $meas == null or $base == 0) then null
                        else (($meas - $base) / $base * 100) end;

$b[0] as $base | $m[0] as $meas |
([$base | .groups[]? | .operations[]?] | map({key: .name, value: .}) | from_entries) as $bmap |
[$meas | .groups[]? | .operations[]?] as $mops |
($mops | map(.name)) as $mnames |
(
  ($mops | map({name: .name, base: $bmap[.name], meas: .}))
  + ([$base | .groups[]? | .operations[]?]
     | map(select((.name as $n | $mnames | index($n)) == null))
     | map({name: .name, base: ., meas: null}))
)
| map(
    (.base.p50_ns) as $b50 | (.base.p99_ns) as $b99 |
    (.meas.p50_ns) as $m50 | (.meas.p99_ns) as $m99 |
    rise($b50; $m50) as $d50 | rise($b99; $m99) as $d99 |
    (if $d50 == null and $d99 == null then null
     elif $d50 == null then {value: $d99, at: "p99"}
     elif $d99 == null then {value: $d50, at: "p50"}
     elif $d99 > $d50 then {value: $d99, at: "p99"}
     else {value: $d50, at: "p50"} end) as $worst |
    (if .base == null then ["no-baseline", "no committed baseline"]
     elif .meas == null or ($m50 == null and $m99 == null) then ["not-measured", "not measured in this run"]
     elif $compare | not then ["not-compared", "not compared"]
     elif $worst == null then ["not-measured", "not measured in this run"]
     elif $worst.value < 0 then ["faster", "faster"]
     elif $worst.value <= $tolerance then ["within", "within tolerance"]
     else ["regressed", "**regressed**"] end) as $verdict |
    "\($verdict[0])\t| `\(.name)` | \($b50|t) | \($b99|t) | \($m50|t) | \($m99|t) | " +
    (if $worst == null then "— | " else "\($worst.value|pc) (\($worst.at)) | " end) +
    "\($verdict[1]) |"
  )
| .[]
'

# One line naming a record: what it measured, where, and when.
describe_record() {
  jq -r '
    def b: if . == null then "?" else ((. / 1073741824 * 10 | round) / 10 | tostring) + " GiB" end;
    "class `\(.bench_class)`, \(.seed.ehrs) EHRs and \(.seed.compositions) compositions, " +
    "commit `\(.measured_at_commit[0:12])`, \(.cpu_count) cores, \(.memory_bytes | b), " +
    "\(.postgres_version | split(" ") | .[0:2] | join(" ")), measured \(.measured_at[0:10])"
  ' "$1"
}

# The record the harness just wrote: the newest `measured_at` of the records on
# disk. One generation exists per schema, so this is normally the only file.
discover_measured() {
  local newest="" path
  shopt -s nullglob
  for path in docs/benchmarks/storage/*/record.json; do
    if [[ -z "$newest" ]] \
      || [[ "$(jq -r '.measured_at' "$path")" > "$(jq -r '.measured_at' "$newest")" ]]; then
      newest="$path"
    fi
  done
  shopt -u nullglob
  [[ -n "$newest" ]] || return 1
  printf '%s\n' "$newest"
}

# Write `$2…` to the summary file, or to stdout when none was named.
emit() {
  if [[ -n "$SUMMARY" ]]; then
    printf '%s\n' "$@" >> "$SUMMARY"
  else
    printf '%s\n' "$@"
  fi
}

compare() {
  local measured="$1" baseline="$2" tolerance="$3" runner="$4"
  local generation measured_class baseline_class compare_flag rows regressed=0

  generation="$(jq -r '.schema_generation' "$measured")"
  measured_class="$(jq -r '.bench_class' "$measured")"
  baseline_class="$(jq -r '.bench_class' "$baseline")"
  compare_flag=true
  [[ "$measured_class" == "$baseline_class" ]] || compare_flag=false

  # A run that measured nothing leaves the committed record where it was, and
  # comparing that file with itself reads as a perfectly unchanged system.
  if [[ "$(jq -r '.measured_at' "$measured")" == "$(jq -r '.measured_at' "$baseline")" ]]; then
    echo "error: $measured is the committed record itself — the bench wrote none" >&2
    return 2
  fi

  emit "## Storage benchmark, schema generation \`$generation\`" ""
  emit "Exploration, never a conformance record: this lane measures the storage" \
       "layer below the wire and earns no performance class. The records live in" \
       "\`docs/benchmarks/storage/<generation>/\`; their shape is documented in" \
       "\`docs/benchmarks/storage/README.md\`." ""
  emit "- Baseline (committed): $(describe_record "$baseline")"
  emit "- Measured (this run): $(describe_record "$measured")"
  [[ -z "$runner" ]] || emit "- Runner: $runner"
  emit "- Tolerance: a p50 or p99 more than ${tolerance}% above the baseline fails the run." ""

  if [[ "$compare_flag" == false ]]; then
    emit "The two records measured different corpora (\`$baseline_class\` against" \
         "\`$measured_class\`), so the deltas below are reported and nothing is" \
         "judged. Dispatch this lane with the baseline's class to compare." ""
  fi

  rows="$(jq -r -n \
    --slurpfile b "$baseline" \
    --slurpfile m "$measured" \
    --argjson tolerance "$tolerance" \
    --argjson compare "$compare_flag" \
    "$COMPARE_JQ")"

  emit "| Operation | Baseline p50 | Baseline p99 | Measured p50 | Measured p99 | Delta | Verdict |" \
       "|---|---|---|---|---|---|---|"

  # The log lines are collected rather than printed inside the loop: without a
  # summary file both streams are stdout, and an interleaved table is unreadable.
  local verdict row
  local -a red=()
  while IFS=$'\t' read -r verdict row; do
    [[ -n "$row" ]] || continue
    emit "$row"
    if [[ "$verdict" == "regressed" ]]; then
      regressed=$((regressed + 1))
      red+=("$row")
    fi
  done <<< "$rows"
  emit ""
  for row in "${red[@]+"${red[@]}"}"; do
    echo "regressed: $row"
  done

  if [[ "$regressed" -gt 0 ]]; then
    emit "$regressed operation(s) rose more than ${tolerance}% above the committed record." ""
    echo "storage-bench-compare: $regressed operation(s) beyond the ${tolerance}% tolerance." >&2
    return 1
  fi
  emit "No operation rose more than ${tolerance}% above the committed record." ""
  echo "storage-bench-compare: nothing beyond the ${tolerance}% tolerance."
  return 0
}

# ── the self-test ───────────────────────────────────────────────────────────
# The verdict logic is what this script exists for, so it is exercised rather
# than trusted: an improvement, a rise inside the tolerance, a rise beyond it,
# a mismatched class, an operation each side carries alone, and a record
# compared with itself.

record_fixture() {
  local path="$1" generation="$2" class="$3" ehrs="$4" p50="$5" p99="$6" when="${7:-2026-09-15T11:00:00Z}"
  mkdir -p "$(dirname "$path")"
  cat > "$path" <<EOF
{
  "schema_generation": "$generation",
  "postgres_version": "PostgreSQL 18.6 (self-test)",
  "bench_class": "$class",
  "cpu_count": 8,
  "memory_bytes": 17179869184,
  "measured_at_commit": "0123456789abcdef0123456789abcdef01234567",
  "measured_at": "$when",
  "timings_measured": true,
  "seed": { "ehrs": $ehrs, "compositions": 300, "template_id": "Vital signs",
            "entry_rm_type": "OBSERVATION", "seconds": 1.0 },
  "groups": [
    { "name": "storage_commit",
      "operations": [
        { "name": "create_first_version", "iterations": 100, "samples": 10,
          "mean_ns": $p50, "median_ns": $p50, "p50_ns": $p50, "p95_ns": $p99, "p99_ns": $p99 },
        { "name": "supersede", "iterations": 100, "samples": 10,
          "mean_ns": 2000000, "median_ns": 2000000,
          "p50_ns": 2000000, "p95_ns": 3000000, "p99_ns": 3000000 }
      ] } ]
}
EOF
}

self_test() {
  local tmp failures=0 out status
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN

  # The baseline every case compares against.
  record_fixture "$tmp/baseline.json" generation-1 poc 20 1000000 2000000 2026-09-15T10:00:00Z

  # 1. An improvement: both percentiles fall.
  record_fixture "$tmp/faster.json" generation-1 poc 20 800000 1600000
  status=0
  compare "$tmp/faster.json" "$tmp/baseline.json" 15 "self-test" \
    > "$tmp/faster.out" 2>&1 || status=$?
  out="$(cat "$tmp/faster.out")"
  [[ "$status" -eq 0 ]] || { echo "self-test: an improvement failed the run" >&2; failures=1; }
  grep -q 'create_first_version.*faster' <<<"$out" \
    || { echo "self-test: an improvement did not read as faster" >&2; failures=1; }

  # 2. A rise inside the tolerance: reported, green.
  record_fixture "$tmp/mild.json" generation-1 poc 20 1100000 2200000
  status=0
  compare "$tmp/mild.json" "$tmp/baseline.json" 15 "self-test" \
    > "$tmp/mild.out" 2>&1 || status=$?
  out="$(cat "$tmp/mild.out")"
  [[ "$status" -eq 0 ]] || { echo "self-test: a rise inside the tolerance failed the run" >&2; failures=1; }
  grep -q 'create_first_version.*+10%.*within tolerance' <<<"$out" \
    || { echo "self-test: a rise inside the tolerance did not read as within tolerance" >&2; failures=1; }

  # 3. A rise beyond the tolerance: red, and the failing percentile is named.
  record_fixture "$tmp/hot.json" generation-1 poc 20 1050000 3000000
  status=0
  compare "$tmp/hot.json" "$tmp/baseline.json" 15 "self-test" \
    > "$tmp/hot.out" 2>&1 || status=$?
  out="$(cat "$tmp/hot.out")"
  [[ "$status" -eq 1 ]] || { echo "self-test: a rise beyond the tolerance did not fail the run" >&2; failures=1; }
  grep -q 'create_first_version.*+50%.*(p99).*regressed' <<<"$out" \
    || { echo "self-test: a rise beyond the tolerance did not read as a regression" >&2; failures=1; }
  grep -q 'supersede.*+0%.*within tolerance' <<<"$out" \
    || { echo "self-test: an unchanged operation beside a regression was misjudged" >&2; failures=1; }

  # 4. A different corpus: reported, judged by nothing.
  record_fixture "$tmp/other-class.json" generation-1 s 100 3000000 6000000
  status=0
  compare "$tmp/other-class.json" "$tmp/baseline.json" 15 "self-test" \
    > "$tmp/other-class.out" 2>&1 || status=$?
  out="$(cat "$tmp/other-class.out")"
  [[ "$status" -eq 0 ]] || { echo "self-test: a class mismatch failed the run" >&2; failures=1; }
  grep -q 'create_first_version.*not compared' <<<"$out" \
    || { echo "self-test: a class mismatch was judged anyway" >&2; failures=1; }

  # 5. An operation the baseline never carried: reported, judged by nothing.
  record_fixture "$tmp/narrow.json" generation-1 poc 20 1000000 2000000
  jq '.groups[0].operations[1].name = "prune_retained"' "$tmp/narrow.json" > "$tmp/new-op.json"
  status=0
  compare "$tmp/new-op.json" "$tmp/baseline.json" 15 "self-test" \
    > "$tmp/new-op.out" 2>&1 || status=$?
  out="$(cat "$tmp/new-op.out")"
  [[ "$status" -eq 0 ]] || { echo "self-test: a new operation failed the run" >&2; failures=1; }
  grep -q 'prune_retained.*no committed baseline' <<<"$out" \
    || { echo "self-test: a new operation was not reported as unbaselined" >&2; failures=1; }
  grep -q 'supersede.*not measured in this run' <<<"$out" \
    || { echo "self-test: a baseline operation missing from the run was not reported" >&2; failures=1; }

  # 6. The bench wrote nothing and the committed record is still on disk: a
  # comparison of one file with itself is refused rather than reported green.
  status=0
  compare "$tmp/baseline.json" "$tmp/baseline.json" 15 "self-test" \
    > "$tmp/unwritten.out" 2>&1 || status=$?
  [[ "$status" -eq 2 ]] \
    || { echo "self-test: a record compared with itself was not refused" >&2; failures=1; }

  if [[ "$failures" -ne 0 ]]; then
    echo "storage-bench-compare: SELF-TEST FAILED" >&2
    return 1
  fi
  echo "storage-bench-compare: self-test passed (6 cases)."
  return 0
}

if [[ "$SELF_TEST" -eq 1 ]]; then
  SUMMARY=""
  self_test
  exit $?
fi

# Paths named on the command line are the CALLER's, and the working directory
# moves to the repository root next.
absolute() { case "$1" in /*) printf '%s\n' "$1" ;; *) printf '%s\n' "$PWD/$1" ;; esac; }
[[ -z "$MEASURED" ]] || MEASURED="$(absolute "$MEASURED")"
[[ -z "$BASELINE" ]] || BASELINE="$(absolute "$BASELINE")"
[[ -z "$SUMMARY" ]] || SUMMARY="$(absolute "$SUMMARY")"

cd "$(dirname "$0")/../.."

[[ "$TOLERANCE" =~ ^[0-9]+(\.[0-9]+)?$ ]] \
  || { echo "error: --tolerance takes a non-negative number, not '$TOLERANCE'" >&2; exit 2; }

if [[ -z "$MEASURED" ]]; then
  MEASURED="$(discover_measured)" \
    || { echo "error: no docs/benchmarks/storage/*/record.json on disk — run the bench first" >&2; exit 2; }
fi
[[ -f "$MEASURED" ]] || { echo "error: no such record: $MEASURED" >&2; exit 2; }

generation="$(jq -r '.schema_generation' "$MEASURED")"

# The baseline is the COMMITTED record: the run overwrote the working-tree copy.
baseline_tmp=""
if [[ -z "$BASELINE" ]]; then
  committed="docs/benchmarks/storage/$generation/record.json"
  if git cat-file -e "HEAD:$committed" 2>/dev/null; then
    baseline_tmp="$(mktemp)"
    trap 'rm -f "$baseline_tmp"' EXIT
    git show "HEAD:$committed" > "$baseline_tmp"
    BASELINE="$baseline_tmp"
  else
    emit "## Storage benchmark, schema generation \`$generation\`" ""
    emit "No committed record: \`$committed\` is not in this repository, so this" \
         "run has nothing to compare against. The fresh record is attached to the" \
         "run as an artifact; committing a baseline is described in" \
         "\`docs/benchmarks/storage/README.md\`." ""
    echo "storage-bench-compare: no committed record for $generation."
    exit 0
  fi
fi
[[ -f "$BASELINE" ]] || { echo "error: no such record: $BASELINE" >&2; exit 2; }

compare "$MEASURED" "$BASELINE" "$TOLERANCE" "$RUNNER"
