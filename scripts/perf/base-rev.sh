#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The base the performance gate's A/B run measures against (#5741): the
# newest commit on the first-parent line from <start> (the base branch's head)
# whose database suite passed in CI, not blindly the base branch's head.
#
# A base that cannot boot fails the gate whatever the change does, and a
# change that repairs a broken main could never merge past it (#5740). A
# commit counts as green when a CI run of it passed the required `conclusion`
# check AND ran every `build & test (pg18)` shard to success: the run on push,
# or a merge-queue run of the same tree. The second half matters because a
# push the queue verified skips its test shards (#4546), so its `conclusion`
# alone says nothing about the tree. No openEHR spec governs this: our own
# design.
#
# Usage:
#   scripts/perf/base-rev.sh <start-rev> [max-commits]
#       prints `rev=<sha>` and `reason=<text>` for $GITHUB_OUTPUT, and fails
#       naming the commits it read when none of the first max-commits (default
#       30) is green. Needs `gh` with actions: read and REPO=<owner/name>.
#   scripts/perf/base-rev.sh --self-test
#       runs the walk over a stub history and stub verdicts.
set -euo pipefail

# verdict SHA: `green` when a CI run of SHA passed `conclusion` with every
# pg18 shard succeeding, else a word saying why not. BASE_REV_VERDICTS names a
# file of `<sha> <verdict>` lines that answers instead (the self-test).
verdict() {
  local sha="$1" event run jobs conclusion shards failed
  if [[ -n "${BASE_REV_VERDICTS:-}" ]]; then
    awk -v sha="$sha" '$1 == sha { print $2; found = 1 } END { if (!found) print "no-run" }' \
      "$BASE_REV_VERDICTS"
    return
  fi
  local seen=no-run
  for event in push merge_group; do
    while IFS= read -r run; do
      [[ -n "$run" ]] || continue
      jobs="$(gh api --paginate "repos/$REPO/actions/runs/$run/jobs?filter=latest&per_page=100" \
        --jq '.jobs[] | "\(.name)\t\(.conclusion)"')" || { seen=unreadable; continue; }
      conclusion="$(awk -F'\t' '$1 == "conclusion" { print $2 }' <<<"$jobs")"
      shards="$(awk -F'\t' '$1 ~ /^build & test \(pg18\)/' <<<"$jobs" | wc -l | tr -d ' ')"
      failed="$(awk -F'\t' '$1 ~ /^build & test \(pg18\)/ && $2 != "success"' <<<"$jobs" | wc -l | tr -d ' ')"
      if [[ "$conclusion" == success && "$shards" -gt 0 && "$failed" -eq 0 ]]; then
        echo green
        return
      fi
      if [[ "$shards" -eq 0 ]]; then seen=tests-not-run; else seen=red; fi
    done < <(gh api "repos/$REPO/actions/workflows/ci.yml/runs?event=$event&head_sha=$sha&per_page=20" \
      --jq '.workflow_runs[] | select(.status == "completed") | .id' || true)
  done
  echo "$seen"
}

walk() {
  local start="$1" max="$2" sha state read=()
  while IFS= read -r sha; do
    state="$(verdict "$sha")"
    read+=("${sha:0:9}=$state")
    if [[ "$state" == green ]]; then
      echo "rev=$sha"
      if [[ "$sha" == "$(git rev-parse "$start")" ]]; then
        echo "reason=the base branch's head, whose database suite passed in CI"
      else
        echo "reason=the newest green commit ${#read[@]} back from the base branch's head; skipped ${read[*]:0:${#read[@]}-1}"
      fi
      return 0
    fi
  done < <(git rev-list --first-parent --max-count="$max" "$start")
  echo "::error::base-rev: none of the ${max} commits from ${start:0:9} passed its database suite in CI: ${read[*]}" >&2
  return 1
}

if [[ "${1:-}" == "--self-test" ]]; then
  script="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT
  stub_git() {
    git -C "$work" -c user.name=self-test -c user.email=self-test@example.org \
      -c commit.gpgsign=false -c core.hooksPath=/dev/null "$@"
  }
  stub_git init -q -b main
  for n in 1 2 3 4; do
    echo "$n" > "$work/f"
    stub_git add f
    stub_git commit -q -m "c$n"
  done
  c4="$(stub_git rev-parse HEAD)"; c3="$(stub_git rev-parse HEAD~1)"
  c2="$(stub_git rev-parse HEAD~2)"; c1="$(stub_git rev-parse HEAD~3)"
  fail=0
  expect() {
    local label="$1" verdicts="$2" max="$3" want="$4" got
    printf '%s\n' "$verdicts" > "$work/verdicts"
    got="$(cd "$work" && BASE_REV_VERDICTS="$work/verdicts" bash "$script" "$c4" "$max" 2>/dev/null \
      | sed -n 's/^rev=//p' || true)"
    if [[ "$got" != "$want" ]]; then
      echo "::error::--self-test: ${label}: wanted '${want}', got '${got}'." >&2
      fail=1
    else
      echo "  ok: ${label}"
    fi
  }
  expect "a green head is the base" "$c4 green" 30 "$c4"
  expect "a broken head is skipped for the newest green commit" \
    "$c4 tests-not-run
$c3 red
$c2 green
$c1 green" 30 "$c2"
  expect "no green commit within the bound fails" "$c4 red
$c3 red
$c2 green" 2 ""
  exit "$fail"
fi

[[ $# -ge 1 ]] || { echo "usage: $0 <start-rev> [max-commits] | --self-test" >&2; exit 2; }
walk "$1" "${2:-30}"
