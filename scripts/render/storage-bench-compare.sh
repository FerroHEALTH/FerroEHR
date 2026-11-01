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
# code differed by up to 156 %). Two verdicts come out of the rounds. The plan
# verdict reads pages per call, which the runner's speed does not change: an
# operation fails when the candidate's median is more than the page tolerance
# and one page above the baseline's highest round. The time verdict divides
# each round by its machine factor, so a round where the whole runner slowed
# does not read as the code; an operation then regresses only when its median
# rose beyond the tolerance AND every candidate round is slower than every
# baseline round at that percentile, and the candidate build as a whole
# regresses when every one of its rounds ran slower across the board than
# every baseline round, by more than the tolerance at the median. The storage
# sizes do not depend on the machine and are still judged against the
# committed record.
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
#     --page-tolerance <pct>
#                          A/B mode: how far an operation's median pages per
#                          call may rise above the baseline's highest round
#                          before the run fails (default 10)
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
  echo "usage: scripts/render/storage-bench-compare.sh [--measured <file>] [--baseline <file>] [--tolerance <pct>] [--page-tolerance <pct>] [--size-budget <pct>] [--summary <file>] [--runner <text>] [--ab <dir>] | --self-test" >&2
  exit 2
}

MEASURED=""
BASELINE=""
TOLERANCE="15"
PAGE_TOLERANCE="10"
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
    --page-tolerance) PAGE_TOLERANCE="${2:-}"; shift 2 || usage ;;
    --size-budget) SIZE_BUDGET="${2:-}"; shift 2 || usage ;;
    --summary) SUMMARY="${2:-}"; shift 2 || usage ;;
    --runner) RUNNER="${2:-}"; shift 2 || usage ;;
    --ab) AB_DIR="${2:-}"; shift 2 || usage ;;
    --self-test) SELF_TEST=1; shift ;;
    *) usage ;;
  esac
done

command -v jq >/dev/null || { echo "error: jq is required" >&2; exit 2; }

# A rise of less than one page per call is never read as a plan change: the
# counters are the whole database's, and an operation that reads one or two
# pages moved by about a page between rounds of one build (`retention_prune`
# 1.3 to 2.6, `ehr_by_subject` 1.2 to 4.2, runs 41105458534 and 41105463959).
readonly PAGE_FLOOR=1

# The page tolerance holds at the `s` class only with eight rounds a build
# (#4627): `contains_population` read 210 to 695 pages per call across the 16
# `s` rounds of runs 41263871147 and 40945113344, and resampling them fails
# unchanged code in about 3.6 % of runs at four rounds and 0.26 % at eight.
# The workflow refuses fewer than eight rounds for `s`, so the bound stays 10 %.

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

# The A/B programs (#4318) share these definitions. `$B` and `$M` are the
# baseline's and the candidate's round records. A round's machine factor is the
# median, over the operations, of the round's p50 divided by that operation's
# median p50 across every round of both builds: 1.22 says the whole runner ran
# 22 % slow in that round. One factor is taken from the p50s and applies to both
# percentiles, because a p99 over ten samples is the slowest sample and says
# little about the machine. Pages per call are the database's shared-buffer
# hits plus reads over the operation's iterations, a count of the plan's work
# that the runner's speed does not change.
# shellcheck disable=SC2016 # every $name below is a jq variable, never a shell expansion
readonly AB_DEFS_JQ='
def t: if . == null then "—"
       elif . >= 1000000000 then ((. / 100000000 | round) / 10 | tostring) + " s"
       elif . >= 1000000 then ((. / 100000 | round) / 10 | tostring) + " ms"
       else ((. / 100 | round) / 10 | tostring) + " µs" end;
def pc: if . == null then "—"
        else (if . >= 0 then "+" else "" end) + ((. * 10 | round) / 10 | tostring) + "%" end;
def pg: if . == null then "—" else ((. * 10 | round) / 10 | tostring) end;
def rise($base; $meas): if ($base == null or $meas == null or $base == 0) then null
                        else (($meas - $base) / $base * 100) end;
def med: sort | if length == 0 then null
                elif length % 2 == 1 then .[(length - 1) / 2]
                else (.[length / 2 - 1] + .[length / 2]) / 2 end;
