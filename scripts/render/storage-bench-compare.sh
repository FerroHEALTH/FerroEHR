#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Compare a fresh storage-benchmark record against the committed one for the
# same schema generation, and say per operation whether it moved (#3370).
#
# The latency verdict of the lane is the A/B mode (`--ab`, #4318): a baseline
# build and a candidate build measured in turn on ONE runner, several rounds
# each, judged against each other. A latency read on one GitHub-hosted runner
# and compared with a record from another said little (two runs of the same
# code differed by up to 156 %). An operation regresses only when its median
# across the rounds rose beyond the tolerance AND every candidate round is
# slower than every baseline round at that percentile; a rise inside the
# spread of the rounds is reported as within the noise, never as a pass of the
# tolerance. The storage sizes do not depend on the machine and are still
# judged against the committed record.
#
# The harness (`app/ferroehr/benches/storage.rs`) writes its record over
# `docs/benchmarks/storage/<generation>/<class>/record.json`, which is where the
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
#                          docs/benchmarks/storage/*/*/record.json on disk)
#     --baseline <file>    the record to compare against (default: the
#                          measured record's generation and class, read
#                          from HEAD)
#     --tolerance <pct>    how far p50 or p99 may rise before the run fails
#                          (default 15)
#     --size-budget <pct>  how far the bytes per composition version or the
#                          heap bytes per node row may grow before the run
#                          fails (default 5)
#     --summary <file>     where the markdown table is written (default
#                          $GITHUB_STEP_SUMMARY, else stdout)
#     --runner <text>      the machine this run measured on, named in the
#                          summary so a red verdict is attributable
#     --ab <dir>           the A/B mode: judge latency between the round
#                          records `<dir>/baseline-*.json` and
#                          `<dir>/candidate-*.json`, measured on one machine,
#                          and the candidate's sizes against the committed
#                          record of its generation and class
#   scripts/render/storage-bench-compare.sh --self-test
#
# A generation with no committed record fails the run (#4291): a schema
# generation change once left the lane comparing nothing and passing, so the
# change that moves the generation commits the new baseline with it.
#
# Exit: 0 when nothing regressed beyond the tolerance, 1 when something did or
# when the measured generation has no committed record, 2 on a usage or input
# error.
set -euo pipefail

usage() {
  echo "usage: scripts/render/storage-bench-compare.sh [--measured <file>] [--baseline <file>] [--tolerance <pct>] [--size-budget <pct>] [--summary <file>] [--runner <text>] [--ab <dir>] | --self-test" >&2
  exit 2
}

MEASURED=""
BASELINE=""
TOLERANCE="15"
SIZE_BUDGET="5"
SUMMARY="${GITHUB_STEP_SUMMARY:-}"
RUNNER=""
AB_DIR=""
SELF_TEST=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --measured) MEASURED="${2:-}"; shift 2 || usage ;;
    --baseline) BASELINE="${2:-}"; shift 2 || usage ;;
    --tolerance) TOLERANCE="${2:-}"; shift 2 || usage ;;
    --size-budget) SIZE_BUDGET="${2:-}"; shift 2 || usage ;;
    --summary) SUMMARY="${2:-}"; shift 2 || usage ;;
    --runner) RUNNER="${2:-}"; shift 2 || usage ;;
    --ab) AB_DIR="${2:-}"; shift 2 || usage ;;
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

# The A/B latency program (#4318): `$b` and `$m` are the baseline's and the
# candidate's round records. Each side's p50 and p99 are the medians across its
# rounds, shown with the range of the rounds. A rise beyond the tolerance is a
# regression only when the ranges at the worse percentile do not overlap, so a
# difference the rounds cannot separate is reported as noise.
# shellcheck disable=SC2016 # every $name below is a jq variable, never a shell expansion
readonly COMPARE_AB_JQ='
def t: if . == null then "—"
       elif . >= 1000000000 then ((. / 100000000 | round) / 10 | tostring) + " s"
       elif . >= 1000000 then ((. / 100000 | round) / 10 | tostring) + " ms"
       else ((. / 100 | round) / 10 | tostring) + " µs" end;
