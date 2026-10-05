#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Every job in a gated workflow must be reachable from that workflow's sink.
#
# Branch protection points at ci.yml's `conclusion` alone, on purpose: that is
# what lets jobs be added, renamed or path-gated without editing repository
# settings. The cost of that design is that a job missing from the sink's
# `needs` still RUNS and still goes red, while merging stays green — a gate
# that looks enforced and is not. Not hypothetical: on 2026-08-12 four gates
# (`chart-boot`, `error-source-chain`, `no-python`, `spec-citations`) were
# found running without gating anything.
#
# release.yml has the same shape with `announce` as the sink (#2838): a leg
# omitted from `announce.needs` publishes-or-fails invisibly, and the crates
# leg (#2837) was added to it by hand with nothing enforcing the next one —
# so both graphs are checked here.
#
# Usage: scripts/checks/ci-conclusion-complete.sh [<workflow> <sink>]...
#   No args: the built-in table below. Args: explicit workflow/sink pairs.
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_known_flags "[<workflow> <sink>]..." "" "$@"

cd "$(dirname "$0")/../.."

check_graph() {
  local workflow="$1" sink="$2" jobs needs missing stale
  jobs=$(yq -o=json '.jobs | keys' "$workflow" | jq -r '.[]' | grep -vx "$sink" | sort)
  needs=$(yq -o=json ".jobs.${sink}.needs" "$workflow" | jq -r '.[]' | sort)

  if missing=$(comm -23 <(echo "$jobs") <(echo "$needs")) && [[ -n "$missing" ]]; then
    echo "error: $workflow jobs that do not gate through '$sink'" >&2
    echo >&2
    # shellcheck disable=SC2001 # bullets EVERY line of a multi-line list; ${//} has no ^ anchor
    echo "$missing" | sed 's/^/  - /' >&2
    echo >&2
    echo "A job absent from jobs.${sink}.needs can fail without anything" >&2
    echo "asserting it. Add each job above to jobs.${sink}.needs in $workflow." >&2
    return 1
  fi

  # The mirror direction: a needs entry naming a job that no longer exists
  # makes the sink fail permanently on an unresolvable dependency.
  if stale=$(comm -13 <(echo "$jobs") <(echo "$needs")) && [[ -n "$stale" ]]; then
    echo "error: $workflow jobs.${sink}.needs names jobs that do not exist" >&2
    # shellcheck disable=SC2001 # bullets EVERY line of a multi-line list; ${//} has no ^ anchor
    echo "$stale" | sed 's/^/  - /' >&2
    return 1
  fi

  echo "ok: all $(echo "$jobs" | wc -l | tr -d ' ') $workflow jobs gate through '$sink'"
}

if [[ "$#" -gt 0 ]]; then
  [[ $(($# % 2)) -eq 0 ]] || { echo "usage: $0 [<workflow> <sink>]..." >&2; exit 2; }
  pairs=("$@")
else
  pairs=(
    .github/workflows/ci.yml conclusion
    .github/workflows/release.yml announce
  )
fi

failures=0
i=0
while [[ "$i" -lt "${#pairs[@]}" ]]; do
  check_graph "${pairs[$i]}" "${pairs[$((i + 1))]}" || failures=$((failures + 1))
  i=$((i + 2))
done
[[ "$failures" -eq 0 ]]