def ops: [.groups[]? | .operations[]?];
def op($name): [ops[] | select(.name == $name)] | .[0];
def pages: if . == null or .database == null or (.iterations // 0) == 0
              or .database.blks_hit_delta == null or .database.blks_read_delta == null then null
           else (.database.blks_hit_delta + .database.blks_read_delta) / .iterations end;
def op_medians($all): [$all[] | ops[] | select(.p50_ns != null)] | group_by(.name)
                      | map({key: .[0].name, value: (map(.p50_ns) | med)}) | from_entries;
def factor($meds): [ops[] | select(.p50_ns != null and ($meds[.name] // 0) > 0) | .p50_ns / $meds[.name]]
                   | if length == 0 then 1 else med end;
def series($records; $factors; $name; $field):
  [range(0; $records | length) as $i | ($records[$i] | op($name) | .[$field]?) as $v
   | select($v != null) | $v / $factors[$i]];
def page_series($records; $name): [$records[] | op($name) | pages | select(. != null)];
def shown($xs): if ($xs | length) == 0 then "—"
                elif ($xs | length) == 1 then ($xs[0] | t)
                else "\($xs | med | t) (\($xs | min | t)–\($xs | max | t))" end;
def worst($d50; $d99): if $d50 == null and $d99 == null then null
                       elif $d50 == null then {value: $d99, at: "p99"}
                       elif $d99 == null then {value: $d50, at: "p50"}
                       elif $d99 > $d50 then {value: $d99, at: "p99"}
                       else {value: $d50, at: "p50"} end;
$b[0] as $B | $m[0] as $M
| op_medians($B + $M) as $meds
| ($B | map(factor($meds))) as $bf | ($M | map(factor($meds))) as $mf
'

# One row per operation. The latency verdict reads the round-adjusted times
# (each round's p50 and p99 divided by its machine factor), and the table shows
# the raw rounds beside the adjusted change. A rise beyond the tolerance is a
# regression only when every candidate round is slower than every baseline
# round at that percentile; a rise the rounds cannot separate reads as noise.
# The page verdict fails an operation whose candidate median reads more than
# the page tolerance, and at least one page, above the baseline's highest
# round: the counters are the whole database's, so a round can carry
# background work, and a plan the baseline reached in one round is not new.
# shellcheck disable=SC2016 # every $name below is a jq variable, never a shell expansion
readonly COMPARE_AB_JQ='
([$M[] | ops[] | .name] + [$B[] | ops[] | .name]
 | reduce .[] as $n ([]; if index($n) then . else . + [$n] end)) as $names
| ($B | map(1)) as $b1 | ($M | map(1)) as $m1
| $names[]
| . as $n
| series($B; $b1; $n; "p50_ns") as $b50s | series($B; $b1; $n; "p99_ns") as $b99s
| series($M; $m1; $n; "p50_ns") as $m50s | series($M; $m1; $n; "p99_ns") as $m99s
| series($B; $bf; $n; "p50_ns") as $a50s | series($B; $bf; $n; "p99_ns") as $a99s
| series($M; $mf; $n; "p50_ns") as $c50s | series($M; $mf; $n; "p99_ns") as $c99s
| worst(rise($b50s | med; $m50s | med); rise($b99s | med; $m99s | med)) as $raw
| worst(rise($a50s | med; $c50s | med); rise($a99s | med; $c99s | med)) as $adj
| (if $adj == null then false
   elif $adj.at == "p99" then ($c99s | min) <= ($a99s | max)
   else ($c50s | min) <= ($a50s | max) end) as $overlap
| (if ($b50s | length) == 0 and ($b99s | length) == 0 then ["no-baseline", "not measured on the baseline"]
   elif ($m50s | length) == 0 and ($m99s | length) == 0 then ["not-measured", "not measured on the candidate"]
   elif $compare | not then ["not-compared", "not compared"]
   elif $adj == null then ["not-measured", "not measured on the candidate"]
   elif $adj.value < 0 then ["faster", "faster"]
   elif $adj.value <= $tolerance then ["within", "within tolerance"]
   elif $overlap then ["noise", "within the noise of the rounds"]
   else ["regressed", "**regressed**"] end) as $verdict
| page_series($B; $n) as $bps | page_series($M; $n) as $mps
| (if ($bps | length) == 0 or ($mps | length) == 0 then ["unpaged", "not counted"]
   elif $compare | not then ["not-compared", "not compared"]
   elif ($mps | med) > ($bps | max) * (1 + $page_tolerance / 100)
        and ($mps | med) - ($bps | max) >= $page_floor then ["more-pages", "**more pages**"]
   else ["same-pages", "same plan"] end) as $plan
| "\($verdict[0])\t\($plan[0])\t| `\($n)` | \(shown($b50s)) | \(shown($b99s)) | \(shown($m50s)) | \(shown($m99s)) | " +
  (if $adj == null then "— | " else "\($adj.value|pc) (\($adj.at)) | " end) +
  (if $raw == null then "— | " else "\($raw.value|pc) | " end) +
  "\($verdict[1]) | \($bps | med | pg) (≤ \($bps | max | pg)) | \($mps | med | pg) | \($plan[1]) |"
'

# The rounds in the order they ran, each with its machine factor, then the
# build-wide verdict: the candidate's rounds as a whole regress when the
# median of their factors is more than the tolerance above the baseline's AND
# every candidate round's factor is above every baseline round's. Interleaved
# rounds share the runner's slow spells, so a slowdown in every candidate
# round and in no baseline round is the code's.
# shellcheck disable=SC2016 # every $name below is a jq variable, never a shell expansion
readonly ROUNDS_AB_JQ='
( [range(0; $B | length) as $i | {side: "baseline", at: $B[$i].measured_at, f: $bf[$i]}]
  + [range(0; $M | length) as $i | {side: "candidate", at: $M[$i].measured_at, f: $mf[$i]}]
  | sort_by(.at)
  | .[]
  | "round\t| \(.at[0:19] | sub("T"; " ")) | \(.side) | \((.f * 1000 | round) / 1000) | " +
    (if .f > 1 + $tolerance / 100 then "slower across the board"
     elif .f < 1 - $tolerance / 100 then "faster across the board"
     else "" end) + " |" ),
( rise($bf | med; $mf | med) as $d
  | (($mf | min) > ($bf | max)) as $separated
  | (if $compare | not then "not-compared"
     elif $d <= $tolerance then "within"
     elif $separated then "regressed"
     else "noise" end) as $v
  | "build\t\($v)\t\($d | pc)" )
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

# The A/B verdict (#4318): latency and pages per call between the round records
# in `$1`, sizes of the first candidate round against the committed record `$2`
# (empty when the candidate's generation has none, which fails the run as
# before).
compare_ab() {
  local dir="$1" committed="$2" tolerance="$3" runner="$4" budget="${5:-5}" page_tolerance="${6:-10}"
  local -a base=() cand=()
  local classes compare_flag rows verdict plan row regressed=0 noise=0 grew=0 paged=0 generation
  local build_verdict="" build_rise="" failed=0
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

  # Both programs read the rounds the same way, so they run over one input.
  ab_jq() {
    jq -r -n \
      --slurpfile b <(jq -s '.' "${base[@]}") \
      --slurpfile m <(jq -s '.' "${cand[@]}") \
      --argjson tolerance "$tolerance" \
      --argjson page_tolerance "$page_tolerance" \
      --argjson page_floor "$PAGE_FLOOR" \
      --argjson compare "$compare_flag" \
      "$AB_DEFS_JQ | $1"
  }

  emit "## Storage benchmark, baseline against candidate on one runner" ""
  emit "Exploration, never a conformance record: this lane measures the storage" \
       "layer below the wire and earns no performance class. The procedure is in" \
       "\`docs/benchmarks/storage/README.md\`." ""
  emit "- Baseline (${#base[@]} rounds): $(describe_record "${base[0]}")"
  emit "- Candidate (${#cand[@]} rounds): $(describe_record "${cand[0]}")"
  [[ -z "$runner" ]] || emit "- Runner: $runner"
  emit "- Plan: an operation fails when the candidate's median pages per call (shared" \
       "  buffers hit plus read, over the iterations) is more than ${page_tolerance}% and at" \
       "  least ${PAGE_FLOOR} page above the baseline's highest round, whatever its time." \
       "- Time: each round's p50 and p99 are divided by the round's machine factor (the" \
       "  median, over the operations, of the round's p50 against that operation's" \
       "  median of every round). An operation regresses when its adjusted median p50" \
       "  or p99 rose more than ${tolerance}% above the baseline's and every candidate round" \
       "  is slower than every baseline round at that percentile; a rise the rounds" \
       "  cannot separate reads as noise. The candidate build as a whole regresses" \
       "  when the median of its rounds' factors is more than ${tolerance}% above the" \
       "  baseline's and every candidate round's factor is above every baseline round's." ""
  if [[ "$compare_flag" == false ]]; then
    emit "The rounds measured different corpora (\`$classes\`), so the deltas below" \
         "are reported and nothing is judged." ""
  fi

  emit "Rounds in the order they ran:" "" \
       "| Recorded at | Build | Machine factor | Flag |" \
       "|---|---|---|---|"
  while IFS=$'\t' read -r verdict plan row; do
    case "$verdict" in
      round) emit "$plan" ;;
      build) build_verdict="$plan"; build_rise="$row" ;;
    esac
  done < <(ab_jq "$ROUNDS_AB_JQ")
  emit ""

  rows="$(ab_jq "$COMPARE_AB_JQ")"
  emit "| Operation | Baseline p50 | Baseline p99 | Candidate p50 | Candidate p99 | Delta, adjusted | Delta, raw | Time | Baseline pages (highest) | Candidate pages | Plan |" \
       "|---|---|---|---|---|---|---|---|---|---|---|"
  local -a red=()
  while IFS=$'\t' read -r verdict plan row; do
    [[ -n "$row" ]] || continue
    emit "$row"
    case "$verdict" in
      regressed) regressed=$((regressed + 1)); red+=("$row") ;;
      noise) noise=$((noise + 1)) ;;
    esac
    if [[ "$plan" == "more-pages" ]]; then
      paged=$((paged + 1))
      [[ "$verdict" == "regressed" ]] || red+=("$row")
    fi
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
  case "$build_verdict" in
    within|not-compared) emit "The candidate's rounds ran ${build_rise} against the baseline's across the board (the median machine factor)." "" ;;
    noise) emit "The candidate's rounds ran ${build_rise} against the baseline's across the board, inside the spread of the rounds' machine factors." "" ;;
    regressed)
      emit "**The candidate build ran ${build_rise} against the baseline across the board, in every round.**" ""
      echo "storage-bench-compare: the candidate build ran ${build_rise} across the board, beyond the ${tolerance}% tolerance." >&2
      failed=1 ;;
  esac
  if [[ "$paged" -gt 0 ]]; then
    emit "$paged operation(s) read more than ${page_tolerance}% more pages per call than the baseline's highest round: a plan change." ""
    echo "storage-bench-compare: $paged operation(s) beyond the ${page_tolerance}% page tolerance." >&2
    failed=1
  fi
  if [[ "$grew" -gt 0 ]]; then
    emit "$grew storage fact(s) grew more than ${budget}% above the committed record." ""
    echo "storage-bench-compare: $grew storage fact(s) beyond the ${budget}% budget." >&2
    failed=1
  fi
  if [[ "$regressed" -gt 0 ]]; then
    emit "$regressed operation(s) rose more than ${tolerance}% above the baseline, outside the spread of the rounds." ""
    echo "storage-bench-compare: $regressed operation(s) beyond the ${tolerance}% tolerance." >&2
    failed=1
  fi
  [[ "$failed" -eq 0 ]] || return 1
  emit "No operation read more pages per call than the baseline and none rose more than ${tolerance}% above it outside the spread of the rounds." ""
  echo "storage-bench-compare: nothing beyond the ${tolerance}% tolerance or the ${page_tolerance}% page tolerance."
  return 0
}

