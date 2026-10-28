#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The verdict on the logging component's stated performance (EHDS Annex II
# 1.1), from the load test's reports with the access log off and on
# (scripts/perf/audit.sh).
#
# Each directory holds the run-*.json reports of one side, the same odd number
# of runs on both; the "on" reports carry the load test's `audit` section. The
# claims, stated in website/book/src/compliance/logging-performance.md:
#   completeness  in every run, every composition create and read has exactly
#                 one access record and every AQL execution at least one;
#   latency       the median p50 and p95 with the log on are at most
#                 MAX_LATENCY_RATIO of those with it off;
#   throughput    the median throughput with the log on is at least
#                 MIN_THROUGHPUT_RATIO of that with it off;
#   retrieval     the median p95 of a page of the measured window's records
#                 through ITI-81 is at most MAX_RETRIEVAL_P95_MS.
# No specification names a method or a figure: our own design.
#
# Usage: scripts/perf/audit-verdict.sh <off-dir> <on-dir>
#        scripts/perf/audit-verdict.sh --self-test
#   Prints a Markdown summary on stdout. Exit 0 when every claim holds, 1 when
#   one does not, 2 on a usage or data error.
set -euo pipefail

readonly MAX_LATENCY_RATIO=1.25
readonly MIN_THROUGHPUT_RATIO=0.80
readonly MAX_RETRIEVAL_P95_MS=250

usage() {
  echo "usage: $0 <off-dir> <on-dir> | --self-test" >&2
  exit 2
}

