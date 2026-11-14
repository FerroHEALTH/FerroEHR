#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The summary of a hospital-family record (#5610), and the check that keeps
# its tracked failures from rotting.
#
# Each hospital probe names in `issue` the issues it is tracked by, beside the
# scenario's own #5610. A failing probe whose tracking issues are all closed
# is a fix that did not hold, and a failing probe that names none is a
# finding nobody filed; both are refused loudly. A failing probe with an open
# tracking issue is reported with it.
#
# Usage: scripts/deploy-probes/hospital_summary.sh <record.json> [<summary.md>]
#   GH_TOKEN and GITHUB_REPOSITORY name the tracker the issue states are read
#   from. The markdown goes to <summary.md> (the job summary in CI) and to
#   stdout.
set -euo pipefail

record="${1:?usage: hospital_summary.sh <record.json> [<summary.md>]}"
summary="${2:-/dev/null}"
repo="${GITHUB_REPOSITORY:-FerroHEALTH/FerroEHR}"
scenario=5610

# The state of the issue `n` (OPEN, CLOSED or UNKNOWN).
state_of() {
  gh issue view "$1" -R "$repo" --json state --jq '.state' 2>/dev/null || printf 'UNKNOWN'
}

table="$(mktemp)"
trap 'rm -f "$table"' EXIT
rotten=0
{
  printf '## Hospital scenario (#%s)\n\n' "$scenario"
  jq -r '"passed \(.passed), failed \(.failed), not exercised \(.uncovered)\n"' "$record"
  printf '| Probe | Outcome | Tracked by | Note |\n|---|---|---|---|\n'
  while IFS=$'\t' read -r id outcome issue title; do
    tracked=()
    for n in $(grep -oE '[0-9]+' <<<"$issue" || true); do
      [[ "$n" = "$scenario" ]] || tracked+=("$n")
    done
    note=""
    cells=""
    open=0
    for n in "${tracked[@]+"${tracked[@]}"}"; do
      s="$(state_of "$n")"
      cells+="#$n ($s) "
      [[ "$s" = CLOSED ]] || open=1
    done
    if [[ "$outcome" = fail ]]; then
      if [[ "${#tracked[@]}" -eq 0 ]]; then
        note="**untracked failure: file an issue under #$scenario**"
        rotten=1
      elif [[ "$open" -eq 0 ]]; then
        note="**every tracking issue is closed and the probe still fails**"
        rotten=1
      fi
    fi
    printf '| %s | %s | %s | %s %s |\n' "$id" "$outcome" "${cells:--}" "$title" "$note"
  done < <(jq -r '.probes[] | [.id, .outcome, .issue, .title] | @tsv' "$record")
} > "$table"
tee -a "$summary" < "$table"

if [[ "$rotten" -eq 1 ]]; then
  echo "::error::a hospital probe fails with no open tracking issue; see the summary"
  exit 1
fi
