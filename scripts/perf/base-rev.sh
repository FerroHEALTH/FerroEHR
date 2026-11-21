#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The base the performance gate's A/B run measures against (#5741, #5956):
# the base branch's head, the first parent of the commit under test, unless
# CI proved that commit's database suite broken.
#
# The head arm is the pull request's merge commit, so it carries every commit
# of the base branch. Any commit the walk passes over is in the head and not
# in the base, and the gate bills its cost to the change (#5956). So the walk
# passes over a commit only for the one reason it exists for: a base whose
# database suite fails cannot boot, and would block the change that repairs
# it (#5740). A commit is broken when a CI run of it on push, or a merge-queue
# run of its tree, ran a `build & test (pgN)` shard that failed, and no run
# of it passed every shard. A commit whose CI is still running, whose run did
# not reach the shards, or whose run failed only in another job (cargo-deny,
# a lint) is the base: none of that says the server cannot boot.
#
# A merge-queue run is matched by its head commit's tree
# (`scripts/lib/queue-runs.sh`). No openEHR spec governs this: our own design.
#
# Usage:
#   scripts/perf/base-rev.sh <start-rev> [max-commits]
#       prints `rev=<sha>` and `reason=<text>` for $GITHUB_OUTPUT, and fails
#       naming the commits it read when each of the first max-commits
#       (default 30) is broken. Needs `gh` with actions: read and
#       REPO=<owner/name>.
#   scripts/perf/base-rev.sh --self-test
#       runs the walk over a stub history and stub CI runs.
set -euo pipefail

# shellcheck source=scripts/lib/queue-runs.sh
. "$(dirname "$0")/../lib/queue-runs.sh"

QUEUE_INDEX=""

# runs SHA TREE: the completed CI runs of SHA on push, then the merge-queue
# runs whose head commit carries TREE.
runs() {
  local sha="$1" tree="$2"
  if [[ -n "${CI_RUNS_STUB:-}" ]]; then
    awk -v sha="$sha" '$1 == "push" && $2 == sha { print $3 }' "$CI_RUNS_STUB/runs"
  else
    gh_retry gh api "repos/$REPO/actions/workflows/ci.yml/runs?event=push&head_sha=$sha&per_page=20" \
      --jq '.workflow_runs[] | select(.status == "completed") | .id'
  fi
  awk -v tree="$tree" '$1 == tree { print $2 }' "$QUEUE_INDEX"
}

# verdict SHA: `broken` when a CI run of SHA, or a merge-queue run of its
# tree, ran a test shard that failed and no run passed every test shard;
# `green` when a run passed every one; otherwise a word saying why the suite
# has no verdict yet (no-run, tests-not-run, unreadable), which never makes
# the commit broken.
verdict() {
  local sha="$1" tree list run found ran failed state=no-run
  tree="$(git rev-parse "$sha^{tree}")"
  list="$(runs "$sha" "$tree")" || { echo unreadable; return; }
  while IFS= read -r run; do
    [[ -n "$run" ]] || continue
    found="$(run_jobs "$run")" || { [[ "$state" == broken ]] || state=unreadable; continue; }
    ran="$(awk -F'\t' '$1 ~ /^build & test \(pg[0-9]+\)/ && $2 != "skipped"' <<<"$found" | wc -l | tr -d ' ')"
    failed="$(awk -F'\t' '$1 ~ /^build & test \(pg[0-9]+\)/ && $2 != "skipped" && $2 != "success"' <<<"$found" | wc -l | tr -d ' ')"
    if [[ "$ran" -gt 0 && "$failed" -eq 0 ]]; then
      echo green
      return
    fi
    if [[ "$failed" -gt 0 ]]; then
      state=broken
    elif [[ "$state" != broken ]]; then
      state=tests-not-run
    fi
  done <<<"$list"
  echo "$state"
}

