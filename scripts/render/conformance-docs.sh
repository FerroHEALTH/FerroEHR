#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Regenerate the DERIVED party documents (verdicts.json, the three
# CONFORMANCE_*.md renders, the shields.io badges) FROM the committed inputs —
# the party statement, that party's committed results.json, and the pinned
# instrument's catalogue — via `veredictum verdicts`, the same pure pipeline
# scripts/conformance.sh runs at the end of a campaign.
#
# Why it exists as its own lane: the published SVGs are regenerate-and-diff
# guarded in CI while these documents were not, so a catalogue or statement
# change could leave a published statement/certificate silently stale (#2377).
# The docs CI job runs this script and `git diff --exit-code`s the party
# directories, which makes that impossible.
#
# Nothing here re-measures: results.json is an INPUT and is never written.
#
# Env:
#   CONF_SUT   which parties to regenerate (default: every party that has both
#              a committed statement and committed results)
set -euo pipefail
cd "$(dirname "$0")/../.."
# shellcheck source=scripts/lib/veredictum.sh
source scripts/lib/veredictum.sh

if [[ -n "${CONF_SUT:-}" ]]; then
  parties=("$CONF_SUT")
else
  parties=()
  for statement in docs/conformance/party/*/statement.json; do
    party="$(basename "$(dirname "$statement")")"
    [[ -f "docs/conformance/$party/results.json" ]] && parties+=("$party")
  done
fi

VEREDICTUM="$(veredictum_bin)"
ROOT="$(veredictum_artifacts)"

for party in "${parties[@]}"; do
  statement="docs/conformance/party/$party/statement.json"
  results="docs/conformance/$party/results.json"
  for f in "$statement" "$results"; do
    [[ -f "$f" ]] || {
      echo "render-conformance-docs: $f missing — run the conformance suite first" >&2
      exit 1
    }
  done
  echo "==> $party"
  "$VEREDICTUM" verdicts \
    --statement "$statement" \
    --results "$results" \
    --root "$ROOT" \
    --out "docs/conformance/$party"
done
