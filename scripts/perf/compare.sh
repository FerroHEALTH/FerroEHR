#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The performance guard's verdict: a release candidate against the previous
# release, from the load test's reports of both (tools/loadtest).
#
# Each directory holds the run-*.json reports of one side, the same odd number
# of runs on both. The verdict compares the median of each side: the candidate
# fails when its throughput is below 90 % of the previous release's, or its
# p95 latency is above 110 %. The per-operation medians are printed beside the
# verdict for diagnosis and decide nothing. No openEHR spec governs this: our
# own design.
#
# Usage: scripts/perf/compare.sh <previous-dir> <candidate-dir>
#        scripts/perf/compare.sh --self-test
#   Prints a Markdown summary on stdout. Exit 0 within budget, 1 on a
#   regression, 2 on a usage or data error.
set -euo pipefail

readonly MIN_THROUGHPUT_RATIO=0.90
readonly MAX_P95_RATIO=1.10

usage() {
  echo "usage: $0 <previous-dir> <candidate-dir> | --self-test" >&2
  exit 2
}

# side DIR: one JSON object with the medians of DIR's reports, or exit 2.
side() {
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
    def median: sort | .[length / 2 | floor];
    if any(.[]; (.throughput_ops_per_second | type) != "number" or (.p95_ms | type) != "number")
    then error("a report lacks throughput_ops_per_second or p95_ms") else . end
    | . as $all
    | {
        runs: length,
        throughput: (map(.throughput_ops_per_second) | median),
        p95: (map(.p95_ms) | median),
        ops: ([$all[] | .by_operation // {} | keys[]] | unique
              | map(. as $k | {key: $k, value: ([$all[] | .by_operation[$k].p95_ms // 0] | median)})
              | from_entries)
      }
  ' "${files[@]}" 2>/dev/null || {
    echo "error: the reports in $dir do not parse as load test reports" >&2
    exit 2
  }
}

compare() {
  local prev_dir="$1" cand_dir="$2" prev cand
  prev="$(side "$prev_dir")" || return 2
  cand="$(side "$cand_dir")" || return 2
  if [[ "$(jq -r .runs <<<"$prev")" != "$(jq -r .runs <<<"$cand")" ]]; then
    echo "error: the two sides hold different numbers of runs" >&2
    return 2
  fi
  local out
  out="$(jq -nr --argjson p "$prev" --argjson c "$cand" \
    --argjson tmin "$MIN_THROUGHPUT_RATIO" --argjson pmax "$MAX_P95_RATIO" '
    def pct: . * 1000 | round / 10 | tostring + " %";
    def num: . * 10 | round / 10 | tostring;
    ($c.throughput / $p.throughput) as $tr
    | (if $p.p95 > 0 then $c.p95 / $p.p95 else 1 end) as $pr
    | ($tr < $tmin) as $tbad
    | ($pr > $pmax) as $pbad
    | "### Performance guard: candidate against the previous release",
      "",
      "Median of \($p.runs) runs each.",
      "",
      "| | previous | candidate | ratio | budget | verdict |",
      "|---|---|---|---|---|---|",
      "| throughput (op/s) | \($p.throughput | num) | \($c.throughput | num) | \($tr | pct) | at least \($tmin | pct) | \(if $tbad then "**refused**" else "ok" end) |",
      "| p95 latency (ms) | \($p.p95 | num) | \($c.p95 | num) | \($pr | pct) | at most \($pmax | pct) | \(if $pbad then "**refused**" else "ok" end) |",
      "",
      "| operation | previous p95 (ms) | candidate p95 (ms) |",
      "|---|---|---|",
      (($p.ops + $c.ops) | keys[] as $k | "| \($k) | \($p.ops[$k] // 0 | num) | \($c.ops[$k] // 0 | num) |"),
      "",
      (if $tbad or $pbad then "VERDICT: regression" else "VERDICT: within budget" end)
  ')"
  printf '%s\n' "$out"
  [[ "$(tail -n 1 <<<"$out")" == "VERDICT: within budget" ]] || return 1
}

self_test() {
  local tmp out status
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN
  # report DIR N THROUGHPUT P95: writes run-N.json into DIR.
  report() {
    local dir="$1" n="$2" throughput="$3" p95="$4"
    mkdir -p "$dir"
    printf '{"workers":8,"operations":2000,"elapsed_seconds":1,"throughput_ops_per_second":%s,"p50_ms":1,"p95_ms":%s,"by_operation":{"aql_contains":{"count":1,"p50_ms":1,"p95_ms":%s}}}\n' \
      "$throughput" "$p95" "$p95" >"$dir/run-$n.json"
  }
  # side_of DIR T1 T2 T3 P1 P2 P3
  side_of() {
    local dir="$1" t1="$2" t2="$3" t3="$4" p1="$5" p2="$6" p3="$7"
    report "$dir" 1 "$t1" "$p1"
    report "$dir" 2 "$t2" "$p2"
    report "$dir" 3 "$t3" "$p3"
  }
  # expect NAME WANT_STATUS WANT_LINE PREV CAND
  expect() {
    local name="$1" want_status="$2" want_line="$3" prev="$4" cand="$5"
    status=0
    out="$(compare "$prev" "$cand" 2>&1)" || status=$?
    if [[ "$status" != "$want_status" ]] || ! grep -qF -- "$want_line" <<<"$out"; then
      echo "self-test FAILED: $name (exit $status, wanted $want_status and '$want_line')" >&2
      echo "$out" >&2
      exit 1
    fi
  }
  side_of "$tmp/prev" 1000 400 1100 20 90 22
  # Medians 1000 op/s and 22 ms; an outlier run on either side moves nothing.
  side_of "$tmp/pass" 950 960 3000 24 10 200
  expect "a candidate within budget passes" 0 "VERDICT: within budget" "$tmp/prev" "$tmp/pass"
  side_of "$tmp/slow" 899 880 2000 20 20 20
  expect "throughput below 90 % is refused" 1 "| throughput (op/s) | 1000 | 899 | 89.9 % | at least 90 % | **refused** |" "$tmp/prev" "$tmp/slow"
  side_of "$tmp/late" 1000 1000 1000 24.3 24.3 1
  expect "p95 latency above 110 % is refused" 1 "| p95 latency (ms) | 22 | 24.3 | 110.5 % | at most 110 % | **refused** |" "$tmp/prev" "$tmp/late"
  side_of "$tmp/prev20" 1000 1000 1000 20 20 20
  side_of "$tmp/edge" 900 900 900 22 22 22
  expect "exactly 90 % and 110 % pass" 0 "VERDICT: within budget" "$tmp/prev20" "$tmp/edge"
  mkdir -p "$tmp/two"
  report "$tmp/two" 1 1000 20
  report "$tmp/two" 2 1000 20
  expect "an even number of runs is refused" 2 "the median needs an odd number" "$tmp/prev" "$tmp/two"
  mkdir -p "$tmp/empty"
  expect "a side without reports is refused" 2 "holds no run-*.json report" "$tmp/prev" "$tmp/empty"
  mkdir -p "$tmp/bad"
  echo '{"throughput_ops_per_second":"fast"}' >"$tmp/bad/run-1.json"
  expect "a report without numbers is refused" 2 "do not parse as load test reports" "$tmp/bad" "$tmp/prev"
  echo "compare.sh self-test: 7 cases passed"
}

case "${1:-}" in
--self-test)
  [[ "$#" -eq 1 ]] || usage
  self_test
  ;;
-*) usage ;;
*)
  [[ "$#" -eq 2 ]] || usage
  compare "$1" "$2"
  ;;
esac
