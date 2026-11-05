#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Record the Mitz Implementatiehandleiding Open en Gesloten Vraag 3.8.2 in
# docs/specs/mitz/, the document that defines the closed authorization
# question the party binding's Mitz consent source asks.
#
# The document states no licence, so it is cache only: the script fetches it
# into the git-ignored .vendor-cache/mitz/, refuses it unless it hashes to the
# pin, and writes the record. No copy enters the repository
# (.claude/rules/vendored-corpora.md).
#
# Usage:
#   scripts/vendor/mitz.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=scripts/vendor/lib/pinned.sh
. "$ROOT/scripts/vendor/lib/pinned.sh"

PAGE="https://vzvz.atlassian.net/wiki/spaces/MA11/pages/828314367/Bijlage+Architectuurdocumenten"
ATTACHMENTS="https://vzvz.atlassian.net/wiki/download/attachments/828314367"
DEST="$ROOT/docs/specs/mitz"
RETRIEVED="2026-11-05"

# `file sha256` per cache-only document, by its attachment name.
CACHED=(
  "VZVZ_Mitz_Implementatiehandleiding_OpenGesloten_v3.8.2.pdf a5ce8f0d7eba8969a395a8560cf69f9e359f9da4c76145adc0748b9907ac0cf3"
)

pinned_require curl shasum

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/tree"

for pin in "${CACHED[@]}"; do
  pinned_cache "${ATTACHMENTS}/${pin% *}?version=1&api=v2" mitz "${pin% *}" "${pin##* }"
done

cat >"$work/tree/PROVENANCE.md" <<EOF
# Provenance: the Mitz Implementatiehandleiding 3.8.2, cache only

**This directory holds this record and nothing else.** The document is read
from the cache and never committed.

- Source: an attachment of the VZVZ Mitz afsprakenstelsel Confluence page
  "Bijlage Architectuurdocumenten" (page id \`828314367\` in space \`MA11\`,
  <${PAGE}>), readable without an account, downloaded from
  <${ATTACHMENTS}/${CACHED[0]% *}?version=1&api=v2>, which redirects to the
  Atlassian media store
- Document: VZVZ Mitz Implementatiehandleiding Open en Gesloten Vraag,
  version 3.8.2, 27 May 2024, Definitief; attachment version 1
- Retrieved: ${RETRIEVED}, checked against the sha256 below, the same
  sha256 as FerroHEALTH/FerroFED records in \`docs/specs/mitz/\`
- Upstream licence: none stated. The document carries no licence,
  copyright notice or terms of use, and its Confluence page states none (read
  ${RETRIEVED}). With no grant, the script fetches it into the git-ignored
  \`.vendor-cache/mitz/\` and this repository carries none of its content.
- What the party binding cites: §2.3 and §3.2, the closed authorization
  question, with its attributes in §3.2.4.2 (the responsible professional
  among them) and its expected actions in §3.2.4.6
- Fetched by: \`scripts/vendor/mitz.sh\`; never add a file here, re-run the
  script to update (.claude/rules/vendored-corpora.md)

| File | sha256 | Status |
|---|---|---|
$(pinned_status_rows "cache only" "${CACHED[@]}")
EOF

pinned_install "$work/tree" "$DEST"
say "${#CACHED[@]} document cache only, recorded in docs/specs/mitz/"