# ── the self-test ───────────────────────────────────────────────────────────
# The verdict logic is what this script exists for, so it is exercised rather
# than trusted: an improvement, a rise inside the tolerance, a rise beyond it,
# a mismatched class, an operation each side carries alone, a record compared
# with itself, and a generation with no committed record.

record_fixture() {
  local path="$1" generation="$2" class="$3" ehrs="$4" p50="$5" p99="$6" when="${7:-2026-09-15T11:00:00Z}"
  # $8 and $9 are the bytes per composition version and per node row, $10 the
  # pages per call of `create_first_version`, and $11 a factor on the four
  # other operations' times, the way a slow machine moves every operation.
  local pages="${10:-20}" scale="${11:-1}"
  mkdir -p "$(dirname "$path")"
  fixed_op() {
    local name="$1" p50_ns="$2" p99_ns="$3" per_call="$4"
    printf '        { "name": "%s", "iterations": 100, "samples": 10, "mean_ns": %s, "median_ns": %s,\n' \
      "$name" "$(awk -v v="$p50_ns" -v s="$scale" 'BEGIN { print v * s }')" "$(awk -v v="$p50_ns" -v s="$scale" 'BEGIN { print v * s }')"
    printf '          "p50_ns": %s, "p95_ns": %s, "p99_ns": %s,\n' \
      "$(awk -v v="$p50_ns" -v s="$scale" 'BEGIN { print v * s }')" "$(awk -v v="$p99_ns" -v s="$scale" 'BEGIN { print v * s }')" "$(awk -v v="$p99_ns" -v s="$scale" 'BEGIN { print v * s }')"
    printf '          "database": { "blks_hit_delta": %s, "blks_read_delta": 0 } }' "$((per_call * 100))"
  }
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
          "mean_ns": $p50, "median_ns": $p50, "p50_ns": $p50, "p95_ns": $p99, "p99_ns": $p99,
          "database": { "blks_hit_delta": $(awk -v v="$pages" 'BEGIN { print v * 100 }'), "blks_read_delta": 0 } },
