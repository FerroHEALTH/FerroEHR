#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The perf gate's verdict with the owner's accepted costs (#5924): a breach of
# `scripts/perf/gate.sh ab` passes only when the pull request carries the
# `perf-accepted` label and its description names, for every breached metric,
# an issue on which the owner accepted the cost. The breach is then reported
# as accepted, the full report beside it; it never passes silently. A run that
# could not be judged is never accepted. No openEHR spec governs this: our own
# design.
#
# A pull request names an accepted cost on a line of its own:
#
#   Perf-accepted: #5891 transactions_per_request
#
# with the metrics as the report's Breaches table names them, separated by
# spaces or commas; a WAL breach is `wal.flushes_per_request` or
# `wal.bytes_per_request`. The issue carries the acceptance when a comment on
# it by the repository's code owner (the `*` line of `.github/CODEOWNERS` on
# the base branch, read through the API, so the pull request cannot name its
# own owner) says "accepted".
#
# Usage:
#   scripts/perf/accepted.sh judge <gate-status> <summary.md>
#       GH_REPO, GH_TOKEN, BASE_REF (the base branch), and PR (the pull
#       request) or HEAD_SHA (a push: the pull request that merged it)
#   scripts/perf/accepted.sh --self-test
#
# Exit codes: 0 the gate passed or every breach is accepted, 1 a breach not
# accepted, 2 the run could not be judged or a usage or API error.
set -euo pipefail
shopt -s inherit_errexit

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
readonly ROOT
readonly LABEL=perf-accepted

usage() {
  sed -n '/^# Usage:$/,/^#$/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

# breaches <summary.md>: the breached metrics, one per line: every FAIL row of
# the Breaches table, and each WAL breach line of the gate's WAL judge.
breaches() {
  awk '
    /^### / { table = ($0 == "### Breaches") }
    table && /^\| / && $NF == "|" {
      n = split($0, cell, "|")
      metric = cell[2]; result = cell[n - 1]
      gsub(/^ +| +$/, "", metric); gsub(/^ +| +$/, "", result)
      if (result == "FAIL") print metric
    }
    /^- WAL flushes per request:/ { print "wal.flushes_per_request" }
    /^- WAL bytes per request:/ { print "wal.bytes_per_request" }
  ' "$1" | sort -u
}

# claims <body>: "<issue> <metric>" per metric a Perf-accepted line names.
claims() {
  tr -d '\r' <<<"$1" | awk '
    /^Perf-accepted: #[0-9]+ [^ ]/ {
      issue = $2; sub(/^#/, "", issue)
      line = $0; sub(/^Perf-accepted: #[0-9]+ +/, "", line)
      n = split(line, metric, /[ ,]+/)
      for (i = 1; i <= n; i++) if (metric[i] != "") print issue, metric[i]
    }
  ' | sort -u
}

# decide <gate-status> <labelled: true|false> <summary.md> <body> <owned>:
# the verdict. <owned> is a file of "<issue> <comment-url>" for each claimed
# issue on which the code owner accepted. Prints the accepted section on
# stdout and the reasons on stderr.
decide() {
  local status="$1" labelled="$2" summary="$3" body="$4" owned="$5"
  local metric issue url found
  local -a broken=() unaccepted=() accepted=()
  case "$status" in
    0) echo "perf gate: the head passed against the base" >&2; return 0 ;;
    1) ;;
    *) echo "perf gate: the run could not be judged (exit $status), which no acceptance covers" >&2; return "$status" ;;
  esac
  mapfile -t broken < <(breaches "$summary")
  if [[ "${#broken[@]}" == 0 ]]; then
    echo "::error::perf gate: the gate breached but its report names no breach, so nothing can be accepted" >&2
    return 1
  fi
  if [[ "$labelled" != true ]]; then
    echo "::error::perf gate: breached: ${broken[*]} (no $LABEL label)" >&2
    return 1
  fi
  for metric in "${broken[@]}"; do
    found=""
    while read -r issue name; do
      [[ "$name" == "$metric" ]] || continue
      url=$(awk -v i="$issue" '$1 == i { print $2; exit }' "$owned")
      if [[ -n "$url" ]]; then
        found="$issue $url"
        break
      fi
    done < <(claims "$body")
    if [[ -n "$found" ]]; then
      accepted+=("$metric $found")
    else
      unaccepted+=("$metric")
    fi
  done
  if [[ "${#unaccepted[@]}" != 0 ]]; then
    echo "::error::perf gate: breached and not accepted: ${unaccepted[*]} (each needs a Perf-accepted line naming an issue on which the code owner accepted it)" >&2
    return 1
  fi
  echo
  echo "### Accepted breaches"
  echo
  echo "The head breached the base, and the owner accepted each cost (\`$LABEL\`, docs/release.md). The breaches stay in the report above."
  echo
  echo "| Metric | Issue | Acceptance |"
  echo "| --- | --- | --- |"
  for entry in "${accepted[@]}"; do
    read -r metric issue url <<<"$entry"
    echo "| $metric | #$issue | $url |"
    echo "::warning::perf gate: accepted breach of $metric (#$issue, $url)" >&2
  done
  return 0
}