def pc: if . == null then "—"
        else (if . >= 0 then "+" else "" end) + ((. * 10 | round) / 10 | tostring) + "%" end;
def rise($base; $meas): if ($base == null or $meas == null or $base == 0) then null
                        else (($meas - $base) / $base * 100) end;
def med: sort | if length == 0 then null
                elif length % 2 == 1 then .[(length - 1) / 2]
                else (.[length / 2 - 1] + .[length / 2]) / 2 end;
def series($records; $name; $field):
  [$records[] | [.groups[]? | .operations[]? | select(.name == $name) | .[$field]] | .[0]
   | select(. != null)];
def shown($xs): if ($xs | length) == 0 then "—"
                elif ($xs | length) == 1 then ($xs[0] | t)
                else "\($xs | med | t) (\($xs | min | t)–\($xs | max | t))" end;

([$m[] | .groups[]? | .operations[]? | .name] + [$b[] | .groups[]? | .operations[]? | .name]
 | reduce .[] as $n ([]; if index($n) then . else . + [$n] end)) as $names
| $names[]
| . as $n
| series($b; $n; "p50_ns") as $b50s | series($b; $n; "p99_ns") as $b99s
| series($m; $n; "p50_ns") as $m50s | series($m; $n; "p99_ns") as $m99s
| ($b50s | med) as $b50 | ($b99s | med) as $b99
| ($m50s | med) as $m50 | ($m99s | med) as $m99
| rise($b50; $m50) as $d50 | rise($b99; $m99) as $d99
| (if $d50 == null and $d99 == null then null
   elif $d50 == null then {value: $d99, at: "p99"}
   elif $d99 == null then {value: $d50, at: "p50"}
   elif $d99 > $d50 then {value: $d99, at: "p99"}
   else {value: $d50, at: "p50"} end) as $worst
| (if $worst == null then false
   elif $worst.at == "p99" then ($m99s | min) <= ($b99s | max)
   else ($m50s | min) <= ($b50s | max) end) as $overlap
| (if ($b50s | length) == 0 and ($b99s | length) == 0 then ["no-baseline", "not measured on the baseline"]
   elif ($m50s | length) == 0 and ($m99s | length) == 0 then ["not-measured", "not measured on the candidate"]
   elif $compare | not then ["not-compared", "not compared"]
   elif $worst == null then ["not-measured", "not measured on the candidate"]
   elif $worst.value < 0 then ["faster", "faster"]
   elif $worst.value <= $tolerance then ["within", "within tolerance"]
   elif $overlap then ["noise", "within the noise of the rounds"]
   else ["regressed", "**regressed**"] end) as $verdict
| "\($verdict[0])\t| `\($n)` | \(shown($b50s)) | \(shown($b99s)) | \(shown($m50s)) | \(shown($m99s)) | " +
  (if $worst == null then "— | " else "\($worst.value|pc) (\($worst.at)) | " end) +
  "\($verdict[1]) |"
'

# The storage-size verdicts (#4303): bytes per composition version and heap
# bytes per node row are judged against the budget; the other facts are
# reported beside them. Sizes do not depend on the machine, so the budget is
# tight, and a record without a `storage` section is reported, never judged.
# shellcheck disable=SC2016 # every $name below is a jq variable, never a shell expansion
readonly SIZE_JQ='
def b: if . == null then "—" else ((. * 10 | round) / 10 | tostring) + " B" end;
def n: if . == null then "—" else ((. * 100 | round) / 100 | tostring) end;
def pc: if . == null then "—"
        else (if . >= 0 then "+" else "" end) + ((. * 10 | round) / 10 | tostring) + "%" end;
def rise($base; $meas): if ($base == null or $meas == null or $base == 0) then null
                        else (($meas - $base) / $base * 100) end;
$b[0].storage as $base | $m[0].storage as $meas |
[ ["bytes_per_composition_version", "bytes per composition version", true, "b"],
  ["node_heap_bytes_per_row", "heap bytes per node row", true, "b"],
  ["node_rows_per_version", "node rows per version", false, "n"],
  ["node_row_bytes", "node row width", false, "b"],
  ["input_bytes_per_version", "canonical JSON per version", false, "b"],
  ["audit_bytes_per_record", "audit bytes per record", false, "b"] ]
