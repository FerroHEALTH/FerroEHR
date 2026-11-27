#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The committed conformance record (docs/conformance/ferroehr/results.json and
# summary.md) against the runs it stands for (#6747).
#
# The record is the released-profile views of one Veredictum run, committed so
# the book and the landing page read their counts from it. It once said every
# judged row passed for three weeks while every run on main failed the four
# committer deviations, because nothing compared the two. This holds them
# together in two places:
#
#   check     offline, in ci.yml: the record is a released-profile run of the
#             pinned Veredictum, and its red rows are exactly the deviations
#             docs/conformance/party/ferroehr/deviations.json lists
#             (scripts/conformance-deviations.sh), so a record older than the
#             list fails.
#   compare   in conformance.yml, after the released run: every row of the run
#             has the status the record gives it, so a change that moves a row
#             commits the record of its own run in the same pull request.
#   refresh   writes the released-profile views of a run into the record.
#
# The rows carry generated inputs (fresh ids each run), so compare reads only
# each row's case, row number and status. No openEHR spec governs this guard:
# our own design.
#
# Usage:
#   scripts/conformance-record.sh check
#   scripts/conformance-record.sh compare <results.json>
#   scripts/conformance-record.sh refresh <run id>
#       the run's conformance-views-released artifact, read with gh
#   scripts/conformance-record.sh --self-test
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly ROOT
RECORD_DIR="$ROOT/docs/conformance/ferroehr"
readonly RECORD_DIR
DEVIATIONS="$ROOT/docs/conformance/party/ferroehr/deviations.json"
readonly DEVIATIONS

usage() {
  sed -n '27,32p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

pinned_version() {
  sed -nE 's/^VEREDICTUM_VERSION="([^"]+)"$/\1/p' "$ROOT/scripts/lib/veredictum.sh"
}

# check_record <results.json> <summary.md> <deviations.json> <pinned version>
check_record() {
  local results="$1" summary="$2" deviations="$3" pinned="$4" profile engine
  local bad=0
  for f in "$results" "$summary"; do
    [[ -f "$f" ]] || { echo "::error::$f is missing; commit the released-profile views of a conformance run"; return 1; }
  done
  profile="$(jq -r '.specification.profile // empty' "$results")"
  engine="$(jq -r '.engine.version // empty' "$results")"
  if [[ "$profile" != released ]]; then
    echo "::error::the committed record is a '${profile:-unnamed}' profile run; it holds the released profile"
    bad=1
  fi
  if [[ -z "$pinned" || "$engine" != "$pinned" ]]; then
    echo "::error::the committed record was judged by Veredictum ${engine:-unnamed} while the pin is ${pinned:-unreadable}; refresh it from a run of the pin"
    bad=1
  fi
  if ! grep -q "veredictum $engine" "$summary"; then
    echo "::error::$summary does not name the instrument the record names (veredictum $engine); commit both views of one run"
    bad=1
  fi
  if ! bash "$ROOT/scripts/conformance-deviations.sh" "$results" "$deviations"; then
    echo "::error::the committed record's red rows are not the listed deviations; refresh it from a run of this tree"
    bad=1
  fi
  return "$bad"
}

# compare_runs <run results.json> <record results.json>
compare_runs() {
  local run="$1" record="$2" diff
  diff="$(jq -rn --slurpfile run "$run" --slurpfile record "$record" '
    def statuses($doc): reduce $doc.rows[] as $r ({}; .["\($r.case) row \($r.row)"] = $r.status);
    statuses($run[0]) as $now | statuses($record[0]) as $then
    | ([$now, $then] | map(keys) | add | unique)[] as $k
    | select($now[$k] != $then[$k])
    | "\($k): the run says \($now[$k] // "absent"), the record says \($then[$k] // "absent")"
  ')"
  local run_engine record_engine run_profile
  run_engine="$(jq -r '.engine.version // empty' "$run")"
  record_engine="$(jq -r '.engine.version // empty' "$record")"
  run_profile="$(jq -r '.specification.profile // empty' "$run")"
  if [[ "$run_profile" != released ]]; then
    echo "::error::the run is a '${run_profile:-unnamed}' profile run; only a released run compares with the record"
    return 1
  fi
  if [[ "$run_engine" != "$record_engine" ]]; then
    diff="${diff:+$diff
}the run was judged by Veredictum $run_engine, the record by $record_engine"
  fi
  if [[ -n "$diff" ]]; then
    echo "::error::the committed record (docs/conformance/ferroehr/) differs from this run in $(grep -c . <<<"$diff") place(s); commit this run's released views with 'scripts/conformance-record.sh refresh <run id>'"
    head -n 40 <<<"$diff"
    return 1
  fi
  echo "conformance-record: the run gives every row the status the committed record gives it ($(jq '.rows | length' "$run") rows)"
}