# reports DIR: the run-*.json files of DIR as one JSON array, or exit 2.
reports() {
  local dir="$1"
  local -a files
  shopt -s nullglob
  files=("$dir"/run-*.json)
  shopt -u nullglob
  if ((${#files[@]} == 0)); then
    echo "error: $dir holds no run-*.json report" >&2
    exit 2
  fi
  if ((${#files[@]} % 2 == 0)); then
    echo "error: $dir holds ${#files[@]} reports; the median needs an odd number" >&2
    exit 2
  fi
  jq -s '
    if any(.[]; (.throughput_ops_per_second | type) != "number"
                or (.p50_ms | type) != "number" or (.p95_ms | type) != "number")
    then error("a report lacks throughput_ops_per_second, p50_ms or p95_ms") else . end
  ' "${files[@]}" 2>/dev/null || {
    echo "error: the reports in $dir do not parse as load test reports" >&2
    exit 2
  }
}

verdict() {
  local off on
  off="$(reports "$1")" || return 2
  on="$(reports "$2")" || return 2
  if [[ "$(jq length <<<"$off")" != "$(jq length <<<"$on")" ]]; then
    echo "error: the two sides hold different numbers of runs" >&2
    return 2
  fi
  if ! jq -e 'all(.[]; (.audit.complete | type) == "boolean"
                      and (.audit.retrieval.p95_ms | type) == "number")' <<<"$on" >/dev/null; then
    echo "error: an \"on\" report lacks the audit section (run the load test with --audit)" >&2
    return 2
  fi
  local out
  out="$(jq -nr --argjson off "$off" --argjson on "$on" \
    --argjson lmax "$MAX_LATENCY_RATIO" --argjson tmin "$MIN_THROUGHPUT_RATIO" \
    --argjson rmax "$MAX_RETRIEVAL_P95_MS" '
    def median: sort | .[length / 2 | floor];
    def pct: . * 1000 | round / 10 | tostring + " %";
    def num: . * 10 | round / 10 | tostring;
    def ratio($a; $b): if $b > 0 then $a / $b else 1 end;
    def ok($bad): if $bad then "**not met**" else "met" end;
    ($off | map(.p50_ms) | median) as $p50off | ($on | map(.p50_ms) | median) as $p50on
    | ($off | map(.p95_ms) | median) as $p95off | ($on | map(.p95_ms) | median) as $p95on
    | ($off | map(.throughput_ops_per_second) | median) as $toff
    | ($on | map(.throughput_ops_per_second) | median) as $ton
    | ($on | map(.audit.retrieval.p95_ms) | median) as $ret
    | ($on | all(.[]; .audit.complete)) as $complete
    | (ratio($p50on; $p50off) > $lmax) as $p50bad
    | (ratio($p95on; $p95off) > $lmax) as $p95bad
    | (ratio($ton; $toff) < $tmin) as $tbad
    | ($ret > $rmax) as $rbad
    | "### Logging component: stated performance against the measured",
      "",
      "Median of \($on | length) runs with the access log off and on (fail mode closed, local store).",
      "",
      "| claim | log off | log on | measured | stated | verdict |",
      "|---|---|---|---|---|---|",
      "| every access recorded (creates and reads exactly once, AQL at least once) | | \($on | map(.audit.records_by_operation | to_entries | map("\(.key) \(.value)") | join(", ")) | .[0]) | \(if $complete then "every run complete" else "a run incomplete" end) | every run complete | \(ok($complete | not)) |",
      "| p50 latency (ms) | \($p50off | num) | \($p50on | num) | \(ratio($p50on; $p50off) | pct) | at most \($lmax | pct) | \(ok($p50bad)) |",
      "| p95 latency (ms) | \($p95off | num) | \($p95on | num) | \(ratio($p95on; $p95off) | pct) | at most \($lmax | pct) | \(ok($p95bad)) |",
      "| throughput (op/s) | \($toff | num) | \($ton | num) | \(ratio($ton; $toff) | pct) | at least \($tmin | pct) | \(ok($tbad)) |",
      "| ITI-81 page of \($on[0].audit.page_size) records, p95 (ms) | | \($ret | num) | \($ret | num) ms | at most \($rmax) ms | \(ok($rbad)) |",
      "",
      (if ($complete | not) or $p50bad or $p95bad or $tbad or $rbad
       then "VERDICT: a claim is not met" else "VERDICT: every claim met" end)
  ')"
  printf '%s\n' "$out"
  [[ "$(tail -n 1 <<<"$out")" == "VERDICT: every claim met" ]] || return 1
}

self_test() {
  local tmp out status
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN
  # report DIR N THROUGHPUT P50 P95 [COMPLETE RETRIEVAL_P95]: writes run-N.json.
  report() {
    local dir="$1" n="$2" t="$3" p50="$4" p95="$5" complete="${6:-}" ret="${7:-}"
    mkdir -p "$dir"
    local audit=""
    if [[ -n "$complete" ]]; then
      audit=",\"audit\":{\"records_by_operation\":{\"composition_create\":6,\"composition_get\":8,\"query_execute_adhoc_query\":7},\"operations_by_operation\":{\"composition_create\":6,\"composition_get\":8,\"query_execute_adhoc_query\":6},\"complete\":$complete,\"settled_after_seconds\":1,\"retrieval\":{\"count\":50,\"p50_ms\":1,\"p95_ms\":$ret},\"page_size\":50}"
    fi
    printf '{"workers":8,"operations":20,"elapsed_seconds":1,"throughput_ops_per_second":%s,"p50_ms":%s,"p95_ms":%s,"by_operation":{}%s}\n' \
      "$t" "$p50" "$p95" "$audit" >"$dir/run-$n.json"
  }
  # expect NAME WANT_STATUS WANT_LINE OFF ON
  expect() {
    local name="$1" want_status="$2" want_line="$3" off="$4" on="$5"
    status=0
    out="$(verdict "$off" "$on" 2>&1)" || status=$?
    if [[ "$status" != "$want_status" ]] || ! grep -qF -- "$want_line" <<<"$out"; then
      echo "self-test FAILED: $name (exit $status, wanted $want_status and '$want_line')" >&2
      echo "$out" >&2
      exit 1
    fi
  }
  for n in 1 2 3; do report "$tmp/off" "$n" 100 10 20; done
  report "$tmp/pass" 1 90 12 24 true 100
  report "$tmp/pass" 2 95 11 22 true 90
  report "$tmp/pass" 3 10 99 99 true 900
  expect "a run within every claim passes, the median ignores one outlier" 0 "VERDICT: every claim met" "$tmp/off" "$tmp/pass"
  report "$tmp/gap" 1 95 11 22 true 90
  report "$tmp/gap" 2 95 11 22 false 90
  report "$tmp/gap" 3 95 11 22 true 90
  expect "one incomplete run fails completeness" 1 "a run incomplete" "$tmp/off" "$tmp/gap"
  for n in 1 2 3; do report "$tmp/slow" "$n" 95 12.6 20 true 90; done
  expect "a p50 above 125 % is not met" 1 "| p50 latency (ms) | 10 | 12.6 | 126 % | at most 125 % | **not met** |" "$tmp/off" "$tmp/slow"
  for n in 1 2 3; do report "$tmp/thin" "$n" 79 10 20 true 90; done
  expect "throughput below 80 % is not met" 1 "| throughput (op/s) | 100 | 79 | 79 % | at least 80 % | **not met** |" "$tmp/off" "$tmp/thin"
  for n in 1 2 3; do report "$tmp/late" "$n" 100 10 20 true 251; done
  expect "a retrieval p95 above 250 ms is not met" 1 "at most 250 ms | **not met** |" "$tmp/off" "$tmp/late"
  for n in 1 2 3; do report "$tmp/edge" "$n" 80 12.5 25 true 250; done
  expect "exactly at every budget passes" 0 "VERDICT: every claim met" "$tmp/off" "$tmp/edge"
  for n in 1 2 3; do report "$tmp/bare" "$n" 100 10 20; done
  expect "an on side without the audit section is refused" 2 "lacks the audit section" "$tmp/off" "$tmp/bare"
  mkdir -p "$tmp/two"
  report "$tmp/two" 1 100 10 20 true 90
  report "$tmp/two" 2 100 10 20 true 90
  expect "an even number of runs is refused" 2 "the median needs an odd number" "$tmp/off" "$tmp/two"
  echo "audit-verdict.sh self-test: 8 cases passed"
}

case "${1:-}" in
--self-test)
  [[ "$#" -eq 1 ]] || usage
  self_test
  ;;
*)
  [[ "$#" -eq 2 ]] || usage
  verdict "$1" "$2"
  ;;
esac
