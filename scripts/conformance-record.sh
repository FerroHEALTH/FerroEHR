#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The committed conformance record (docs/conformance/ferroehr/results.json and
# summary.md, and the development run's beside them under development/)
# against the runs it stands for (#6747, #6769).
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
#             list fails. The development record is a development-profile run
#             of the pin in which every listed deviation is red; it may hold
#             other red rows, which the development text asks and the
#             README badge names beside the deviations.
#   compare   in conformance.yml, after each run: every row of the run has
#             the status the record of its profile gives it, so a change that
#             moves a row commits the record of its own run in the same pull
#             request.
#   refresh   writes the views of both profiles of a run into the records.
#
# The rows carry generated inputs (fresh ids each run), so compare reads only
# each row's case, row number and status. No openEHR spec governs this guard:
# our own design.
#
# Usage:
#   scripts/conformance-record.sh check
#   scripts/conformance-record.sh compare <results.json>
#   scripts/conformance-record.sh refresh <run id>
#       the run's conformance-views-released and -development artifacts, read with gh
#   scripts/conformance-record.sh --self-test
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly ROOT
RECORD_DIR="$ROOT/docs/conformance/ferroehr"
readonly RECORD_DIR
DEV_DIR="$RECORD_DIR/development"
readonly DEV_DIR
DEVIATIONS="$ROOT/docs/conformance/party/ferroehr/deviations.json"
readonly DEVIATIONS

usage() {
  sed -n '32,37p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
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

# check_dev_record <results.json> <summary.md> <deviations.json> <pinned version>
check_dev_record() {
  local results="$1" summary="$2" deviations="$3" pinned="$4" profile engine missing
  local bad=0
  for f in "$results" "$summary"; do
    [[ -f "$f" ]] || { echo "::error::$f is missing; commit the development-profile views of a conformance run"; return 1; }
  done
  profile="$(jq -r '.specification.profile // empty' "$results")"
  engine="$(jq -r '.engine.version // empty' "$results")"
  if [[ "$profile" != development ]]; then
    echo "::error::the development record is a '${profile:-unnamed}' profile run; it holds the development profile"
    bad=1
  fi
  if [[ -z "$pinned" || "$engine" != "$pinned" ]]; then
    echo "::error::the development record was judged by Veredictum ${engine:-unnamed} while the pin is ${pinned:-unreadable}; refresh it from a run of the pin"
    bad=1
  fi
  if ! grep -q "veredictum $engine" "$summary"; then
    echo "::error::$summary does not name the instrument the record names (veredictum $engine); commit both views of one run"
    bad=1
  fi
  missing="$(jq -rn --slurpfile run "$results" --slurpfile list "$deviations" '
    ([$run[0].rows[] | select(.status == "failed" or .status == "errored" or .status == "inconclusive") | .case] | unique) as $red
    | $list[0].deviations[].case | select(. as $c | $red | index($c) | not)
  ')"
  if [[ -n "$missing" ]]; then
    echo "::error::the development record does not hold these listed deviations red; refresh it from a run of this tree: $(paste -sd ' ' <<<"$missing")"
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
  local run_engine record_engine run_profile record_profile
  run_engine="$(jq -r '.engine.version // empty' "$run")"
  record_engine="$(jq -r '.engine.version // empty' "$record")"
  run_profile="$(jq -r '.specification.profile // empty' "$run")"
  record_profile="$(jq -r '.specification.profile // empty' "$record")"
  if [[ "$run_profile" != "$record_profile" ]]; then
    echo "::error::the run is a '${run_profile:-unnamed}' profile run and the record a '${record_profile:-unnamed}' one; a run compares with the record of its profile"
    return 1
  fi
  if [[ "$run_engine" != "$record_engine" ]]; then
    diff="${diff:+$diff
}the run was judged by Veredictum $run_engine, the record by $record_engine"
  fi
  if [[ -n "$diff" ]]; then
    echo "::error::the committed $run_profile record differs from this run in $(grep -c . <<<"$diff") place(s); commit this run's views with 'scripts/conformance-record.sh refresh <run id>'"
    head -n 40 <<<"$diff"
    return 1
  fi
  echo "conformance-record: the $run_profile run gives every row the status the committed record gives it ($(jq '.rows | length' "$run") rows)"
}

