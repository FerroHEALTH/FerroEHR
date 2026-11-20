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
# The release's performance job (#5927) honours the same acceptances: every
# `Perf-accepted:` line of a pull request merged into the release's milestone
# that carries the label, each verified against the code owner's comment on
# its issue exactly as the pull request's verdict verifies it. A breach no
# such acceptance covers fails the release, and each honoured acceptance is
# listed with its pull request.
#
# Usage:
#   scripts/perf/accepted.sh judge <gate-status> <summary.md>
#       GH_REPO, GH_TOKEN, BASE_REF (the base branch), and PR (the pull
#       request) or HEAD_SHA (a push: the pull request that merged it)
#   scripts/perf/accepted.sh release <gate-status> <summary.md>
#       GH_REPO, GH_TOKEN, BASE_REF (the default branch), and MILESTONE (the
#       milestone titled as the release's tag)
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

# cover <summary.md> <claims> <owned> <scope>: whether every breached metric
# is accepted. <claims> is a file of "<issue> <metric> <pull>" per claimed
# metric, <pull> "-" for the pull request's own description; <owned> a file of
# "<issue> <comment-url>" per claimed issue on which the code owner accepted.
# <scope> is `pr` or `release`. Prints the accepted section on stdout and the
# reasons on stderr.
cover() {
  local summary="$1" claims="$2" owned="$3" scope="$4"
  local metric issue name pull url entry
  local -a broken=() unaccepted=() accepted=() found=()
  mapfile -t broken < <(breaches "$summary")
  if [[ "${#broken[@]}" == 0 ]]; then
    echo "::error::perf gate: the gate breached but its report names no breach, so nothing can be accepted" >&2
    return 1
  fi
  for metric in "${broken[@]}"; do
    found=()
    while read -r issue name pull; do
      [[ "$name" == "$metric" ]] || continue
      url=$(awk -v i="$issue" '$1 == i { print $2; exit }' "$owned")
      [[ -n "$url" ]] && found+=("$metric $issue $url $pull")
    done <"$claims"
    if [[ "${#found[@]}" != 0 ]]; then
      accepted+=("${found[@]}")
    else
      unaccepted+=("$metric")
    fi
  done
  if [[ "${#unaccepted[@]}" != 0 ]]; then
    if [[ "$scope" == release ]]; then
      echo "::error::perf gate: breached and not accepted: ${unaccepted[*]} (no pull request merged into the milestone carries the $LABEL label and a Perf-accepted line naming an issue on which the code owner accepted it)" >&2
    else
      echo "::error::perf gate: breached and not accepted: ${unaccepted[*]} (each needs a Perf-accepted line naming an issue on which the code owner accepted it)" >&2
    fi
    return 1
  fi
  echo
  echo "### Accepted breaches"
  echo
  if [[ "$scope" == release ]]; then
    echo "The release breached the previous release, and the owner accepted each cost on a pull request merged into the release's milestone (\`$LABEL\`, docs/release.md). The breaches stay in the report above."
    echo
    echo "| Metric | Pull request | Issue | Acceptance |"
    echo "| --- | --- | --- | --- |"
  else
    echo "The head breached the base, and the owner accepted each cost (\`$LABEL\`, docs/release.md). The breaches stay in the report above."
    echo
    echo "| Metric | Issue | Acceptance |"
    echo "| --- | --- | --- |"
  fi
  for entry in "${accepted[@]}"; do
    read -r metric issue url pull <<<"$entry"
    if [[ "$scope" == release ]]; then
      echo "| $metric | #$pull | #$issue | $url |"
      echo "::warning::perf gate: accepted breach of $metric (#$pull, #$issue, $url)" >&2
    else
      echo "| $metric | #$issue | $url |"
      echo "::warning::perf gate: accepted breach of $metric (#$issue, $url)" >&2
    fi
  done
  return 0
}

# gate_status <gate-status>: 0 for a pass, 1 for a breach to judge, and the
# status itself for a run that could not be judged.
gate_status() {
  case "$1" in
    0) echo "perf gate: the head passed against the base" >&2; return 0 ;;
    1) return 1 ;;
    *) echo "perf gate: the run could not be judged (exit $1), which no acceptance covers" >&2; return "$1" ;;
  esac
}