# owner <base-ref>: the login on the `*` line of the base branch's CODEOWNERS.
owner() {
  gh_retry gh api "repos/${GH_REPO}/contents/.github/CODEOWNERS?ref=$1" --jq .content \
    | base64 -d | awk '$1 == "*" { sub(/^@/, "", $2); print $2; exit }'
}

judge() {
  [[ "$#" -eq 2 ]] || usage
  local status="$1" summary="$2" pull labels body login issue url owned verdict=0
  : "${GH_REPO:?GH_REPO names the repository}" "${BASE_REF:?BASE_REF names the base branch}"
  # shellcheck source=scripts/lib/gh-retry.sh
  source "$ROOT/scripts/lib/gh-retry.sh"
  [[ -f "$summary" ]] && cat "$summary"
  if [[ "$status" != 1 ]]; then
    decide "$status" false "$summary" "" /dev/null
    return
  fi
  pull="${PR:-}"
  if [[ -z "$pull" ]]; then
    : "${HEAD_SHA:?HEAD_SHA names the pushed commit when PR is empty}"
    pull=$(gh_retry gh api "repos/${GH_REPO}/commits/${HEAD_SHA}/pulls" \
      --jq '[.[] | select(.merged_at != null)][0].number // empty') || return 2
  fi
  labels=false
  body=""
  if [[ -n "$pull" ]]; then
    labels=$(gh_retry gh api "repos/${GH_REPO}/pulls/${pull}" \
      --jq "any(.labels[]; .name == \"$LABEL\")") || return 2
    body=$(gh_retry gh api "repos/${GH_REPO}/pulls/${pull}" --jq '.body // ""') || return 2
  fi
  owned=$(mktemp)
  if [[ "$labels" == true ]]; then
    login=$(owner "$BASE_REF") || return 2
    [[ -n "$login" ]] || { echo "perf gate: the base branch's CODEOWNERS names no owner of *" >&2; return 2; }
    while read -r issue; do
      url=$(gh_retry gh api --paginate "repos/${GH_REPO}/issues/${issue}/comments" \
        --jq ".[] | select(.user.login == \"$login\" and (.body | test(\"accepted\"; \"i\"))) | .html_url" \
        | sed -n 1p) || return 2
      [[ -n "$url" ]] && echo "$issue $url" >>"$owned"
    done < <(claims "$body" | awk '{ print $1 }' | sort -u)
  fi
  decide "$status" "$labels" "$summary" "$body" "$owned" >"$owned.section" || verdict=$?
  if [[ -s "$owned.section" ]]; then
    cat "$owned.section" >>"$summary"
    cat "$owned.section"
    [[ -n "${GITHUB_STEP_SUMMARY:-}" ]] && cat "$owned.section" >>"$GITHUB_STEP_SUMMARY"
  fi
  rm -f "$owned" "$owned.section"
  return "$verdict"
}

