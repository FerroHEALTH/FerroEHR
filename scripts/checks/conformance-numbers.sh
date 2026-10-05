#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The stale-numbers gate (CI, docs workflow): fails the build if a hand-typed
# conformance OR performance claim appears in the site SOURCES. Conformance
# numbers/verdicts and performance rates/latencies on the website are derived
# at build time from the committed runner artifacts (by
# scripts/render/conformance-stats.sh and scripts/render/perf-assets.sh) —
# sources must carry only the data-cnf markers / {{#include}} directives / the
# generated SVG assets, never literal numbers.
#
# Usage: scripts/checks/conformance-numbers.sh   (no arguments)
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"
cd "$(dirname "$0")/../.."

# Three claim shapes are forbidden in website/landing + website/book/src +
# README.md:
#   1. a 1-4 digit number adjacent to conformance-count vocabulary
#   2. a spelled-out profile verdict (CORE/STANDARD/OPTIONS with PASS/FAIL/OBTAINED)
#   3. a digit run adjacent to a performance rate/latency/size unit
# website/book/generated/ is the render OUTPUT and is exempt (and gitignored).
# Digit-anchored: a number immediately (allowing intervening HTML tags)
# followed by the forbidden vocabulary/unit. Digit-less mechanism prose
# ("every executed case passed", "a peak factor of eight", "10:1") is
# deliberately NOT matched.
pattern_counts='[0-9]{1,4}[[:space:]]*(<[^>]*>)*[[:space:]]*(cases?([- ]by[- ]format)?|executions?|passed|failed|skipped|executed|documented skips)([^_a-zA-Z]|$)'
pattern_verdicts='(CORE|STANDARD|OPTIONS|Core|Standard|Options)[[:space:]]*[:·][[:space:]]*(PASS|FAIL|OBTAINED|Pass|Fail|Obtained)'
# Performance units. The committed SVG assets carry generated (diff-guarded)
# labels, so *.svg is excluded — its numbers are rendered, never hand-typed.
pattern_perf='[0-9][0-9.,]*[[:space:]]*(<[^>]*>)*[[:space:]]*(req/s|/s|ms|MB|GiB)([^_a-zA-Z]|$)'

# The count/verdict patterns exclude *.svg for the same reason the perf
# pattern does: the committed conformance SVGs (heat grid, chapter bars) are
# rendered from verdicts/results and regenerate-and-diff guarded in this same
# workflow — their "N cases" labels are generated, never hand-typed.
fail=0
for pattern in "$pattern_counts" "$pattern_verdicts"; do
  if hits=$(grep -rInE --exclude='*.svg' "$pattern" website/landing website/book/src README.md 2>/dev/null); then
    echo "$hits"
    fail=1
  fi
done
if hits=$(grep -rInE --exclude='*.svg' "$pattern_perf" website/landing website/book/src README.md 2>/dev/null); then
  echo "$hits"
  fail=1
fi

if [[ "$fail" -ne 0 ]]; then
  echo "::error::Hand-typed conformance/performance numbers or verdicts found in site sources (above). The website derives conformance claims from docs/conformance/ferroehr/ and performance claims from the committed measurement records — use the data-cnf markers (landing), the generated includes (book), or the generated SVG assets, never literals. See scripts/render/conformance-stats.sh and scripts/render/perf-assets.sh." >&2
  exit 1
fi
echo "check-conformance-numbers: no hand-typed conformance/performance claims in site sources."
