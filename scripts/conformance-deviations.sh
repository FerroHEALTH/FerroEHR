#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The verdict of a released conformance run over the deviations FerroEHR
# carries on purpose (docs/conformance/party/ferroehr/deviations.json).
#
# Veredictum records a deviation from a MUST as a failed row and offers no
# way to declare one, so each listed row stays red in its results. This
# judges those results: it passes when every red row (failed, errored or
# inconclusive) is listed, and fails on a red row that is not, on a listed
# row that passes or is skipped, and on a listed case the run does not hold.
# The list therefore only shrinks. No openEHR spec governs this gate: our own
# design; each entry names the clause it breaks, the decision and the
# upstream report.
#
# Usage:
#   scripts/conformance-deviations.sh <results.json> [<deviations.json>] [<summary.md>]
#       exit 0 when the run holds no red row outside the list and every
#       listed row is red, 1 otherwise; with <summary.md>, the deviations
#       are appended to it as a Markdown table
#   scripts/conformance-deviations.sh --self-test
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly ROOT

usage() {
  sed -n '16,21p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

# check_list <deviations.json>: every entry names one case and its grounds.
check_list() {
  jq -e '
    (.deviations | type == "array" and length > 0)
    and ([.deviations[].case] | length == (unique | length))
    and all(.deviations[];
      (.case | type == "string" and test("^[a-z0-9_]+/[A-Za-z0-9_.-]+$"))
      and (.clause.source | type == "string" and length > 0)
      and (.clause.section | type == "string" and length > 0)
      and (.clause.quote | type == "string" and length > 0)
      and (.deviation | type == "string" and length > 0)
      and (.decision | type == "string" and test("^https://github.com/FerroHEALTH/FerroEHR/issues/[0-9]+$"))
      and (.upstream_report | type == "string" and test("^https://github.com/FerroHEALTH/FerroEHR/issues/[0-9]+$")))
  ' "$1" >/dev/null
}

# judge <results.json> <deviations.json> [<summary.md>]
judge() {
  local results="$1" deviations="$2" summary="${3:-}" problems
  if ! check_list "$deviations"; then
    echo "::error::$deviations is malformed: each entry needs one case id (no wildcard), the clause's source, section and quote, the deviation, and the decision and upstream report issue URLs, with no case listed twice"
    return 1
  fi
  problems="$(jq -r --slurpfile list "$deviations" '
    ($list[0].deviations | map(.case)) as $listed
    | (reduce .rows[] as $row ({}; .[$row.case] += [$row.status])) as $by_case
    | ([.rows[] | select(.status == "failed" or .status == "errored" or .status == "inconclusive")
        | select(.case as $c | $listed | index($c) | not)
        | "a red row no deviation lists: \(.case) (\(.status))"] | unique[]),
      ($listed[] | select($by_case[.] == null)
        | "a listed deviation the run does not hold: \(.)"),
      ($listed[] | select($by_case[.] != null)
        | select($by_case[.] | all(. == "passed" or . == "skipped"))
        | "a listed deviation the run no longer fails; remove it from the list: \(.)")
  ' "$results")"
  if [[ -n "$summary" ]]; then
    {
      printf '### Known deviations\n\n'
      printf "Each row stays red in the Veredictum results; this gate records it with its ground (%s).\n\n" "\`docs/conformance/party/ferroehr/deviations.json\`"
      printf '| Case | Status | Clause | Decision | Upstream report |\n|---|---|---|---|---|\n'
      jq -r --slurpfile results "$results" '
        .deviations[] | . as $d
        | ([$results[0].rows[] | select(.case == $d.case) | .status] | unique | join(", ")) as $status
        | "| `\($d.case)` | \(if $status == "" then "not run" else $status end) | \($d.clause.source) §\($d.clause.section) | \($d.decision) | \($d.upstream_report) |"
      ' "$deviations"
      printf '\n'
    } >>"$summary"
  fi
  if [[ -n "$problems" ]]; then
    while IFS= read -r problem; do echo "::error::$problem"; done <<<"$problems"
    return 1
  fi
  echo "conformance-deviations: every red row is a listed deviation, and every listed deviation is red ($(jq '.deviations | length' "$deviations") listed)"
}

self_test() {
  local dir list status
  dir="$(mktemp -d)"
  trap 'rm -rf "$dir"' RETURN
  list="$dir/deviations.json"
  jq -n '{deviations: [{case: "a/listed", clause: {source: "s", section: "x", quote: "q"},
    deviation: "d", decision: "https://github.com/FerroHEALTH/FerroEHR/issues/1",
    upstream_report: "https://github.com/FerroHEALTH/FerroEHR/issues/2"}]}' >"$list"
  expect() { # <name> <want exit> <rows json>
    jq -n --argjson rows "$3" '{rows: $rows}' >"$dir/results.json"
    status=0
    judge "$dir/results.json" "$list" >"$dir/out" 2>&1 || status=$?
    if [[ "$status" != "$2" ]]; then
      echo "conformance-deviations: self-test '$1' exited $status, want $2:" >&2
      cat "$dir/out" >&2
      exit 1
    fi
  }
  expect "only the listed row red" 0 \
    '[{"case":"a/listed","status":"failed"},{"case":"b/other","status":"passed"}]'
  expect "an unlisted red row" 1 \
    '[{"case":"a/listed","status":"failed"},{"case":"b/other","status":"errored"}]'
  expect "a listed row that passes" 1 \
    '[{"case":"a/listed","status":"passed"},{"case":"b/other","status":"passed"}]'
  expect "a listed id the run does not hold" 1 \
    '[{"case":"b/other","status":"passed"}]'
  jq '.deviations[0].case = "a/*"' "$list" >"$dir/wild.json" && mv "$dir/wild.json" "$list"
  expect "a wildcard entry" 1 '[{"case":"a/listed","status":"failed"}]'
  check_list "$ROOT/docs/conformance/party/ferroehr/deviations.json" || {
    echo "conformance-deviations: the committed list is malformed" >&2
    exit 1
  }
  echo "conformance-deviations: self-test OK (an unlisted red row, a listed row that passes, an unknown id and a wildcard each fail; the committed list is well formed)."
}

case "${1:-}" in
  --self-test) self_test ;;
  "" | -h | --help) usage ;;
  *) judge "$1" "${2:-$ROOT/docs/conformance/party/ferroehr/deviations.json}" "${3:-}" ;;
esac
