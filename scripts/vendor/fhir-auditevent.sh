#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor the FHIR R4 and R5 AuditEvent resource pages into
# docs/specs/fhir-auditevent/: the resource page, its formal definitions, its
# mappings (DICOM PS3.15 among them) and the licence page of each release.
#
# The source is the HL7 FHIR specification at its two published releases,
# each page pinned by sha256. The copy is verbatim; move a pin and re-run
# (.claude/rules/vendored-corpora.md).
#
# Usage:
#   scripts/vendor/fhir-auditevent.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=scripts/vendor/lib/pinned.sh
. "$ROOT/scripts/vendor/lib/pinned.sh"

BASE="https://hl7.org/fhir"
DEST="$ROOT/docs/specs/fhir-auditevent"
RETRIEVED="2026-11-07"

# `path sha256` per page, by its path under the specification's root.
PINS=(
  "R4/auditevent.html 62b2896777066614191d069e13ae9db907c8e79c2767ffa7f00caf28be10163a"
  "R4/auditevent-definitions.html ca2eb37e21b90ac52914d89edf16566b1219053e6dd05004c7544d0275b1b0b4"
  "R4/auditevent-mappings.html d00169c754dcfc99106a786d56d34ab746f28105695a49138ef2c94dec867c25"
  "R4/license.html b2aeb02247ca2aa855ed1fce29f62600aeeb8dcc8257ec1e0b40ceaf4fd60a15"
  "R5/auditevent.html 07b9783a9555cba55855718efc419c5ba06038c377b7b5eeb3a30d6fd9ebbf8c"
  "R5/auditevent-definitions.html 577ea4432c9dbed44f65f64e26a03b8e2b7adb5b22c66934c134cfb7d61e45e0"
  "R5/auditevent-mappings.html c36911405faf82a3c95529891fb20ac11690e21a0f3e5079f798b13bb10c1997"
  "R5/license.html 452f8651729bb0e6b93c75a22f772cb7ece300a3585e1da17be944964b44aa90"
)

pinned_require curl shasum

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

pinned_fetch_all "$BASE" "$work/tree" "${PINS[@]}"

cat >"$work/tree/PROVENANCE.md" <<EOF
# Provenance: the FHIR R4 and R5 AuditEvent pages

- Source: the HL7 FHIR specification, <${BASE}/R4/> (FHIR R4, 4.0.1) and
  <${BASE}/R5/> (FHIR R5, 5.0.0), the published releases
- Pin: each page by its sha256 below; a release page does not change once
  published, and the script refuses any other bytes
- Document: the \`AuditEvent\` resource of each release: the resource page
  (scope, boundaries, the element table), its formal definitions and its
  mappings, which carry the DICOM PS3.15 mapping the access log design
  compares against; the licence page of each release
- Retrieved: ${RETRIEVED}
- Upstream licence: read for this copy on ${RETRIEVED}. Each release's
  \`license.html\`, vendored here, states that the specification is licensed
  under Creative Commons "No Rights Reserved" (CC0 1.0). HL7 keeps the
  "FHIR" trademark, which this copy does not use as a mark.
  \`REUSE.toml\` declares the tree \`CC0-1.0\`.
- What is taken: the eight pages above, each whole, without the stylesheets,
  scripts and images they link
- Layout: the release directory, then the page name, as the specification
  serves them
- Fetched by: \`scripts/vendor/fhir-auditevent.sh\`; never hand-edit, re-run
  the script to update (.claude/rules/vendored-corpora.md)

| File | sha256 |
|---|---|
$(pinned_rows "${PINS[@]}")
EOF

pinned_install "$work/tree" "$DEST"
say "${#PINS[@]} files into docs/specs/fhir-auditevent/"