refresh() {
  local run_id="$1" dir
  [[ "$run_id" =~ ^[0-9]+$ ]] || { echo "conformance-record: '$run_id' is not a run id" >&2; exit 2; }
  dir="$(mktemp -d)"
  trap 'rm -rf "$dir"' RETURN
  gh run download "$run_id" --repo FerroHEALTH/FerroEHR --name conformance-views-released --dir "$dir"
  for f in results.json summary.md; do
    [[ -f "$dir/$f" ]] || { echo "conformance-record: run $run_id's artifact holds no $f" >&2; exit 1; }
  done
  check_record "$dir/results.json" "$dir/summary.md" "$DEVIATIONS" "$(pinned_version)"
  cp "$dir/results.json" "$dir/summary.md" "$RECORD_DIR/"
  echo "conformance-record: wrote run $run_id's released views to docs/conformance/ferroehr/"
}

self_test() {
  local dir status
  dir="$(mktemp -d)"
  trap 'rm -rf "$dir"' RETURN
  jq -n '{deviations: [{case: "a/listed", clause: {source: "s", section: "x", quote: "q"},
    deviation: "d", decision: "https://github.com/FerroHEALTH/FerroEHR/issues/1",
    upstream_report: "https://github.com/FerroHEALTH/FerroEHR/issues/2"}]}' >"$dir/deviations.json"
  printf '| **Instrument** | veredictum 9.9.9, pre-approval |\n' >"$dir/summary.md"
  doc() { # <file> <profile> <engine> <listed status> <other status>
    jq -n --arg p "$2" --arg e "$3" --arg a "$4" --arg b "$5" \
      '{engine: {version: $e}, specification: {profile: $p},
        rows: [{case: "a/listed", row: 0, status: $a}, {case: "b/other", row: 0, status: $b}]}' >"$1"
  }
  expect() { # <name> <want exit> <command...>
    local name="$1" want="$2"
    shift 2
    status=0
    "$@" >"$dir/out" 2>&1 || status=$?
    if [[ "$status" != "$want" ]]; then
      echo "conformance-record: self-test '$name' exited $status, want $want:" >&2
      cat "$dir/out" >&2
      exit 1
    fi
  }
  doc "$dir/good.json" released 9.9.9 failed passed
  expect "a current record" 0 check_record "$dir/good.json" "$dir/summary.md" "$dir/deviations.json" 9.9.9
  doc "$dir/stale.json" released 9.9.9 passed passed
  expect "a record older than the deviations list" 1 check_record "$dir/stale.json" "$dir/summary.md" "$dir/deviations.json" 9.9.9
  expect "a record of another pin" 1 check_record "$dir/good.json" "$dir/summary.md" "$dir/deviations.json" 9.9.10
  doc "$dir/dev.json" development 9.9.9 failed passed
  expect "a development-profile record" 1 check_record "$dir/dev.json" "$dir/summary.md" "$dir/deviations.json" 9.9.9
  expect "a run equal to the record" 0 compare_runs "$dir/good.json" "$dir/good.json"
  doc "$dir/moved.json" released 9.9.9 failed errored
  expect "a run that moved a row" 1 compare_runs "$dir/moved.json" "$dir/good.json"
  jq '.rows += [{case: "c/new", row: 0, status: "passed"}]' "$dir/good.json" >"$dir/more.json"
  expect "a run with a row the record lacks" 1 compare_runs "$dir/more.json" "$dir/good.json"
  doc "$dir/newpin.json" released 9.9.10 failed passed
  expect "a run of another pin" 1 compare_runs "$dir/newpin.json" "$dir/good.json"
  expect "a development run" 1 compare_runs "$dir/dev.json" "$dir/good.json"
  echo "conformance-record: self-test OK (a stale, another-pin or development record fails check; a moved row, a new row, another pin or a development run fails compare)."
}

case "${1:-}" in
  check)
    [[ "$#" -eq 1 ]] || usage
    check_record "$RECORD_DIR/results.json" "$RECORD_DIR/summary.md" "$DEVIATIONS" "$(pinned_version)"
    echo "conformance-record: the committed record is a released run of the pinned Veredictum, red exactly on the listed deviations"
    ;;
  compare)
    [[ "$#" -eq 2 ]] || usage
    compare_runs "$2" "$RECORD_DIR/results.json"
    ;;
  refresh)
    [[ "$#" -eq 2 ]] || usage
    refresh "$2"
    ;;
  --self-test) self_test ;;
  *) usage ;;
esac
