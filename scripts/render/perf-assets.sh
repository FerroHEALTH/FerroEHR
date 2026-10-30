#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Render the published performance SVG assets FROM the committed measurement
# records (docs/conformance/ferroehr/results.json measurements block, plus the
# stress report when one exists) via `veredictum perf-assets`. Deterministic
# by construction: the docs CI job re-runs this script and
# `git diff --exit-code`s the output, so a hand-drawn or stale asset fails
# the build (the same honesty rule as check-conformance-numbers.sh).
#
# The committed record is a Veredictum 0.1.6 document, so it renders with that
# release (VEREDICTUM_RECORD_VERSION) until the measured lanes re-measure it.
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
RESULTS="docs/conformance/ferroehr/results.json"
STRESS="docs/conformance/ferroehr/stress.json"

args=(perf-assets
  --root "$(veredictum_record_artifacts)"
  --results "$RESULTS"
  --out "$OUT"
  --summary "$SUMMARY")
# The latency-throughput stress curve renders only once a committed stress
# report exists (veredictum stress — exploration, never a conformance record).
[[ -f "$STRESS" ]] && args+=(--stress "$STRESS")

"$(veredictum_bin "$VEREDICTUM_RECORD_VERSION")" "${args[@]}"
