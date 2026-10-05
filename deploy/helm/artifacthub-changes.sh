#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# Derive the per-release Artifact Hub annotations from CHANGELOG.md, so the hub
# listing has ONE source of truth rather than a second changelog that drifts.
#
# The hub's change `kind` values — added, changed, deprecated, removed, fixed,
# security — ARE Keep a Changelog's subsection names
# (https://artifacthub.io/docs/topics/annotations/helm/), which this project
# already maintains and already guards (scripts/checks/changelog-structure.sh),
# so the mapping is one-to-one and needs no editorial decision.
#
# Usage:
#   deploy/helm/artifacthub-changes.sh <version|Unreleased>   # print the YAML
#   deploy/helm/artifacthub-changes.sh 3.17.4 --inject        # write into Chart.yaml
#
# The parsing lives in artifacthub-changes.awk beside this file.
#
# Emits `artifacthub.io/changes` and, when the section has a `### Security`
# subsection, `artifacthub.io/containsSecurityUpdates: "true"`.
#
# WHY the whole release section rather than only the chart's own entries: a chart
# version is published at a release and its `appVersion` names that release, so
# what a consumer is deciding about is the release the chart deploys. There is no
# machine-readable way to split "chart change" from "server change" in the
# changelog, and guessing from wording would silently drop entries.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
CHANGELOG="${REPO_ROOT}/CHANGELOG.md"
CHART="${SCRIPT_DIR}/ferroehr/Chart.yaml"

VERSION="${1:-Unreleased}"
INJECT=0
[[ "${2:-}" == "--inject" ]] && INJECT=1

command -v awk >/dev/null 2>&1 || {
  echo "awk is required (there is no Python in this repository — owner directive)" >&2
  exit 1
}

RENDERED="$(awk -v version="$VERSION" -f "${SCRIPT_DIR}/artifacthub-changes.awk" "$CHANGELOG")"

if [[ "$INJECT" -eq 0 ]]; then
  printf '%s\n' "$RENDERED"
  exit 0
fi

# Append under the existing `annotations:` mapping. Refuses rather than
# duplicating a key: a second `artifacthub.io/changes` would make the packaged
# chart's metadata depend on YAML key-collision behaviour.
grep -q '^annotations:' "$CHART" || { echo "no 'annotations:' block in ${CHART}" >&2; exit 1; }
if grep -q 'artifacthub.io/changes:' "$CHART"; then
  echo "${CHART} already carries artifacthub.io/changes — it is injected at package time, not committed" >&2
  exit 1
fi
printf '%s\n' "$RENDERED" >> "$CHART"
echo "injected the ${VERSION} changes into ${CHART}" >&2