walk() {
  local start="$1" max="$2" sha state since read=() shas=()
  while IFS= read -r sha; do shas+=("$sha"); done \
    < <(git rev-list --first-parent --max-count="$max" "$start")
  since="$(queue_since "${shas[${#shas[@]}-1]}")"
  QUEUE_INDEX="$(mktemp)"
  if ! queue_runs_since "$since" "$QUEUE_INDEX"; then
    rm -f "$QUEUE_INDEX"
    echo "::error::base-rev: could not list the merge-queue runs created since $since" >&2
    return 1
  fi
  for sha in "${shas[@]}"; do
    state="$(verdict "$sha")"
    read+=("${sha:0:9}=$state")
    if [[ "$state" != broken ]]; then
      rm -f "$QUEUE_INDEX"
      echo "rev=$sha"
      if [[ "$sha" == "$(git rev-parse "$start")" ]]; then
        echo "reason=the base branch's head (database suite: $state)"
      else
        echo "reason=the newest commit not broken, ${#read[@]} back from the base branch's head (database suite: $state); the head also carries the broken commits ${read[*]:0:${#read[@]}-1}, which this base lacks, so their cost is in the comparison"
      fi
      return 0
    fi
  done
  rm -f "$QUEUE_INDEX"
  echo "::error::base-rev: each of the ${max} commits from ${start:0:9} failed its database suite in CI: ${read[*]}" >&2
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
  tree4="$(stub_git rev-parse "HEAD^{tree}")"
  mkdir "$work/stub"
  fail=0
  # ci ID EVENT KEY CONCLUSION SHARD...: one stub CI run, its `conclusion` job
  # and a pg19 shard job per SHARD conclusion.
  ci() {
    local id="$1" event="$2" key="$3" conclusion="$4" n=0 shard
    shift 4
    echo "$event $key $id" >>"$work/stub/runs"
    printf '%s\tconclusion\t%s\n' "$id" "$conclusion" >>"$work/stub/jobs"
    for shard in "$@"; do
      n=$((n + 1))
      printf '%s\tbuild & test (pg19) %s/%s\t%s\n' "$id" "$n" "$#" "$shard" >>"$work/stub/jobs"
    done
  }
  # expect LABEL MAX WANT [START]: the walk from START (default c4, a pull
  # request's base branch head; c3 is a push of c4, whose start is HEAD^1).
  expect() {
    local label="$1" max="$2" want="$3" start="${4:-$c4}" got
    touch "$work/stub/runs" "$work/stub/jobs"
    got="$(cd "$work" && CI_RUNS_STUB="$work/stub" bash "$script" "$start" "$max" 2>/dev/null \
      | sed -n 's/^rev=//p' || true)"
    if [[ "$got" != "$want" ]]; then
      echo "::error::--self-test: ${label}: wanted '${want}', got '${got}'." >&2
      fail=1
    else
      echo "  ok: ${label}"
    fi
    rm -f "$work/stub/runs" "$work/stub/jobs"
  }
  ci 1 push "$c4" success success success
  expect "a green head is the base" 30 "$c4"
  expect "a head with no CI run yet is the base" 30 "$c4"
  ci 1 push "$c4" success skipped
  expect "a head whose run did not reach the shards is the base" 30 "$c4"
  ci 1 push "$c4" failure success success
  expect "a head red only outside the database suite is the base" 30 "$c4"
  ci 1 push "$c4" failure success failure
  ci 2 push "$c3" failure success failure
  ci 3 push "$c2" success success success
  ci 4 push "$c1" success success success
  expect "a broken head is passed over for the newest commit not broken" 30 "$c2"
  ci 1 push "$c4" failure success failure
  ci 2 push "$c3" failure
  ci 3 push "$c2" failure failure
  expect "a pending commit under a broken head is the base" 30 "$c3"
  ci 1 push "$c4" failure success failure
  ci 2 push "$c4" success success success
  expect "a shard that passed on another run is not broken" 30 "$c4"
  ci 1 push "$c4" failure failure
  ci 2 push "$c3" failure success failure
  ci 3 push "$c2" success success
  expect "a bound of broken commits fails" 2 ""
  ci 1 push "$c2" success success success
  expect "a push of c4 measures against its parent c3 while c3's CI is still running" 30 "$c3" "$c3"
  ci 1 push "$c3" failure success success
  ci 2 push "$c2" success success success
  expect "a push of c4 measures against its parent c3 when c3 failed only outside its shards" 30 "$c3" "$c3"
  ci 1 push "$c4" success skipped
  ci 2 merge_group "$tree4" success success success
  ci 3 push "$c3" success success
  expect "a push that skipped its shards counts by the queue run of its tree" 30 "$c4"
  ci 1 push "$c4" success skipped
  ci 2 merge_group "$tree4" success success failure
  ci 3 push "$c3" success success
  expect "a queue run of the tree with a failed shard makes the commit broken" 30 "$c3"
  ci 1 push "$c4" success skipped
  ci 2 merge_group "$c4" success success failure
  ci 3 push "$c3" success success
  expect "a queue run is matched by its tree, never by the commit's SHA" 30 "$c4"
  exit "$fail"
fi

[[ $# -ge 1 ]] || { echo "usage: $0 <start-rev> [max-commits] | --self-test" >&2; exit 2; }
walk "$1" "${2:-30}"
