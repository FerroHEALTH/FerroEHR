#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Record DICOM PS3.15 2026c, Annex A.5 (the Audit Trail Message Format
# Profile), in docs/specs/dicom-ps3-15/, the profile the access log design
# maps to.
#
# PS3.15 carries a NEMA copyright and grants no licence to redistribute it,
# so it is cache only: the script fetches the pages into the git-ignored
# .vendor-cache/dicom-ps3-15/, refuses each unless it hashes to its pin, and
# writes the record. No copy enters the repository
# (.claude/rules/vendored-corpora.md).
#
# Usage:
#   scripts/vendor/dicom-ps3-15.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=scripts/vendor/lib/pinned.sh
. "$ROOT/scripts/vendor/lib/pinned.sh"

RELEASE="2026c"
BASE="https://dicom.nema.org/medical/dicom/${RELEASE}/output/chtml/part15"
DEST="$ROOT/docs/specs/dicom-ps3-15"
RETRIEVED="2026-11-07"

# `file sha256` per cache-only page, by its name in the chunked HTML edition.
CACHED=(
  "chapter_Notice.html fbe9ce02c1c74bc09226c12202e9301a0fe34303009f6c31e7b904cc8e9a25c1"
  "sect_A.5.html 86ed3a6fff0dd07f0ff1c83c5c250da0e57633181b33cce074b2722b4a1bbaa4"
  "sect_A.5.2.html 972580a2c9e9db09eede5a2259670e72e1f036cb850445d20b0b3a1b40ae9bd9"
  "sect_A.5.3.html 8ded8b3875c49baa999acc3d10e423cc9fb49615c7e817eca446070dce1afd29"
  "sect_A.5.3.2.html c42247d00715384d2eb894d79c03c726085ab9e8b3e144ca73eb37f7262f5d4d"
  "sect_A.5.3.4.html f8ff3bc25b13c326ebd3d21738bb92cb7a17ff8f426a8fd620c9f8fa75cb8f33"
  "sect_A.5.3.5.html f759811d8ea8e8a4307518db3b1fb8664b916b4ea490d58de621e375a5ea899f"
  "sect_A.5.3.10.html 48403141b112aa990b2bf0c36edb631fdc102baa8d1891ace4d73abe936a1d2b"
  "sect_A.5.3.11.html a7089c951de4975bd4be81b7a19a70469177d110c617b2529ff0a13a50e8a1b0"
  "sect_A.5.3.14.html 953ae6e1023cc0717cf2b23cf1764928346fe37945b3a64c937417dca74b33aa"
)

pinned_require curl shasum

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/tree"

for pin in "${CACHED[@]}"; do
  pinned_cache "${BASE}/${pin% *}" dicom-ps3-15 "${pin% *}" "${pin##* }"
done

cat >"$work/tree/PROVENANCE.md" <<EOF
# Provenance: DICOM PS3.15 ${RELEASE}, Annex A.5, cache only

**This directory holds this record and nothing else.** The pages are read
from the cache and never committed.

- Source: the DICOM Standard, release ${RELEASE}, chunked HTML edition, PS3.15
  "Security and System Management Profiles", <${BASE}/>
- Document: Annex A.5 "Audit Trail Message Format Profile": A.5 (the
  conformance requirements and A.5.1, the DICOM Audit Message Schema, derived
  from the RFC 3881 schema), A.5.2 (the general message format conventions)
  and A.5.3 (the DICOM specific audit messages) with the messages the design
  maps to: A.5.3.2 Audit Log Used, A.5.3.4 Data Export, A.5.3.5 Data Import,
  A.5.3.10 Query, A.5.3.11 Security Alert and A.5.3.14 Patient Record; the
  Notice and Disclaimer page of the same part
- Retrieved: ${RETRIEVED}, each page checked against the sha256 below
- Upstream licence: each page reads "Copyright © 2026 NEMA". The Notice and
  Disclaimer page, read ${RETRIEVED}, grants no licence to copy or
  redistribute. With no grant, the script fetches the pages into the
  git-ignored \`.vendor-cache/dicom-ps3-15/\` and this repository carries none
  of their content. Citations name the release, the section and the field.
- Fetched by: \`scripts/vendor/dicom-ps3-15.sh\`; never add a file here,
  re-run the script to update (.claude/rules/vendored-corpora.md)

| File | sha256 | Status |
|---|---|---|
$(pinned_status_rows "cache only" "${CACHED[@]}")
EOF

pinned_install "$work/tree" "$DEST"
say "${#CACHED[@]} pages cache only, recorded in docs/specs/dicom-ps3-15/"
