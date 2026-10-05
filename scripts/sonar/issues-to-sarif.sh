#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# Exports the SonarQube Cloud project's OPEN and CONFIRMED findings as SARIF
# 2.1.0, so `sonar.yml` can upload them to GitHub code scanning (#3032).
# SonarQube Cloud's own GitHub integration is checks and pull-request
# decoration only — its native code-scanning uploads carry the SECURITY class
# alone and arrive empty here, which is why the tools page showed a
# SonarCloud entry with 0 results while the dashboard carried the real list.
#
# The query excludes ACCEPTED and FALSE_POSITIVE on purpose: a disposition
# recorded in the dashboard (the ai-code-review.md law) disappears from the
# next upload, and code scanning then closes the alert — the two surfaces
# cannot disagree for longer than one analysis.
#
# Usage: issues-to-sarif.sh <project-key> <output.sarif>
# Reads SONAR_TOKEN from the environment. When the scanner's
# .scannerwork/report-task.txt is present, waits for that analysis' compute
# task first, so the export reads the run's own result rather than the
# previous one.

set -euo pipefail

PROJECT="${1:?usage: issues-to-sarif.sh <project-key> <output.sarif>}"
OUT="${2:?usage: issues-to-sarif.sh <project-key> <output.sarif>}"
: "${SONAR_TOKEN:?SONAR_TOKEN is required}"

HOST="https://sonarcloud.io"
CURL=(curl -fsS --proto '=https' --tlsv1.2 -H "Authorization: Bearer ${SONAR_TOKEN}")

# The scanner writes the compute-engine task id beside its work directory.
# Polling it is what makes "export after the scan" mean THIS scan: the web API
# serves the last PROCESSED analysis, and processing is asynchronous.
TASK_FILE=".scannerwork/report-task.txt"
if [[ -f "$TASK_FILE" ]]; then
  task_url="$(sed -n 's/^ceTaskUrl=//p' "$TASK_FILE")"
  if [[ -n "$task_url" ]]; then
    for _ in $(seq 1 60); do
      status="$("${CURL[@]}" "$task_url" | jq -r '.task.status')"
      case "$status" in
        SUCCESS) break ;;
        FAILED | CANCELED)
          echo "::error::the analysis compute task ended ${status}, so there is no result to export" >&2
          exit 1
          ;;
        *) sleep 5 ;;
      esac
    done
    if [[ "${status:-}" != "SUCCESS" ]]; then
      echo "::error::the analysis compute task did not finish within the wait budget" >&2
      exit 1
    fi
  fi
fi

# Page through every open finding. The endpoint caps a listing at 10,000
# results, which is two orders of magnitude above this project's worst day;
# hitting the cap fails loud below rather than exporting a silent subset.
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
page=1
while :; do
  "${CURL[@]}" "${HOST}/api/issues/search?projects=${PROJECT}&issueStatuses=OPEN,CONFIRMED&ps=500&p=${page}" \
    >"${tmp}/page-${page}.json"
  total="$(jq -r '.paging.total' "${tmp}/page-${page}.json")"
  if ((total > 10000)); then
    echo "::error::${total} open findings exceed the listing cap; the export would be a silent subset" >&2
    exit 1
  fi
  ((page * 500 >= total)) && break
  page=$((page + 1))
done

# One SARIF run: every finding is a result carrying the Sonar issue key as its
# fingerprint (stable across uploads, so an alert keeps its identity), and the
# rule set is the distinct rules the results actually cite. Severity maps
# BLOCKER/CRITICAL to error, MAJOR to warning, and the rest to note. A finding
# without a text range (a project-level finding) is anchored to line 1 of the
# file, or skipped when it names no file at all.
jq -s --arg project "$PROJECT" --arg host "$HOST" '
  [.[].issues[]] as $issues
  | {
      "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
      version: "2.1.0",
      runs: [{
        automationDetails: { id: "sonarcloud/" },
        tool: { driver: {
          name: "SonarQube Cloud",
          informationUri: ($host + "/project/overview?id=" + $project),
          rules: ([$issues[].rule] | unique | map({
            id: .,
            helpUri: ($host + "/organizations/rubentalstra/rules?open=" + . + "&rule_key=" + .)
          }))
        }},
        results: [
          $issues[]
          | select(.component != $project)
          | {
              ruleId: .rule,
              level: (if .severity == "BLOCKER" or .severity == "CRITICAL" then "error"
                      elif .severity == "MAJOR" then "warning"
                      else "note" end),
              message: { text: .message },
              partialFingerprints: { sonarIssueKey: .key },
              locations: [{
                physicalLocation: {
                  artifactLocation: { uri: (.component | sub("^" + $project + ":"; "")) },
                  region: {
                    startLine: (.textRange.startLine // 1),
                    endLine: (.textRange.endLine // .textRange.startLine // 1)
                  }
                }
              }]
            }
        ]
      }]
    }
' "${tmp}"/page-*.json >"$OUT"

count="$(jq '.runs[0].results | length' "$OUT")"
echo "exported ${count} finding(s) to ${OUT}"