# decide <gate-status> <labelled: true|false> <summary.md> <body> <owned>:
# the pull request's verdict over its own description.
decide() {
  local status="$1" labelled="$2" summary="$3" body="$4" owned="$5" claimed verdict=0
  local -a broken=()
  gate_status "$status" || verdict=$?
  [[ "$verdict" == 1 ]] || return "$verdict"
  if [[ "$labelled" != true ]]; then
    mapfile -t broken < <(breaches "$summary")
    if [[ "${#broken[@]}" == 0 ]]; then
      echo "::error::perf gate: the gate breached but its report names no breach, so nothing can be accepted" >&2
    else
      echo "::error::perf gate: breached: ${broken[*]} (no $LABEL label)" >&2
    fi
    return 1
  fi
  claimed=$(mktemp)
  claims "$body" | awk '{ print $0, "-" }' >"$claimed"
  verdict=0
  cover "$summary" "$claimed" "$owned" pr || verdict=$?
  rm -f "$claimed"
  return "$verdict"
}

# decide_release <gate-status> <summary.md> <claims> <owned>: the release's
# verdict over the claims of the milestone's labelled pull requests.
decide_release() {
  local status="$1" summary="$2" claimed="$3" owned="$4" verdict=0
  gate_status "$status" || verdict=$?
  [[ "$verdict" == 1 ]] || return "$verdict"
  cover "$summary" "$claimed" "$owned" release
}

# owner_of <codeowners>: the login on the first `*` line of a CODEOWNERS text.
# awk reads its whole input, so no writer before it meets a closed pipe.
owner_of() {
  awk '$1 == "*" && !found { sub(/^@/, "", $2); print $2; found = 1 }' <<<"$1"
}

# owner <base-ref>: the login on the `*` line of the base branch's CODEOWNERS.
owner() {
  local encoded text
  encoded=$(gh_retry gh api "repos/${GH_REPO}/contents/.github/CODEOWNERS?ref=$1" --jq .content) || return 2
  text=$(base64 -d <<<"$encoded") || return 2
  owner_of "$text"
}

# verify <base-ref> <owned>: for each issue number on stdin, appends
# "<issue> <comment-url>" to <owned> when a comment on it by the base branch's
# code owner says "accepted". The pull request's verdict and the release's
# share it, so the release verifies an acceptance exactly as the pull request
# did.
verify() {
  local ref="$1" owned="$2" login issue url
  login=$(owner "$ref") || return 2
  [[ -n "$login" ]] || { echo "perf gate: the base branch's CODEOWNERS names no owner of *" >&2; return 2; }
  while read -r issue; do
    url=$(gh_retry gh api --paginate "repos/${GH_REPO}/issues/${issue}/comments" \
      --jq ".[] | select(.user.login == \"$login\" and (.body | test(\"accepted\"; \"i\"))) | .html_url" \
      | sed -n 1p) || return 2
    [[ -n "$url" ]] && echo "$issue $url" >>"$owned"
  done
  return 0
}