refresh() {
  local run_id="$1" dir
  [[ "$run_id" =~ ^[0-9]+$ ]] || { echo "conformance-record: '$run_id' is not a run id" >&2; exit 2; }
  dir="$(mktemp -d)"
  trap 'rm -rf "$dir"' RETURN
  gh run download "$run_id" --repo FerroHEALTH/FerroEHR --name conformance-views-released --dir "$dir/released"
  gh run download "$run_id" --repo FerroHEALTH/FerroEHR --name conformance-views-development --dir "$dir/development"
  for f in released/results.json released/summary.md development/results.json development/summary.md; do
    [[ -f "$dir/$f" ]] || { echo "conformance-record: run $run_id's artifacts hold no $f" >&2; exit 1; }
  done
  check_record "$dir/released/results.json" "$dir/released/summary.md" "$DEVIATIONS" "$(pinned_version)"
  check_dev_record "$dir/development/results.json" "$dir/development/summary.md" "$DEVIATIONS" "$(pinned_version)"
  cp "$dir/released/results.json" "$dir/released/summary.md" "$RECORD_DIR/"
  cp "$dir/development/results.json" "$dir/development/summary.md" "$DEV_DIR/"
  echo "conformance-record: wrote run $run_id's views to docs/conformance/ferroehr/ and its development/"
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
    local file="$1" profile="$2" engine="$3" listed="$4" other="$5"
    jq -n --arg p "$profile" --arg e "$engine" --arg a "$listed" --arg b "$other" \
      '{engine: {version: $e}, specification: {profile: $p},
        rows: [{case: "a/listed", row: 0, status: $a}, {case: "b/other", row: 0, status: $b}]}' >"$file"
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
  expect "a development run against the released record" 1 compare_runs "$dir/dev.json" "$dir/good.json"
  expect "a current development record" 0 check_dev_record "$dir/dev.json" "$dir/summary.md" "$dir/deviations.json" 9.9.9
  doc "$dir/devmore.json" development 9.9.9 failed failed
  expect "a development record with a red row beside the deviations" 0 check_dev_record "$dir/devmore.json" "$dir/summary.md" "$dir/deviations.json" 9.9.9
  doc "$dir/devstale.json" development 9.9.9 passed failed
  expect "a development record that does not hold a deviation red" 1 check_dev_record "$dir/devstale.json" "$dir/summary.md" "$dir/deviations.json" 9.9.9
  expect "a released record as the development one" 1 check_dev_record "$dir/good.json" "$dir/summary.md" "$dir/deviations.json" 9.9.9
  expect "a development run equal to its record" 0 compare_runs "$dir/dev.json" "$dir/dev.json"
  echo "conformance-record: self-test OK (a stale, another-pin or development record fails the released check; a development record not red on a deviation fails its check; a moved row, a new row, another pin or another profile fails compare)."
}

case "${1:-}" in
  check)
    [[ "$#" -eq 1 ]] || usage
    status=0
    check_record "$RECORD_DIR/results.json" "$RECORD_DIR/summary.md" "$DEVIATIONS" "$(pinned_version)" || status=1
    check_dev_record "$DEV_DIR/results.json" "$DEV_DIR/summary.md" "$DEVIATIONS" "$(pinned_version)" || status=1
    [[ "$status" -eq 0 ]] || exit 1
    echo "conformance-record: the committed records are runs of the pinned Veredictum, the released one red exactly on the listed deviations and the development one red on each of them"
    ;;
  compare)
    [[ "$#" -eq 2 ]] || usage
    case "$(jq -r '.specification.profile // empty' "$2")" in
      development) compare_runs "$2" "$DEV_DIR/results.json" ;;
      *) compare_runs "$2" "$RECORD_DIR/results.json" ;;
    esac
    ;;
  refresh)
    [[ "$#" -eq 2 ]] || usage
    refresh "$2"
    ;;
  --self-test) self_test ;;
  *) usage ;;
esac