| .[]
| . as [$key, $label, $judged, $fmt]
| ($base[$key]?) as $bv | ($meas[$key]?) as $mv
| rise($bv; $mv) as $d
| (if $fmt == "b" then [($bv|b), ($mv|b)] else [($bv|n), ($mv|n)] end) as $shown
| (if ($judged | not) then ["reported", "reported"]
   elif $mv == null then ["not-measured", "not measured in this run"]
   elif $bv == null then ["no-baseline", "no committed baseline"]
   elif $compare | not then ["not-compared", "not compared"]
   elif $d <= $budget then ["within", (if $d < 0 then "smaller" else "within budget" end)]
   else ["grew", "**grew**"] end) as $verdict
| "\($verdict[0])\t| \($label) | \($shown[0]) | \($shown[1]) | \($d|pc) | \($verdict[1]) |"
'

# One line naming a record: what it measured, where, and when.
# measured_at <record.json>: when the record was measured.
measured_at() {
  local record="$1"
  jq -r '.measured_at' "$record"
}

describe_record() {
  local record="$1"
  jq -r '
    def b: if . == null then "?" else ((. / 1073741824 * 10 | round) / 10 | tostring) + " GiB" end;
    "class `\(.bench_class)`, \(.seed.ehrs) EHRs and \(.seed.compositions) compositions, " +
    "commit `\(.measured_at_commit[0:12])`, \(.cpu_count) cores, \(.memory_bytes | b), " +
    "\(.postgres_version | split(" ") | .[0:2] | join(" ")), measured \(.measured_at[0:10])"
  ' "$record"
}

