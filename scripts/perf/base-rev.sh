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
# alone says nothing about the tree.
#
# The queue's commit and the commit it lands on main share a tree and never a
# SHA (main's commit is one GitHub writes and signs), so a merge-queue run is
# matched by its head commit's tree (#5826). No openEHR spec governs this: our
# own design.
#
# Usage:
#   scripts/perf/base-rev.sh <start-rev> [max-commits]
#       prints `rev=<sha>` and `reason=<text>` for $GITHUB_OUTPUT, and fails
#       naming the commits it read when none of the first max-commits (default
#       30) is green. Needs `gh` with actions: read and REPO=<owner/name>.
#   scripts/perf/base-rev.sh --self-test
#       runs the walk over a stub history and stub CI runs.
set -euo pipefail

# How far before the oldest commit read the merge-queue runs are listed: a
# queue entry's run starts before its commit lands on main.
QUEUE_LEAD_DAYS=7

# BASE_REV_STUB names a directory that answers instead of the API (the
# self-test): `runs` holds `push <sha> <run>` and `merge_group <tree> <run>`
# lines, `jobs` holds `<run><TAB><job name><TAB><conclusion>` lines.
QUEUE_INDEX=""

# queue_index SINCE FILE: writes `<tree> <run>` to FILE for every completed
# merge-queue CI run created on or after SINCE (YYYY-MM-DD).
queue_index() {
  local since="$1" file="$2"
  if [[ -n "${BASE_REV_STUB:-}" ]]; then
    awk '$1 == "merge_group" { print $2, $3 }' "$BASE_REV_STUB/runs" >"$file"
  else
    gh api --paginate \
      "repos/$REPO/actions/workflows/ci.yml/runs?event=merge_group&status=completed&created=>=$since&per_page=100" \
      --jq '.workflow_runs[] | "\(.head_commit.tree_id) \(.id)"' >"$file"
  fi
}

# runs SHA TREE: the completed CI runs of SHA on push, then the merge-queue
# runs whose head commit carries TREE.
runs() {
  local sha="$1" tree="$2"
  if [[ -n "${BASE_REV_STUB:-}" ]]; then
    awk -v sha="$sha" '$1 == "push" && $2 == sha { print $3 }' "$BASE_REV_STUB/runs"
  else
    gh api "repos/$REPO/actions/workflows/ci.yml/runs?event=push&head_sha=$sha&per_page=20" \
      --jq '.workflow_runs[] | select(.status == "completed") | .id'
  fi
  awk -v tree="$tree" '$1 == tree { print $2 }' "$QUEUE_INDEX"
}

# jobs RUN: `<name><TAB><conclusion>` for the latest attempt of each job.
jobs() {
  local run="$1"
  if [[ -n "${BASE_REV_STUB:-}" ]]; then
    awk -F'\t' -v run="$run" '$1 == run { print $2 "\t" $3 }' "$BASE_REV_STUB/jobs"
  else
    gh api --paginate "repos/$REPO/actions/runs/$run/jobs?filter=latest&per_page=100" \
      --jq '.jobs[] | "\(.name)\t\(.conclusion)"'
  fi
}

# verdict SHA: `green` when a CI run of SHA, or a merge-queue run of its tree,
# passed `conclusion` with every pg18 shard run and succeeding, else a word
# saying why not.
verdict() {
  local sha="$1" tree list run found conclusion ran failed seen=no-run
  tree="$(git rev-parse "$sha^{tree}")"
  list="$(runs "$sha" "$tree")" || { echo unreadable; return; }
  while IFS= read -r run; do
    [[ -n "$run" ]] || continue
    found="$(jobs "$run")" || { seen=unreadable; continue; }
    conclusion="$(awk -F'\t' '$1 == "conclusion" { print $2 }' <<<"$found")"
    ran="$(awk -F'\t' '$1 ~ /^build & test \(pg18\)/ && $2 != "skipped"' <<<"$found" | wc -l | tr -d ' ')"
    failed="$(awk -F'\t' '$1 ~ /^build & test \(pg18\)/ && $2 != "skipped" && $2 != "success"' <<<"$found" | wc -l | tr -d ' ')"
    if [[ "$conclusion" == success && "$ran" -gt 0 && "$failed" -eq 0 ]]; then
      echo green
      return
    fi
    if [[ "$ran" -gt 0 || "$seen" == red ]]; then seen=red; else seen=tests-not-run; fi
  done <<<"$list"
  echo "$seen"
}

