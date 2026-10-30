#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Regenerate the published conformance visuals (the capability heat grid +
# the per-chapter outcome bars) FROM the committed party artifacts — the
# render-perf-assets.sh pattern for functional conformance: deterministic,
# CI regenerate-and-diff guarded, light+dark, numbers rendered never
# hand-typed. Output: website/book/src/conformance-assets/, which the landing
# page embeds too. The committed record is a Veredictum 0.1.6 document, so it
# renders with that release (VEREDICTUM_RECORD_VERSION) until the record of the
# current pin replaces it.
set -euo pipefail
cd "$(dirname "$0")/../.."
# shellcheck source=scripts/lib/veredictum.sh
source scripts/lib/veredictum.sh

ART="docs/conformance/ferroehr"
for f in "$ART/results.json" "$ART/verdicts.json"; do
  [[ -f "$f" ]] || {
    echo "render-conformance-assets: $f missing — run the conformance suite first" >&2
    exit 1
  }
done

"$(veredictum_bin "$VEREDICTUM_RECORD_VERSION")" conformance-assets \
  --root "$(veredictum_record_artifacts)" \
  --results "$ART/results.json" \
  --verdicts "$ART/verdicts.json" \
  --out website/book/src/conformance-assets \
  --suffix=""

# The landing page embeds the product's heat grid straight from the book's
# committed conformance-assets copy (served at /docs/latest/) — no second
# committed copy exists.