# The record the harness just wrote: the newest `measured_at` of the records on
# disk. One generation exists per schema, so this is normally the only file.
discover_measured() {
  local newest="" path
  shopt -s nullglob
  for path in docs/benchmarks/storage/*/*/record.json; do
    if [[ -z "$newest" ]] \
      || [[ "$(measured_at "$path")" > "$(measured_at "$newest")" ]]; then
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

# The verdict for a generation that has no committed record: written to the
# summary as an error, and a failed run.
missing_baseline() {
  local generation="$1" committed="$2"
  emit "## Storage benchmark, schema generation \`$generation\`" ""
  emit "**Error: no committed record.** \`$committed\` is not in this" \
       "repository, so this run has nothing to compare against and fails. The" \
       "change that moved the schema generation commits its baseline with it;" \
       "the procedure is in \`docs/benchmarks/storage/README.md\`. The fresh" \
       "record is attached to the run as an artifact." ""
  [[ -z "${GITHUB_ACTIONS:-}" ]] || echo "::error::no committed storage-bench record at $committed"
  echo "storage-bench-compare: no committed record for $generation." >&2
  return 1
}

compare() {
  local measured="$1" baseline="$2" tolerance="$3" runner="$4" budget="${5:-5}"
  local generation measured_class baseline_class compare_flag rows regressed=0 grew=0

  generation="$(jq -r '.schema_generation' "$measured")"
  measured_class="$(jq -r '.bench_class' "$measured")"
  baseline_class="$(jq -r '.bench_class' "$baseline")"
  compare_flag=true
  [[ "$measured_class" == "$baseline_class" ]] || compare_flag=false

  # A run that measured nothing leaves the committed record where it was, and
  # comparing that file with itself reads as a perfectly unchanged system.
  if [[ "$(measured_at "$measured")" == "$(measured_at "$baseline")" ]]; then
    echo "error: $measured is the committed record itself — the bench wrote none" >&2
    return 2
  fi

  emit "## Storage benchmark, schema generation \`$generation\`" ""
  emit "Exploration, never a conformance record: this lane measures the storage" \
       "layer below the wire and earns no performance class. The records live in" \
       "\`docs/benchmarks/storage/<generation>/<class>/\`; their shape is documented in" \
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

  emit "Storage per composition version, a growth beyond ${budget}% fails the run:" "" \
       "| Fact | Baseline | Measured | Delta | Verdict |" \
       "|---|---|---|---|---|"
  while IFS=$'\t' read -r verdict row; do
    [[ -n "$row" ]] || continue
    emit "$row"
    if [[ "$verdict" == "grew" ]]; then
      grew=$((grew + 1))
      echo "grew: $row"
    fi
  done < <(jq -r -n \
    --slurpfile b "$baseline" \
    --slurpfile m "$measured" \
    --argjson budget "$budget" \
    --argjson compare "$compare_flag" \
    "$SIZE_JQ")
  emit ""

  if [[ "$grew" -gt 0 ]]; then
    emit "$grew storage fact(s) grew more than ${budget}% above the committed record." ""
    echo "storage-bench-compare: $grew storage fact(s) beyond the ${budget}% budget." >&2
    return 1
  fi

  if [[ "$regressed" -gt 0 ]]; then
    emit "$regressed operation(s) rose more than ${tolerance}% above the committed record." ""
    echo "storage-bench-compare: $regressed operation(s) beyond the ${tolerance}% tolerance." >&2
    return 1
  fi
  emit "No operation rose more than ${tolerance}% above the committed record." ""
  echo "storage-bench-compare: nothing beyond the ${tolerance}% tolerance."
  return 0
}

# The A/B verdict (#4318): latency between the round records in `$1`, sizes of
# the first candidate round against the committed record `$2` (empty when the
# candidate's generation has none, which fails the run as before).
compare_ab() {
  local dir="$1" committed="$2" tolerance="$3" runner="$4" budget="${5:-5}"
  local -a base=() cand=()
  local classes compare_flag rows verdict row regressed=0 noise=0 grew=0 generation
  shopt -s nullglob
  base=("$dir"/baseline-*.json)
  cand=("$dir"/candidate-*.json)
  shopt -u nullglob
  if [[ "${#base[@]}" -eq 0 || "${#cand[@]}" -eq 0 ]]; then
    echo "error: $dir holds ${#base[@]} baseline and ${#cand[@]} candidate round records; the A/B mode needs at least one of each" >&2
    return 2
  fi
  classes="$(jq -rs '[.[].bench_class] | unique | join(" ")' "${base[@]}" "${cand[@]}")"
  compare_flag=true
  [[ "$classes" != *" "* ]] || compare_flag=false
  generation="$(jq -r '.schema_generation' "${cand[0]}")"

  emit "## Storage benchmark, baseline against candidate on one runner" ""
  emit "Exploration, never a conformance record: this lane measures the storage" \
       "layer below the wire and earns no performance class. The procedure is in" \
       "\`docs/benchmarks/storage/README.md\`." ""
  emit "- Baseline (${#base[@]} rounds): $(describe_record "${base[0]}")"
  emit "- Candidate (${#cand[@]} rounds): $(describe_record "${cand[0]}")"
  [[ -z "$runner" ]] || emit "- Runner: $runner"
  emit "- Tolerance: an operation regresses when the median of its rounds' p50 or" \
       "  p99 rose more than ${tolerance}% above the baseline's and every candidate" \
       "  round is slower than every baseline round at that percentile. A rise" \
       "  beyond the tolerance inside the rounds' spread reads as noise: dispatch" \
       "  more rounds to separate it." ""
  if [[ "$compare_flag" == false ]]; then
    emit "The rounds measured different corpora (\`$classes\`), so the deltas below" \
         "are reported and nothing is judged." ""
  fi

  rows="$(jq -r -n \
    --slurpfile b <(jq -s '.' "${base[@]}") \
    --slurpfile m <(jq -s '.' "${cand[@]}") \
    --argjson tolerance "$tolerance" \
    --argjson compare "$compare_flag" \
    '$b[0] as $b | $m[0] as $m | '"$COMPARE_AB_JQ")"

  emit "| Operation | Baseline p50 | Baseline p99 | Candidate p50 | Candidate p99 | Delta | Verdict |" \
       "|---|---|---|---|---|---|---|"
  local -a red=()
  while IFS=$'\t' read -r verdict row; do
    [[ -n "$row" ]] || continue
    emit "$row"
    case "$verdict" in
      regressed) regressed=$((regressed + 1)); red+=("$row") ;;
      noise) noise=$((noise + 1)) ;;
    esac
  done <<< "$rows"
  emit ""
  for row in "${red[@]+"${red[@]}"}"; do
    echo "regressed: $row"
  done

  if [[ -z "$committed" ]]; then
    missing_baseline "$generation" "docs/benchmarks/storage/$generation/$(jq -r '.bench_class' "${cand[0]}")/record.json" || return $?
  fi
  emit "Storage per composition version, the candidate against the committed record" \
       "(\`$(describe_record "$committed")\`), a growth beyond ${budget}% fails the run:" "" \
       "| Fact | Committed | Candidate | Delta | Verdict |" \
       "|---|---|---|---|---|"
  local size_compare=true
  [[ "$(jq -r '.bench_class' "$committed")" == "$(jq -r '.bench_class' "${cand[0]}")" ]] || size_compare=false
  while IFS=$'\t' read -r verdict row; do
    [[ -n "$row" ]] || continue
    emit "$row"
    if [[ "$verdict" == "grew" ]]; then
      grew=$((grew + 1))
      echo "grew: $row"
    fi
  done < <(jq -r -n \
    --slurpfile b "$committed" \
    --slurpfile m "${cand[0]}" \
    --argjson budget "$budget" \
    --argjson compare "$size_compare" \
    "$SIZE_JQ")
  emit ""

  [[ "$noise" -eq 0 ]] || emit "$noise operation(s) rose more than ${tolerance}% inside the spread of the rounds." ""
  if [[ "$grew" -gt 0 ]]; then
    emit "$grew storage fact(s) grew more than ${budget}% above the committed record." ""
    echo "storage-bench-compare: $grew storage fact(s) beyond the ${budget}% budget." >&2
    return 1
  fi
  if [[ "$regressed" -gt 0 ]]; then
    emit "$regressed operation(s) rose more than ${tolerance}% above the baseline, outside the spread of the rounds." ""
    echo "storage-bench-compare: $regressed operation(s) beyond the ${tolerance}% tolerance." >&2
    return 1
  fi
  emit "No operation rose more than ${tolerance}% above the baseline outside the spread of the rounds." ""
  echo "storage-bench-compare: nothing beyond the ${tolerance}% tolerance."
  return 0
}

