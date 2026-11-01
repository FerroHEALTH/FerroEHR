#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# scripts/watch/ci-budget.sh — the execution time of each budgeted CI lane on main.
#
# WHY (#3131, #3490): a shard count fixes a lane for the suite it was measured
# against and no longer. This reads the recent SUCCESSFUL main runs and, per
# lane, takes the slowest matching job of each run and the median over runs.
# For ci.yml those are its merge-queue runs: a push to main the queue passed
# skips most of ci.yml's lanes (#4546), and every change merges through a
# queue run of its own.
#
# A lane's figure is JOB time, `started_at` to `completed_at`, never the run's
# wall clock. The run's clock also counts the time each job waited for a runner,
# which reached 49 minutes on main runs when several ran at once, and no shard
# count can shorten that queue. Runs in which a lane did not run (a docs-only
# push skips the cargo jobs) do not count towards its sample.
#
# The `ui-e2e server binary` job has no shard count: every ui-e2e shard waits
# for it, so it is the critical path of that lane and is budgeted on its own.
#
# Usage:
#   scripts/watch/ci-budget.sh --repo OWNER/NAME [--out details.md]
#   scripts/watch/ci-budget.sh --self-test
# Prints one line per lane. With --out, writes one markdown bullet per lane
# over its budget (an empty file when none is). Requires gh (authenticated)
# and jq. Exits non-zero only when the PROBE cannot answer — the API
# unreachable, or an answer jq cannot read. A lane over budget is a successful
# run (the watcher family's run-colour law, #2778).
set -euo pipefail

REPO=""
OUT=""
SELF_TEST=false
while [[ "$#" -gt 0 ]]; do
  case "$1" in
  --self-test)
    SELF_TEST=true
    shift
    ;;
  --repo)
    REPO="${2:?--repo needs OWNER/NAME}"
    shift 2
    ;;
  --out)
    OUT="${2:?--out needs a path}"
    shift 2
    ;;
  *)
    echo "usage: $0 --repo OWNER/NAME [--out details.md] | --self-test" >&2
    exit 2
    ;;
  esac
done
[[ -n "$REPO" || "$SELF_TEST" == true ]] || {
  echo "usage: $0 --repo OWNER/NAME [--out details.md] | --self-test" >&2
  exit 2
}

for tool in gh jq; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "$tool is required" >&2
    exit 1
  }
done

# Label | workflow file | job-name regex (jq `test`) | budget in seconds | remedy.
# A regex cannot hold `|`, the field separator: ui-e2e names its viewport
# (`desktop`, `phone`) with `[a-z]+`.
readonly LANES=(
  'CI test shards|ci.yml|^build & test \(pg18\) [0-9]+/[0-9]+$|900|Raise TEST_SHARDS in .github/workflows/ci.yml.'
  'CI ui-e2e shards|ci.yml|^ui-e2e [a-z]+ [0-9]+/[0-9]+$|900|Raise UI_E2E_SHARDS in .github/workflows/ci.yml.'
  "CI ui-e2e server binary|ci.yml|^ui-e2e server binary$|900|Not shardable: look at [profile.e2e] in Cargo.toml and the job's sccache hit rate."
  'SonarQube Cloud coverage shards|sonar.yml|^coverage [0-9]+/[0-9]+$|900|Raise COVERAGE_SHARDS in .github/workflows/sonar.yml.'
)
# How many successful main runs are scanned, and how many of those in which a
# lane ran form its sample. The median of the sample is the figure, so one
# cold-cache outlier cannot raise an alarm and one warm run cannot silence it.
readonly SCANNED=20
readonly SAMPLE=5

# Label | job name | whether the lane's pattern matches it. A job renamed in
# ci.yml or sonar.yml leaves its lane unbudgeted with no error (#4601), so each
# form a lane's jobs take today is a case here, beside names it must refuse.
readonly CASES=(
  'CI test shards|build & test (pg18) 1/3|yes'
  'CI test shards|build & test (viewer)|no'
  'CI ui-e2e shards|ui-e2e desktop 1/3|yes'
  'CI ui-e2e shards|ui-e2e phone 3/3|yes'
  'CI ui-e2e shards|ui-e2e server binary|no'
  'CI ui-e2e shards|ui-e2e viewer + test binaries|no'
  'CI ui-e2e server binary|ui-e2e server binary|yes'
  'CI ui-e2e server binary|ui-e2e desktop 1/3|no'
  'SonarQube Cloud coverage shards|coverage 2/4|yes'
)

