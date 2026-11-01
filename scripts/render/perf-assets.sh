#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Render the published performance SVG assets and the book's summary include
# FROM the committed measured record, via `veredictum perf-assets`.
#
# The record is docs/conformance/ferroehr/perf-result.json, the `perf-result`
# `scripts/conformance.sh perf` writes with the pinned release. The pinned
# binary draws perf-saturation.svg, perf-throughput.svg and
# perf-operations.svg from it and writes its `export --markdown` view as the
# summary.
#
# Deterministic by construction: the docs CI job re-runs this script and
# `git diff --exit-code`s the output, so a hand-drawn or stale asset fails
# the build (the same honesty rule as check-conformance-numbers.sh).
#
# Args:
#   $1  output dir override
#   $2  Markdown summary path override
set -euo pipefail
cd "$(dirname "$0")/../.."
# shellcheck source=scripts/lib/veredictum.sh
source scripts/lib/veredictum.sh

OUT="${1:-website/book/src/perf-assets}"
SUMMARY="${2:-website/book/generated/perf-summary.md}"
RECORD="docs/conformance/ferroehr/perf-result.json"
mkdir -p "$OUT" "$(dirname "$SUMMARY")"

if [[ -f "$RECORD" ]]; then
  "$(veredictum_bin)" perf-assets --results "$RECORD" --out "$OUT" --summary "$SUMMARY"
  exit 0
fi

# TODO(#4452): delete this branch with measured.json, stress.json and aql-probe.json once perf-result.json is committed.
# Until then the page keeps the figures of the Veredictum 0.1.6 record, which
# only that release reads.
LEGACY_VERSION="0.1.6"
LEGACY_MEASURED="docs/conformance/ferroehr/measured.json"
LEGACY_STRESS="docs/conformance/ferroehr/stress.json"
args=(perf-assets
  --root "$(veredictum_root "$LEGACY_VERSION")/artifacts"
  --results "$LEGACY_MEASURED"
  --out "$OUT"
  --summary "$SUMMARY")
[[ -f "$LEGACY_STRESS" ]] && args+=(--stress "$LEGACY_STRESS")
"$(veredictum_bin "$LEGACY_VERSION")" "${args[@]}"
