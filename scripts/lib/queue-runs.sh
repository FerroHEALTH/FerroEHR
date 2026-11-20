#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The merge-queue CI runs that checked a commit's tree, read from the GitHub
# API (#5826, #5834).
#
# The queue's commit and the commit it lands on main share a tree and never a
# SHA: main's commit is a new one GitHub writes and signs. So a merge-queue
# run is found by its head commit's tree, never by the pushed SHA. No openEHR
# spec governs this: our own design.
#
# Source it from a script or a workflow step (REPO=<owner/name>, `gh` with
# actions: read):
#
#   # shellcheck source=scripts/lib/queue-runs.sh
#   . scripts/lib/queue-runs.sh
#   run=$(queue_verified "$sha")    # 0 found, 1 none, 2 the API failed
#
# or run `scripts/lib/queue-runs.sh --self-test`. CI_RUNS_STUB names a
# directory that answers instead of the API: `runs` holds `push <sha> <run>`
# and `merge_group <tree> <run>` lines, `jobs` holds
# `<run><TAB><job name><TAB><conclusion>` lines.

# Every read rides out GitHub's transient 5xx answers (scripts/lib/gh-retry.sh,
# #4173): one 502 on the listing failed change detection on every pull request.
# shellcheck source=scripts/lib/gh-retry.sh
. "$(dirname "${BASH_SOURCE[0]}")/gh-retry.sh"

# How far before a commit its merge-queue runs are listed: a queue entry's run
# starts before its commit lands on main.
QUEUE_LEAD_DAYS=7

# queue_since SHA: the day (YYYY-MM-DD) QUEUE_LEAD_DAYS before SHA's commit.
queue_since() {
  local since
  since=$(($(git log -1 --format=%ct "$1") - QUEUE_LEAD_DAYS * 86400))
  date -u -d "@$since" +%F 2>/dev/null || date -u -r "$since" +%F
}

# queue_runs_since SINCE FILE: writes `<tree> <run>` to FILE for every
# completed merge-queue CI run created on or after SINCE (YYYY-MM-DD).
queue_runs_since() {
  local since="$1" file="$2"
  if [[ -n "${CI_RUNS_STUB:-}" ]]; then
    awk '$1 == "merge_group" { print $2, $3 }' "$CI_RUNS_STUB/runs" >"$file"
  else
    gh_retry gh api --paginate \
      "repos/$REPO/actions/workflows/ci.yml/runs?event=merge_group&status=completed&created=>=$since&per_page=100" \
      --jq '.workflow_runs[] | "\(.head_commit.tree_id) \(.id)"' >"$file"
  fi
}

# run_jobs RUN: `<name><TAB><conclusion>` for the latest attempt of each job.
run_jobs() {
  local run="$1"
  if [[ -n "${CI_RUNS_STUB:-}" ]]; then
    awk -F'\t' -v run="$run" '$1 == run { print $2 "\t" $3 }' "$CI_RUNS_STUB/jobs"
  else
    gh_retry gh api --paginate "repos/$REPO/actions/runs/$run/jobs?filter=latest&per_page=100" \
      --jq '.jobs[] | "\(.name)\t\(.conclusion)"'
  fi
}

# queue_verified SHA: prints the merge-queue run of SHA's tree whose required
# `conclusion` job passed and returns 0; returns 1 when none did, 2 when the
# API could not answer. The run's own conclusion is no answer: a job can fail
# it while the required `conclusion` check passed.
queue_verified() {
  local sha="$1" tree index run jobs read_failed=no
  tree="$(git rev-parse "$sha^{tree}")" || return 2
  index="$(mktemp)"
  if ! queue_runs_since "$(queue_since "$sha")" "$index"; then
    rm -f "$index"
    return 2
  fi
  while IFS= read -r run; do
    if ! jobs="$(run_jobs "$run")"; then
      read_failed=yes
      continue
    fi
    if [[ "$(awk -F'\t' '$1 == "conclusion" { print $2 }' <<<"$jobs")" == success ]]; then
      rm -f "$index"
      echo "$run"
      return 0
    fi
  done < <(awk -v tree="$tree" '$1 == tree { print $2 }' "$index")
  rm -f "$index"
  [[ "$read_failed" == no ]] || return 2
  return 1
}

_queue_runs_self_test() {
  local work c1 c2 tree2 got status fail=0
  work="$(mktemp -d)"
  stub_git() {
    git -C "$work" -c user.name=self-test -c user.email=self-test@example.org \
      -c commit.gpgsign=false -c core.hooksPath=/dev/null "$@"
  }
  stub_git init -q -b main
  for n in 1 2; do
    echo "$n" >"$work/f"
    stub_git add f
    stub_git commit -q -m "c$n"
  done
  c2="$(stub_git rev-parse HEAD)"; c1="$(stub_git rev-parse HEAD~1)"
  tree2="$(stub_git rev-parse "HEAD^{tree}")"
  mkdir "$work/stub"
  # expect LABEL WANT_STATUS WANT_RUN RUN-LINE...: each RUN-LINE is
  # `<event> <key> <run> <conclusion job's conclusion>`.
  expect() {
    local label="$1" want_status="$2" want_run="$3" line event key run conclusion
    shift 3
    : >"$work/stub/runs"
    : >"$work/stub/jobs"
    for line in "$@"; do
      read -r event key run conclusion <<<"$line"
      echo "$event $key $run" >>"$work/stub/runs"
      printf '%s\tconclusion\t%s\n' "$run" "$conclusion" >>"$work/stub/jobs"
    done
    status=0
    got="$(cd "$work" && CI_RUNS_STUB="$work/stub" queue_verified "$c2")" || status=$?
    if [[ "$status" != "$want_status" || "$got" != "$want_run" ]]; then
      echo "::error::queue-runs --self-test: ${label}: wanted ${want_status} '${want_run}', got ${status} '${got}'." >&2
      fail=1
    else
      echo "  ok: ${label}"
    fi
  }
  expect "the queue run of the pushed commit's tree verifies it" 0 7 "merge_group $tree2 7 success"
  expect "a queue run is matched by its tree, never by the pushed SHA" 1 "" "merge_group $c2 7 success"
  expect "a queue run of another tree does not verify it" 1 "" \
    "merge_group $(stub_git rev-parse "$c1^{tree}") 7 success"
  expect "a queue run whose conclusion failed does not verify it" 1 "" "merge_group $tree2 7 failure"
  expect "a later queue run of the same tree that passed verifies it" 0 8 \
    "merge_group $tree2 7 failure" "merge_group $tree2 8 success"
  expect "a push run never verifies it" 1 "" "push $c2 7 success"
  rm -rf "$work"
  return "$fail"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  set -euo pipefail
  case "${1:-}" in
  --self-test) _queue_runs_self_test ;;
  *)
    echo "usage: $0 --self-test (source this file for queue_verified)" >&2
    exit 2
    ;;
  esac
fi