# ── the self-test ───────────────────────────────────────────────────────────
# The verdict logic is what this script exists for, so it is exercised rather
# than trusted: an improvement, a rise inside the tolerance, a rise beyond it,
# a mismatched class, an operation each side carries alone, a record compared
# with itself, and a generation with no committed record.

record_fixture() {
  local path="$1" generation="$2" class="$3" ehrs="$4" p50="$5" p99="$6" when="${7:-2026-09-15T11:00:00Z}"
  # $8 and $9 are the bytes per composition version and per node row.
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
      ] } ],
  "storage": { "composition_versions": 340, "bytes_per_composition_version": ${8:-20000},
               "node_rows_per_version": 34.2, "node_heap_bytes_per_row": ${9:-461},
               "node_row_bytes": 445, "input_bytes_per_version": 8400,
               "audit_bytes_per_record": null }
}
EOF
}

self_test() {
  local tmp failures=0 out status runner="self-test"
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN

  # The baseline every case compares against.
  record_fixture "$tmp/baseline.json" generation-1 poc 20 1000000 2000000 2026-09-15T10:00:00Z

  # 1. An improvement: both percentiles fall.
  record_fixture "$tmp/faster.json" generation-1 poc 20 800000 1600000
  status=0
  compare "$tmp/faster.json" "$tmp/baseline.json" 15 "$runner" \
    > "$tmp/faster.out" 2>&1 || status=$?
  out="$(cat "$tmp/faster.out")"
  [[ "$status" -eq 0 ]] || { echo "self-test: an improvement failed the run" >&2; failures=1; }
  grep -q 'create_first_version.*faster' <<<"$out" \
    || { echo "self-test: an improvement did not read as faster" >&2; failures=1; }

  # 2. A rise inside the tolerance: reported, green.
  record_fixture "$tmp/mild.json" generation-1 poc 20 1100000 2200000
  status=0
  compare "$tmp/mild.json" "$tmp/baseline.json" 15 "$runner" \
    > "$tmp/mild.out" 2>&1 || status=$?
  out="$(cat "$tmp/mild.out")"
  [[ "$status" -eq 0 ]] || { echo "self-test: a rise inside the tolerance failed the run" >&2; failures=1; }
  grep -q 'create_first_version.*+10%.*within tolerance' <<<"$out" \
    || { echo "self-test: a rise inside the tolerance did not read as within tolerance" >&2; failures=1; }

  # 3. A rise beyond the tolerance: red, and the failing percentile is named.
  record_fixture "$tmp/hot.json" generation-1 poc 20 1050000 3000000
  status=0
  compare "$tmp/hot.json" "$tmp/baseline.json" 15 "$runner" \
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
  compare "$tmp/other-class.json" "$tmp/baseline.json" 15 "$runner" \
    > "$tmp/other-class.out" 2>&1 || status=$?
  out="$(cat "$tmp/other-class.out")"
  [[ "$status" -eq 0 ]] || { echo "self-test: a class mismatch failed the run" >&2; failures=1; }
  grep -q 'create_first_version.*not compared' <<<"$out" \
    || { echo "self-test: a class mismatch was judged anyway" >&2; failures=1; }

  # 5. An operation the baseline never carried: reported, judged by nothing.
  record_fixture "$tmp/narrow.json" generation-1 poc 20 1000000 2000000
  jq '.groups[0].operations[1].name = "prune_retained"' "$tmp/narrow.json" > "$tmp/new-op.json"
  status=0
  compare "$tmp/new-op.json" "$tmp/baseline.json" 15 "$runner" \
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
  compare "$tmp/baseline.json" "$tmp/baseline.json" 15 "$runner" \
    > "$tmp/unwritten.out" 2>&1 || status=$?
  [[ "$status" -eq 2 ]] \
    || { echo "self-test: a record compared with itself was not refused" >&2; failures=1; }

  # 8. Bytes per composition version growing beyond the budget fails the run
  # and names the fact; a smaller record passes.
  record_fixture "$tmp/fat.json" generation-1 poc 20 1000000 2000000 2026-09-15T11:00:00Z 22000 461
  status=0
  compare "$tmp/fat.json" "$tmp/baseline.json" 15 "$runner" 5 \
    > "$tmp/fat.out" 2>&1 || status=$?
  out="$(cat "$tmp/fat.out")"
  [[ "$status" -eq 1 ]] || { echo "self-test: storage growth beyond the budget did not fail the run" >&2; failures=1; }
  grep -q 'bytes per composition version.*+10%.*grew' <<<"$out" \
    || { echo "self-test: storage growth was not reported against its fact" >&2; failures=1; }
  record_fixture "$tmp/slim.json" generation-1 poc 20 1000000 2000000 2026-09-15T11:00:00Z 15000 300
  status=0
  compare "$tmp/slim.json" "$tmp/baseline.json" 15 "$runner" 5 \
    > "$tmp/slim.out" 2>&1 || status=$?
  out="$(cat "$tmp/slim.out")"
  [[ "$status" -eq 0 ]] || { echo "self-test: smaller storage failed the run" >&2; failures=1; }
  grep -q 'heap bytes per node row.*smaller' <<<"$out" \
    || { echo "self-test: smaller storage did not read as smaller" >&2; failures=1; }

  # 7. A generation with no committed record fails the run, and says so in
  # the summary as an error.
  status=0
  SUMMARY="$tmp/missing.md" missing_baseline generation-9 docs/benchmarks/storage/generation-9/poc/record.json \
    > "$tmp/missing.out" 2>&1 || status=$?
  [[ "$status" -eq 1 ]] \
    || { echo "self-test: a generation without a committed record did not fail the run" >&2; failures=1; }
  grep -q 'Error: no committed record' "$tmp/missing.md" \
    || { echo "self-test: a missing record was not reported as an error" >&2; failures=1; }

  # 9. The A/B mode. Two rounds a side; the committed record holds the sizes.
  local ab="$tmp/ab"
  ab_case() {
    rm -rf "$ab" && mkdir -p "$ab"
    record_fixture "$ab/baseline-1.json" generation-1 poc 20 "$1" "$2" 2026-09-15T12:00:00Z
    record_fixture "$ab/baseline-2.json" generation-1 poc 20 "$3" "$4" 2026-09-15T12:20:00Z
    record_fixture "$ab/candidate-1.json" generation-1 poc 20 "$5" "$6" 2026-09-15T12:10:00Z
    record_fixture "$ab/candidate-2.json" generation-1 poc 20 "$7" "$8" 2026-09-15T12:30:00Z
  }
  # 9a. The same code twice, rounds a few percent apart: within tolerance.
  ab_case 1000000 2000000 1040000 2060000 1020000 2040000 980000 1990000
  status=0
  compare_ab "$ab" "$tmp/baseline.json" 15 "$runner" 5 > "$tmp/ab-same.out" 2>&1 || status=$?
  out="$(cat "$tmp/ab-same.out")"
  [[ "$status" -eq 0 ]] || { echo "self-test: an A/A run failed" >&2; failures=1; }
  grep -qE 'create_first_version.*(within tolerance|faster)' <<<"$out" \
    || { echo "self-test: an A/A run did not read as within tolerance" >&2; failures=1; }
  # 9b. Every candidate round 30 % slower than every baseline round: red.
  ab_case 1000000 2000000 1050000 2100000 1350000 2700000 1400000 2750000
  status=0
  compare_ab "$ab" "$tmp/baseline.json" 15 "$runner" 5 > "$tmp/ab-slow.out" 2>&1 || status=$?
  out="$(cat "$tmp/ab-slow.out")"
  [[ "$status" -eq 1 ]] || { echo "self-test: a separated A/B regression did not fail the run" >&2; failures=1; }
  grep -q 'create_first_version.*regressed' <<<"$out" \
    || { echo "self-test: a separated A/B regression did not read as regressed" >&2; failures=1; }
  grep -q 'supersede.*within tolerance' <<<"$out" \
    || { echo "self-test: an unchanged operation beside an A/B regression was misjudged" >&2; failures=1; }
  # 9c. A median 30 % up whose rounds overlap the baseline's: noise, named.
  ab_case 1000000 2000000 1200000 2400000 1150000 2300000 1700000 3400000
  status=0
  compare_ab "$ab" "$tmp/baseline.json" 15 "$runner" 5 > "$tmp/ab-noise.out" 2>&1 || status=$?
  out="$(cat "$tmp/ab-noise.out")"
  [[ "$status" -eq 0 ]] || { echo "self-test: an A/B rise inside the rounds' spread failed the run" >&2; failures=1; }
  grep -q 'create_first_version.*within the noise of the rounds' <<<"$out" \
    || { echo "self-test: an A/B rise inside the rounds' spread was not named noise" >&2; failures=1; }
  # 9d. The candidate's sizes grow beyond the budget against the committed record.
  ab_case 1000000 2000000 1000000 2000000 1000000 2000000 1000000 2000000
  record_fixture "$ab/candidate-1.json" generation-1 poc 20 1000000 2000000 2026-09-15T12:10:00Z 23000 461
  status=0
  compare_ab "$ab" "$tmp/baseline.json" 15 "$runner" 5 > "$tmp/ab-fat.out" 2>&1 || status=$?
  out="$(cat "$tmp/ab-fat.out")"
  [[ "$status" -eq 1 ]] || { echo "self-test: A/B storage growth did not fail the run" >&2; failures=1; }
  grep -q 'bytes per composition version.*grew' <<<"$out" \
    || { echo "self-test: A/B storage growth was not reported against its fact" >&2; failures=1; }
  # 9e. No round on one side is a usage error.
  rm -f "$ab"/candidate-*.json
  status=0
  compare_ab "$ab" "$tmp/baseline.json" 15 "$runner" 5 > /dev/null 2>&1 || status=$?
  [[ "$status" -eq 2 ]] || { echo "self-test: an A/B directory without candidate rounds was not refused" >&2; failures=1; }

  if [[ "$failures" -ne 0 ]]; then
    echo "storage-bench-compare: SELF-TEST FAILED" >&2
    return 1
  fi
  echo "storage-bench-compare: self-test passed (14 cases)."
  return 0
}