$(fixed_op supersede 2000000 3000000 2400),
$(fixed_op by_version_uid 250000 300000 17),
$(fixed_op revision_history 140000 160000 16),
$(fixed_op contains_one_ehr 2800000 3100000 560)
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
  # 9f. Both candidate rounds ran on a slower machine (every operation 5 and
  # 12 % up) and `create_first_version` 18 and 25 %: raw, it rose 21 % with the
  # rounds separated. Divided by each round's machine factor it rose 12 %, and
  # the build as a whole 8.5 %: within tolerance.
  rm -rf "$ab" && mkdir -p "$ab"
  record_fixture "$ab/baseline-1.json" generation-1 poc 20 1000000 2000000 2026-09-15T12:00:00Z
  record_fixture "$ab/baseline-2.json" generation-1 poc 20 1000000 2000000 2026-09-15T12:30:00Z
  record_fixture "$ab/candidate-1.json" generation-1 poc 20 1180000 2360000 2026-09-15T12:10:00Z 20000 461 20 1.05
  record_fixture "$ab/candidate-2.json" generation-1 poc 20 1250000 2500000 2026-09-15T12:20:00Z 20000 461 20 1.12
  status=0
  compare_ab "$ab" "$tmp/baseline.json" 15 "$runner" 5 > "$tmp/ab-machine.out" 2>&1 || status=$?
  out="$(cat "$tmp/ab-machine.out")"
  [[ "$status" -eq 0 ]] || { echo "self-test: a slow machine in the candidate's rounds failed the run" >&2; failures=1; }
  grep -q 'create_first_version.*+12%.*+21.5%.*within tolerance' <<<"$out" \
    || { echo "self-test: a slow machine was not divided out of the candidate's rounds" >&2; failures=1; }
  # 9g. Every operation 30 % slower in every candidate round: the machine
  # factors cannot tell that from the code, and the build regresses.
  rm -rf "$ab" && mkdir -p "$ab"
  record_fixture "$ab/baseline-1.json" generation-1 poc 20 1000000 2000000 2026-09-15T12:00:00Z
  record_fixture "$ab/baseline-2.json" generation-1 poc 20 1000000 2000000 2026-09-15T12:30:00Z
  record_fixture "$ab/candidate-1.json" generation-1 poc 20 1300000 2600000 2026-09-15T12:10:00Z 20000 461 20 1.3
  record_fixture "$ab/candidate-2.json" generation-1 poc 20 1300000 2600000 2026-09-15T12:20:00Z 20000 461 20 1.3
  status=0
  compare_ab "$ab" "$tmp/baseline.json" 15 "$runner" 5 > "$tmp/ab-build.out" 2>&1 || status=$?
  out="$(cat "$tmp/ab-build.out")"
  [[ "$status" -eq 1 ]] || { echo "self-test: a build-wide slowdown in every candidate round did not fail the run" >&2; failures=1; }
  grep -q 'candidate build ran +30% against the baseline across the board, in every round' <<<"$out" \
    || { echo "self-test: a build-wide slowdown was not named" >&2; failures=1; }
  # 9h. The same times with fifteen times the pages per call: a plan change
  # fails the run whatever the clock says.
  rm -rf "$ab" && mkdir -p "$ab"
  record_fixture "$ab/baseline-1.json" generation-1 poc 20 1000000 2000000 2026-09-15T12:00:00Z
  record_fixture "$ab/baseline-2.json" generation-1 poc 20 1000000 2000000 2026-09-15T12:30:00Z
  record_fixture "$ab/candidate-1.json" generation-1 poc 20 1000000 2000000 2026-09-15T12:10:00Z 20000 461 300
  record_fixture "$ab/candidate-2.json" generation-1 poc 20 1000000 2000000 2026-09-15T12:20:00Z 20000 461 300
  status=0
  compare_ab "$ab" "$tmp/baseline.json" 15 "$runner" 5 10 > "$tmp/ab-plan.out" 2>&1 || status=$?
  out="$(cat "$tmp/ab-plan.out")"
  [[ "$status" -eq 1 ]] || { echo "self-test: fifteen times the pages per call did not fail the run" >&2; failures=1; }
  grep -q 'create_first_version.*within tolerance.*more pages' <<<"$out" \
    || { echo "self-test: a plan change at an unchanged time was not named" >&2; failures=1; }
  grep -q 'supersede.*same plan' <<<"$out" \
    || { echo "self-test: an unchanged plan beside a plan change was misjudged" >&2; failures=1; }
  # 9i. Pages a baseline round already read (background work in the database
  # counters, 20 and 60) and a rise under one page (1.3 to 2.2) are no plan change.
  rm -rf "$ab" && mkdir -p "$ab"
  record_fixture "$ab/baseline-1.json" generation-1 poc 20 1000000 2000000 2026-09-15T12:00:00Z 20000 461 20
  record_fixture "$ab/baseline-2.json" generation-1 poc 20 1000000 2000000 2026-09-15T12:30:00Z 20000 461 60
  record_fixture "$ab/candidate-1.json" generation-1 poc 20 1000000 2000000 2026-09-15T12:10:00Z 20000 461 21
  record_fixture "$ab/candidate-2.json" generation-1 poc 20 1000000 2000000 2026-09-15T12:20:00Z 20000 461 64
  status=0
  compare_ab "$ab" "$tmp/baseline.json" 15 "$runner" 5 10 > "$tmp/ab-pages-noise.out" 2>&1 || status=$?
  out="$(cat "$tmp/ab-pages-noise.out")"
  [[ "$status" -eq 0 ]] || { echo "self-test: pages inside the baseline's highest round failed the run" >&2; failures=1; }
  grep -q 'create_first_version.*same plan' <<<"$out" \
    || { echo "self-test: pages inside the baseline's highest round read as a plan change" >&2; failures=1; }
  record_fixture "$ab/baseline-1.json" generation-1 poc 20 1000000 2000000 2026-09-15T12:00:00Z 20000 461 1.3
  record_fixture "$ab/baseline-2.json" generation-1 poc 20 1000000 2000000 2026-09-15T12:30:00Z 20000 461 1.3
  record_fixture "$ab/candidate-1.json" generation-1 poc 20 1000000 2000000 2026-09-15T12:10:00Z 20000 461 2.1
  record_fixture "$ab/candidate-2.json" generation-1 poc 20 1000000 2000000 2026-09-15T12:20:00Z 20000 461 2.2
  status=0
  compare_ab "$ab" "$tmp/baseline.json" 15 "$runner" 5 10 > "$tmp/ab-pages-floor.out" 2>&1 || status=$?
  out="$(cat "$tmp/ab-pages-floor.out")"
  [[ "$status" -eq 0 ]] || { echo "self-test: a rise under one page per call failed the run" >&2; failures=1; }
  grep -q 'create_first_version.*same plan' <<<"$out" \
    || { echo "self-test: a rise under one page per call read as a plan change" >&2; failures=1; }
  # 9e. No round on one side is a usage error.
  rm -f "$ab"/candidate-*.json
  status=0
  compare_ab "$ab" "$tmp/baseline.json" 15 "$runner" 5 > /dev/null 2>&1 || status=$?
  [[ "$status" -eq 2 ]] || { echo "self-test: an A/B directory without candidate rounds was not refused" >&2; failures=1; }

  if [[ "$failures" -ne 0 ]]; then
    echo "storage-bench-compare: SELF-TEST FAILED" >&2
    return 1
  fi
  echo "storage-bench-compare: self-test passed (19 cases)."
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

[[ "$PAGE_TOLERANCE" =~ ^[0-9]+(\.[0-9]+)?$ ]] \
  || { echo "error: --page-tolerance takes a non-negative number, not '$PAGE_TOLERANCE'" >&2; exit 2; }

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
  compare_ab "$AB_DIR" "$committed_tmp" "$TOLERANCE" "$RUNNER" "$SIZE_BUDGET" "$PAGE_TOLERANCE"
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