judge() {
  [[ "$#" -eq 2 ]] || usage
  local status="$1" summary="$2" pull labels body owned verdict=0
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
    verify "$BASE_REF" "$owned" < <(claims "$body" | awk '{ print $1 }' | sort -u) || return 2
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

# release <gate-status> <summary.md>: the release's verdict. The claims are
# the Perf-accepted lines of every pull request merged into the milestone
# titled MILESTONE that carries the label; a closed, unmerged one claims
# nothing. The accepted section goes into <summary.md>, which the job's
# summary step prints.
release() {
  [[ "$#" -eq 2 ]] || usage
  local status="$1" summary="$2" milestone pull body claimed owned verdict=0
  : "${GH_REPO:?GH_REPO names the repository}" "${BASE_REF:?BASE_REF names the default branch}"
  : "${MILESTONE:?MILESTONE names the milestone titled as the tag}"
  # shellcheck source=scripts/lib/gh-retry.sh
  source "$ROOT/scripts/lib/gh-retry.sh"
  [[ -f "$summary" ]] && cat "$summary"
  if [[ "$status" != 1 ]]; then
    decide_release "$status" "$summary" /dev/null /dev/null
    return
  fi
  milestone=$(gh_retry gh api --paginate "repos/${GH_REPO}/milestones?state=all&per_page=100" \
    --jq ".[] | select(.title == \"$MILESTONE\") | .number" | sed -n 1p) || return 2
  [[ -n "$milestone" ]] || { echo "perf gate: no milestone is titled '$MILESTONE'" >&2; return 2; }
  claimed=$(mktemp)
  owned=$(mktemp)
  while read -r pull; do
    body=$(gh_retry gh api "repos/${GH_REPO}/pulls/${pull}" --jq '.body // ""') || return 2
    claims "$body" | awk -v pull="$pull" '{ print $0, pull }' >>"$claimed"
  done < <(gh_retry gh api --paginate "repos/${GH_REPO}/issues?milestone=${milestone}&state=closed&per_page=100" \
    --jq ".[] | select(.pull_request.merged_at != null and any(.labels[]; .name == \"$LABEL\")) | .number" | sort -n)
  if [[ -s "$claimed" ]]; then
    verify "$BASE_REF" "$owned" < <(awk '{ print $1 }' "$claimed" | sort -u) || return 2
  fi
  decide_release "$status" "$summary" "$claimed" "$owned" >"$owned.section" || verdict=$?
  if [[ -s "$owned.section" ]]; then
    cat "$owned.section" >>"$summary"
    cat "$owned.section"
  fi
  rm -f "$claimed" "$owned" "$owned.section"
  return "$verdict"
}

# release_self_test <dir>: the release path end to end over a stub `gh` that
# applies the script's own --jq filters to fixture answers. A breach the owner
# accepted on a merged, labelled pull request passes; a claim whose
# "accepted" comment another login wrote fails, and so does one made by an
# unlabelled or an unmerged pull request, whose issue the owner did accept.
release_self_test() {
  local dir="$1" fixtures="$1/api" got out bad=0
  mkdir -p "$fixtures"
  printf '[{"number":9,"title":"v6.0.0"},{"number":8,"title":"v5.9.0"}]' >"$fixtures/milestones"
  printf '%s' '[{"number":5930,"labels":[{"name":"perf-accepted"}],"pull_request":{"merged_at":"2026-11-18T10:00:00Z"}},
 {"number":5931,"labels":[{"name":"perf-accepted"}],"pull_request":{"merged_at":"2026-11-18T11:00:00Z"}},
 {"number":5932,"labels":[],"pull_request":{"merged_at":"2026-11-18T12:00:00Z"}},
 {"number":5933,"labels":[{"name":"perf-accepted"}],"pull_request":{"merged_at":null}},
 {"number":5934,"labels":[{"name":"perf-accepted"}]}]' >"$fixtures/issues"
  printf '%s' '{"body":"Exact reads.\r\n\r\nPerf-accepted: #5891 transactions_per_request\r\n"}' >"$fixtures/pulls-5930"
  printf '%s' '{"body":"Perf-accepted: #42 wal.flushes_per_request"}' >"$fixtures/pulls-5931"
  printf '%s' '{"body":"Perf-accepted: #43 wal.flushes_per_request"}' >"$fixtures/pulls-5932"
  printf '%s' '{"body":"Perf-accepted: #43 wal.flushes_per_request"}' >"$fixtures/pulls-5933"
  printf '{"content":"%s"}' "$(printf '* @owner-one\n' | base64)" >"$fixtures/codeowners"
  printf '%s' '[{"user":{"login":"owner-one"},"body":"Accepted: the cost of an exact decision.","html_url":"https://github.com/o/r/issues/5891#issuecomment-1"}]' >"$fixtures/comments-5891"
  printf '%s' '[{"user":{"login":"mallory"},"body":"accepted","html_url":"https://github.com/o/r/issues/42#issuecomment-2"},{"user":{"login":"owner-one"},"body":"Not yet.","html_url":"https://github.com/o/r/issues/42#issuecomment-4"}]' >"$fixtures/comments-42"
  printf '%s' '[{"user":{"login":"owner-one"},"body":"accepted","html_url":"https://github.com/o/r/issues/43#issuecomment-3"}]' >"$fixtures/comments-43"
  # shellcheck disable=SC2329 # the stub is called through gh_retry's "$@"
  gh() {
    local path="" filter="." file
    shift # api
    while [[ "$#" -gt 0 ]]; do
      case "$1" in
        --paginate) ;;
        --jq) filter="$2"; shift ;;
        *) path="$1" ;;
      esac
      shift
    done
    case "$path" in
      repos/o/r/milestones\?*) file=milestones ;;
      repos/o/r/issues\?milestone=9\&*) file=issues ;;
      repos/o/r/pulls/*) file="pulls-${path##*/}" ;;
      repos/o/r/contents/.github/CODEOWNERS\?ref=main) file=codeowners ;;
      repos/o/r/issues/*/comments) file="comments-$(cut -d/ -f5 <<<"$path")" ;;
      *) echo "stub gh: unexpected path $path (HTTP 404)" >&2; return 1 ;;
    esac
    [[ -f "$fixtures/$file" ]] || { echo "stub gh: no fixture for $path (HTTP 404)" >&2; return 1; }
    jq -r "$filter" "$fixtures/$file"
  }
  ecase() { # <label> <want-exit> <want-in-output> <summary.md>
    local label="$1" want="$2" needle="$3"
    cp "$4" "$dir/e2e.md"
    got=0
    out=$(GH_REPO=o/r BASE_REF=main MILESTONE=v6.0.0 release 1 "$dir/e2e.md" 2>&1) || got=$?
    if [[ "$got" != "$want" || "$out" != *"$needle"* ]]; then
      echo "::error::self-test: release end to end: ${label}: exit ${got}, want ${want}, output: ${out}" >&2
      bad=1
    fi
  }
  ecase "a breach the owner accepted on a merged pull request passes" 0 "| transactions_per_request | #5930 | #5891 | https://github.com/o/r/issues/5891#issuecomment-1 |" "$dir/breach.md"
  if ! grep -q '^### Accepted breaches$' "$dir/e2e.md"; then
    echo "::error::self-test: release end to end: the accepted section is not in the summary" >&2
    bad=1
  fi
  ecase "a forged, unlabelled or unmerged claim is never honoured" 1 "not accepted: wal.flushes_per_request" "$dir/wal.md"
  if grep -q '#5931\|#5932\|#5933\|#5934' <<<"$out"; then
    echo "::error::self-test: release end to end: a forged, unlabelled or unmerged claim was honoured: ${out}" >&2
    bad=1
  fi
  unset -f gh
  return "$bad"
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
  # The release (#5927): the claims of the milestone's labelled pull requests.
  printf '5891 transactions_per_request 5930\n' >"$dir/rel-claims"
  printf '5891 transactions_per_request 5930\n42 wal.flushes_per_request 5931\n' >"$dir/rel-forged"
  rcase() { # <label> <want-exit> <want-in-output> <decide_release args...>
    local label="$1" want="$2" needle="$3" got=0 out
    shift 3
    out=$(decide_release "$@" 2>&1) || got=$?
    if [[ "$got" != "$want" || "$out" != *"$needle"* ]]; then
      echo "::error::self-test: release: ${label}: exit ${got}, want ${want}, output: ${out}" >&2
      fail=1
    fi
  }
  rcase "a pass passes" 0 "passed" 0 "$dir/breach.md" "$dir/none" "$dir/none"
  rcase "an unjudged run is never accepted" 2 "could not be judged" 2 "$dir/breach.md" "$dir/rel-claims" "$dir/owned"
  rcase "a covered breach passes, listed with its pull request" 0 "| transactions_per_request | #5930 | #5891 |" 1 "$dir/breach.md" "$dir/rel-claims" "$dir/owned"
  rcase "a covered breach warns with its pull request" 0 "accepted breach of transactions_per_request (#5930, #5891" 1 "$dir/breach.md" "$dir/rel-claims" "$dir/owned"
  rcase "an uncovered breach fails" 1 "not accepted: wal.flushes_per_request" 1 "$dir/wal.md" "$dir/rel-claims" "$dir/owned"
  rcase "a claim the owner never accepted fails" 1 "not accepted: wal.flushes_per_request" 1 "$dir/wal.md" "$dir/rel-forged" "$dir/owned"
  rcase "a release with no claims fails" 1 "no pull request merged into the milestone" 1 "$dir/breach.md" "$dir/none" "$dir/owned"
  rcase "a breach the report does not name fails" 1 "names no breach" 1 "$dir/empty.md" "$dir/rel-claims" "$dir/owned"
  release_self_test "$dir" || fail=1
  local codeowners=$'# Code owners\n\n*                @owner-one\n/docs/ @someone-else\n*  @owner-two\n'
  if [[ "$(owner_of "$codeowners")" != owner-one ]]; then
    echo "::error::self-test: the owner of * is $(owner_of "$codeowners"), want owner-one" >&2
    fail=1
  fi
  # The CODEOWNERS text arrives base64-encoded and wrapped, as the contents API sends it.
  local wrapped
  wrapped=$(printf '%s' "$codeowners" | base64 | fold -w 60)
  if [[ "$(owner_of "$(base64 -d <<<"$wrapped")")" != owner-one ]]; then
    echo "::error::self-test: the owner of a wrapped base64 CODEOWNERS does not read" >&2
    fail=1
  fi
  rm -rf "$dir"
  [[ "$fail" == 0 ]] || { echo "accepted: self-test FAILED" >&2; exit 1; }
  echo "accepted: self-test OK (a breach passes only with the label, a Perf-accepted line per metric and the owner's acceptance on its issue)"
}

[[ "$#" -ge 1 ]] || usage
case "$1" in
judge) shift; judge "$@" ;;
release) shift; release "$@" ;;
--self-test) self_test ;;
*) usage ;;
esac