if [[ "$SELF_TEST" -eq 1 ]]; then
  SUMMARY=""
  self_test
  exit $?
fi

# Paths named on the command line are the CALLER's, and the working directory
# moves to the repository root next.
absolute() { local path="$1"; case "$path" in /*) printf '%s\n' "$path" ;; *) printf '%s\n' "$PWD/$path" ;; esac; }
[[ -z "$MEASURED" ]] || MEASURED="$(absolute "$MEASURED")"
[[ -z "$BASELINE" ]] || BASELINE="$(absolute "$BASELINE")"
[[ -z "$SUMMARY" ]] || SUMMARY="$(absolute "$SUMMARY")"
[[ -z "$AB_DIR" ]] || AB_DIR="$(absolute "$AB_DIR")"

cd "$(dirname "$0")/../.."

[[ "$TOLERANCE" =~ ^[0-9]+(\.[0-9]+)?$ ]] \
  || { echo "error: --tolerance takes a non-negative number, not '$TOLERANCE'" >&2; exit 2; }

[[ "$SIZE_BUDGET" =~ ^[0-9]+(\.[0-9]+)?$ ]] \
  || { echo "error: --size-budget takes a non-negative number, not '$SIZE_BUDGET'" >&2; exit 2; }

if [[ -n "$AB_DIR" ]]; then
  [[ -d "$AB_DIR" ]] || { echo "error: no such directory: $AB_DIR" >&2; exit 2; }
  first="$(find "$AB_DIR" -maxdepth 1 -name 'candidate-*.json' | sort | sed -n 1p)"
  [[ -n "$first" ]] || { echo "error: $AB_DIR holds no candidate-*.json round record" >&2; exit 2; }
  committed="docs/benchmarks/storage/$(jq -r '.schema_generation' "$first")/$(jq -r '.bench_class' "$first")/record.json"
  committed_tmp=""
  if git cat-file -e "HEAD:$committed" 2>/dev/null; then
    committed_tmp="$(mktemp)"
    trap 'rm -f "$committed_tmp"' EXIT
    git show "HEAD:$committed" > "$committed_tmp"
  fi
  compare_ab "$AB_DIR" "$committed_tmp" "$TOLERANCE" "$RUNNER" "$SIZE_BUDGET"
  exit $?
fi

if [[ -z "$MEASURED" ]]; then
  MEASURED="$(discover_measured)" \
    || { echo "error: no docs/benchmarks/storage/*/*/record.json on disk — run the bench first" >&2; exit 2; }
fi
[[ -f "$MEASURED" ]] || { echo "error: no such record: $MEASURED" >&2; exit 2; }

generation="$(jq -r '.schema_generation' "$MEASURED")"

# The baseline is the COMMITTED record: the run overwrote the working-tree copy.
baseline_tmp=""
if [[ -z "$BASELINE" ]]; then
  committed="docs/benchmarks/storage/$generation/$(jq -r '.bench_class' "$MEASURED")/record.json"
  if git cat-file -e "HEAD:$committed" 2>/dev/null; then
    baseline_tmp="$(mktemp)"
    trap 'rm -f "$baseline_tmp"' EXIT
    git show "HEAD:$committed" > "$baseline_tmp"
    BASELINE="$baseline_tmp"
  else
    missing_baseline "$generation" "$committed" || exit $?
  fi
fi
[[ -f "$BASELINE" ]] || { echo "error: no such record: $BASELINE" >&2; exit 2; }

[[ "$SIZE_BUDGET" =~ ^[0-9]+(\.[0-9]+)?$ ]] \
  || { echo "error: --size-budget takes a non-negative number, not '$SIZE_BUDGET'" >&2; exit 2; }

compare "$MEASURED" "$BASELINE" "$TOLERANCE" "$RUNNER" "$SIZE_BUDGET"
