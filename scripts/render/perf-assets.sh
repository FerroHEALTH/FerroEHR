#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Render the published performance SVG assets FROM the committed measurement
# record (docs/conformance/ferroehr/measured.json, its measurements block, plus
# the stress report when one exists) via `veredictum perf-assets`.
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

# TODO(#4452): re-measure on the current pin's perf/ tree and drop this release.
# The measured record is a Veredictum 0.1.6 document, which only that release reads.
MEASURED_VERSION="0.1.6"

OUT="${1:-website/book/src/perf-assets}"
SUMMARY="${2:-website/book/generated/perf-summary.md}"
MEASURED="docs/conformance/ferroehr/measured.json"
STRESS="docs/conformance/ferroehr/stress.json"

args=(perf-assets
  --root "$(veredictum_root "$MEASURED_VERSION")/artifacts"
  --results "$MEASURED"
  --out "$OUT"
  --summary "$SUMMARY")
# The latency-throughput stress curve renders only once a committed stress
# report exists (veredictum stress — exploration, never a conformance record).
[[ -f "$STRESS" ]] && args+=(--stress "$STRESS")

"$(veredictum_bin "$MEASURED_VERSION")" "${args[@]}"