walk() {
  local start="$1" max="$2" sha state oldest since read=() shas=()
  while IFS= read -r sha; do shas+=("$sha"); done \
    < <(git rev-list --first-parent --max-count="$max" "$start")
  oldest="$(git log -1 --format=%ct "${shas[${#shas[@]}-1]}")"
  since=$((oldest - QUEUE_LEAD_DAYS * 86400))
  since="$(date -u -d "@$since" +%F 2>/dev/null || date -u -r "$since" +%F)"
  QUEUE_INDEX="$(mktemp)"
  if ! queue_index "$since" "$QUEUE_INDEX"; then
    rm -f "$QUEUE_INDEX"
    echo "::error::base-rev: could not list the merge-queue runs created since $since" >&2
    return 1
  fi
  for sha in "${shas[@]}"; do
    state="$(verdict "$sha")"
    read+=("${sha:0:9}=$state")
    if [[ "$state" == green ]]; then
      rm -f "$QUEUE_INDEX"
      echo "rev=$sha"
      if [[ "$sha" == "$(git rev-parse "$start")" ]]; then
        echo "reason=the base branch's head, whose database suite passed in CI"
      else
        echo "reason=the newest green commit ${#read[@]} back from the base branch's head; skipped ${read[*]:0:${#read[@]}-1}"
      fi
      return 0
    fi
  done
  rm -f "$QUEUE_INDEX"
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
  tree4="$(stub_git rev-parse "HEAD^{tree}")"
  mkdir "$work/stub"
  fail=0
  # ci ID EVENT KEY CONCLUSION SHARD...: one stub CI run, its `conclusion` job
  # and a pg18 shard job per SHARD conclusion.
  ci() {
    local id="$1" event="$2" key="$3" conclusion="$4" n=0 shard
    shift 4
    echo "$event $key $id" >>"$work/stub/runs"
    printf '%s\tconclusion\t%s\n' "$id" "$conclusion" >>"$work/stub/jobs"
    for shard in "$@"; do
      n=$((n + 1))
      printf '%s\tbuild & test (pg18) %s/%s\t%s\n' "$id" "$n" "$#" "$shard" >>"$work/stub/jobs"
    done
  }
  expect() {
    local label="$1" max="$2" want="$3" got
    touch "$work/stub/runs" "$work/stub/jobs"
    got="$(cd "$work" && BASE_REV_STUB="$work/stub" bash "$script" "$c4" "$max" 2>/dev/null \
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
  ci 1 push "$c4" success skipped
  ci 2 push "$c3" failure success failure
  ci 3 push "$c2" success success success
  ci 4 push "$c1" success success success
  expect "a broken head is skipped for the newest green commit" 30 "$c2"
  ci 1 push "$c4" failure failure
  ci 2 push "$c3" failure success
  ci 3 push "$c2" success success
  expect "no green commit within the bound fails" 2 ""
  ci 1 push "$c4" success skipped
  ci 2 merge_group "$tree4" success success success
  ci 3 push "$c3" success success
  expect "a push that skipped its shards counts by the queue run of its tree" 30 "$c4"
  ci 1 push "$c4" failure success success
  ci 2 merge_group "$tree4" success success success
  ci 3 push "$c3" success success
  expect "a push failed by another job counts by the queue run of its tree" 30 "$c4"
  ci 1 push "$c4" success skipped
  ci 2 merge_group "$tree4" success success failure
  ci 3 push "$c3" success success
  expect "a queue run of the tree with a failed shard does not count" 30 "$c3"
  ci 1 push "$c4" success skipped
  ci 2 merge_group "$c4" success success success
  ci 3 push "$c3" success success
  expect "a queue run is matched by its tree, never by the commit's SHA" 30 "$c3"
  exit "$fail"
fi

[[ $# -ge 1 ]] || { echo "usage: $0 <start-rev> [max-commits] | --self-test" >&2; exit 2; }
walk "$1" "${2:-30}"