if [[ "$SELF_TEST" == true ]]; then
  failed=0
  for case in "${CASES[@]}"; do
    IFS='|' read -r case_label name want <<<"$case"
    pattern=""
    for lane in "${LANES[@]}"; do
      IFS='|' read -r label _ lane_pattern _ _ <<<"$lane"
      [[ "$label" == "$case_label" ]] && pattern="$lane_pattern"
    done
    if [[ -z "$pattern" ]]; then
      echo "ci-budget self-test: no lane is labelled '$case_label'" >&2
      failed=1
      continue
    fi
    got="$(jq -nr --arg re "$pattern" --arg name "$name" 'if ($name | test($re)) then "yes" else "no" end')"
    if [[ "$got" != "$want" ]]; then
      echo "ci-budget self-test: '$case_label' matches '$name': $got, expected $want" >&2
      failed=1
    fi
  done
  [[ "$failed" -eq 0 ]] || exit 1
  echo "ci-budget: self-test OK (${#CASES[@]} job names against ${#LANES[@]} lanes)"
  exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# The ids of the recent successful main runs of one workflow, newest first:
# ci.yml's merge-queue runs, the main pushes of any other workflow.
runs_of() {
  local file="$1" scope="branch=main"
  [[ "$file" == ci.yml ]] && scope="event=merge_group"
  [[ -s "$WORK/runs-$file" ]] || gh api \
    "repos/$REPO/actions/workflows/$file/runs?$scope&status=success&per_page=$SCANNED" \
    --jq '.workflow_runs[].id' >"$WORK/runs-$file"
  cat "$WORK/runs-$file"
}

# The jobs of one run (its latest attempt), fetched once per run.
jobs_of() {
  local run="$1"
  [[ -s "$WORK/jobs-$run" ]] || gh api "repos/$REPO/actions/runs/$run/jobs?per_page=100" >"$WORK/jobs-$run"
  cat "$WORK/jobs-$run"
}

: >"$WORK/details.md"
for lane in "${LANES[@]}"; do
  IFS='|' read -r label file pattern budget remedy <<<"$lane"
  : >"$WORK/sample"
  runs="$(runs_of "$file")" || {
    echo "::error::could not list the recent main runs of $file" >&2
    exit 1
  }
  while IFS= read -r run; do
    [[ -n "$run" ]] || continue
    jobs="$(jobs_of "$run")" || {
      echo "::error::could not list the jobs of run $run" >&2
      exit 1
    }
    slowest="$(jq -r --arg re "$pattern" \
      '[.jobs[] | select(.conclusion == "success" and (.name | test($re)))
        | ((.completed_at | fromdate) - (.started_at | fromdate))] | max // empty' <<<"$jobs")" || {
      echo "::error::run $run returned jobs jq could not read" >&2
      exit 1
    }
    [[ -n "$slowest" ]] && printf '%s\n' "$slowest" >>"$WORK/sample"
    [[ "$(wc -l <"$WORK/sample")" -ge "$SAMPLE" ]] && break
  done <<<"$runs"

  count="$(wc -l <"$WORK/sample" | tr -d ' ')"
  if [[ "$count" -eq 0 ]]; then
    echo "$label: did not run in the last $SCANNED successful main runs of $file"
    continue
  fi
  median="$(sort -n "$WORK/sample" | sed -n "$((count / 2 + 1))p")"
  echo "$label: median ${median}s over $count runs (budget ${budget}s; sample $(sort -n "$WORK/sample" | paste -sd, -))"
  if [[ "$median" -gt "$budget" ]]; then
    printf -- '- **%s** — median job time over the last %s main runs it ran in: **%ss**, budget %ss. %s\n' \
      "$label" "$count" "$median" "$budget" "$remedy" >>"$WORK/details.md"
  fi
done

if [[ -n "$OUT" ]]; then
  cp "$WORK/details.md" "$OUT"
fi
