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
# summary. With no record committed it draws nothing and the performance
# page publishes no figure.
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

if [[ ! -f "$RECORD" ]]; then
  echo "perf-assets: no $RECORD is committed, so there is nothing to draw" >&2
  exit 0
fi

mkdir -p "$OUT" "$(dirname "$SUMMARY")"
"$(veredictum_bin)" perf-assets --results "$RECORD" --out "$OUT" --summary "$SUMMARY"