self_test() {
  local dir fail=0
  dir=$(mktemp -d)
  cat >"$dir/breach.md" <<'EOF'
## ferrohealth-perf: the A/B run failed

### Breaches

| Metric | Base | Head | Ratio | Tolerance | Floor | Threshold | Breached | Result |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| transactions_per_request | 4.380 | 4.961 | 1.13 | 10 % | 1.02 | 4.818 | 4 of 4 pairs | FAIL |

### The head over the base

| Metric | Base | Head | Ratio | Tolerance | Floor | Threshold | Breached | Result |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| throughput_rps | 20.000 | 20.000 | 1.00 | 20 % | 1.00 | 16.000 | 0 of 4 pairs | pass |
| transactions_per_request | 4.380 | 4.961 | 1.13 | 10 % | 1.02 | 4.818 | 4 of 4 pairs | FAIL |
EOF
  cp "$dir/breach.md" "$dir/wal.md"
  printf '\n### Write-ahead log\n\n- WAL flushes per request: the head 1.20 against the base 1.00, over the 0.10 more the gate allows\n' >>"$dir/wal.md"
  printf '## ferrohealth-perf: the A/B run failed\n\nA phase stalled.\n' >"$dir/empty.md"
  printf '5891 https://github.com/o/r/issues/5891#issuecomment-1\n' >"$dir/owned"
  : >"$dir/none"
  local claim=$'Fixes the copy.\r\n\r\nPerf-accepted: #5891 transactions_per_request\r\n'
  local both=$'Perf-accepted: #5891 transactions_per_request, wal.flushes_per_request'
  local other=$'Perf-accepted: #5891 cpu_per_request_ms'
  local elsewhere=$'Perf-accepted: #42 transactions_per_request'
  case_() { # <label> <want-exit> <want-in-output> <decide args...>
    local label="$1" want="$2" needle="$3" got=0 out
    shift 3
    out=$(decide "$@" 2>&1) || got=$?
    if [[ "$got" != "$want" || "$out" != *"$needle"* ]]; then
      echo "::error::self-test: ${label}: exit ${got}, want ${want}, output: ${out}" >&2
      fail=1
    fi
  }
  case_ "a pass passes" 0 "passed" 0 false "$dir/breach.md" "" "$dir/none"
  case_ "an unjudged run is never accepted" 2 "could not be judged" 2 true "$dir/breach.md" "$claim" "$dir/owned"
  case_ "a breach without the label fails" 1 "no $LABEL label" 1 false "$dir/breach.md" "$claim" "$dir/owned"
  case_ "a breach the owner accepted passes, reported" 0 "| transactions_per_request | #5891 |" 1 true "$dir/breach.md" "$claim" "$dir/owned"
  case_ "an accepted breach warns" 0 "::warning::perf gate: accepted breach of transactions_per_request" 1 true "$dir/breach.md" "$claim" "$dir/owned"
  case_ "an issue without the owner's acceptance fails" 1 "not accepted: transactions_per_request" 1 true "$dir/breach.md" "$claim" "$dir/none"
  case_ "an acceptance of another metric fails" 1 "not accepted: transactions_per_request" 1 true "$dir/breach.md" "$other" "$dir/owned"
  case_ "an acceptance on an unverified issue fails" 1 "not accepted" 1 true "$dir/breach.md" "$elsewhere" "$dir/owned"
  case_ "a WAL breach not named fails" 1 "not accepted: wal.flushes_per_request" 1 true "$dir/wal.md" "$claim" "$dir/owned"
  case_ "every breach named passes" 0 "| wal.flushes_per_request | #5891 |" 1 true "$dir/wal.md" "$both" "$dir/owned"
  case_ "a breach the report does not name fails" 1 "names no breach" 1 true "$dir/empty.md" "$claim" "$dir/owned"
  case_ "a label without a claim fails" 1 "not accepted" 1 true "$dir/breach.md" "No claim here." "$dir/owned"
  rm -rf "$dir"
  [[ "$fail" == 0 ]] || { echo "accepted: self-test FAILED" >&2; exit 1; }
  echo "accepted: self-test OK (a breach passes only with the label, a Perf-accepted line per metric and the owner's acceptance on its issue)"
}

[[ "$#" -ge 1 ]] || usage
case "$1" in
judge) shift; judge "$@" ;;
--self-test) self_test ;;
*) usage ;;
esac
