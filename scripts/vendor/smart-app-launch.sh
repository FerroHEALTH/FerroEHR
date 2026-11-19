#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor the SMART App Launch 2.2.0 Task launch profiles into
# docs/specs/smart-app-launch/: the two StructureDefinitions of the Task
# profile for requesting a SMART launch, their two published examples, the
# smart-codes CodeSystem they bind, and the package manifest.
#
# The source is the FHIR package hl7.fhir.uv.smart-app-launch 2.2.0 from the
# FHIR package registry, its tarball pinned by sha256. The files are taken
# from it verbatim; move the pin and re-run (.claude/rules/vendored-corpora.md).
#
# Usage:
#   scripts/vendor/smart-app-launch.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=scripts/vendor/lib/pinned.sh
. "$ROOT/scripts/vendor/lib/pinned.sh"

PACKAGE="hl7.fhir.uv.smart-app-launch"
VERSION="2.2.0"
TARBALL="https://packages.fhir.org/$PACKAGE/$VERSION"
TARBALL_SHA256="0bb498dd09677684239f24d279b6da711cd814449f2f473c2970fbe881ec9c70"
DEST="$ROOT/docs/specs/smart-app-launch"
RETRIEVED="2026-11-19"

# The files taken from the package, by their path inside it.
FILES=(
  package/package.json
  package/StructureDefinition-task-ehr-launch.json
  package/StructureDefinition-task-standalone-launch.json
  package/CodeSystem-smart-codes.json
  package/example/Task-task-for-ehr-launch.json
  package/example/Task-task-for-standalone-launch.json
)

pinned_require curl shasum tar

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

pinned_fetch "$TARBALL" "$work/package.tgz" "$TARBALL_SHA256"
mkdir -p "$work/x" "$work/tree"
tar -xzf "$work/package.tgz" -C "$work/x"
pins=()
for file in "${FILES[@]}"; do
  [[ -f "$work/x/$file" ]] || die "$PACKAGE $VERSION carries no $file"
  mkdir -p "$work/tree/$(dirname "$file")"
  cp "$work/x/$file" "$work/tree/$file"
  pins+=("$file $(pinned_sha256 "$work/tree/$file")")
  say "$file"
done

cat >"$work/tree/PROVENANCE.md" <<PROV
# Provenance: SMART App Launch 2.2.0, the Task launch profiles

- Source: the FHIR package \`$PACKAGE\` $VERSION from the FHIR package
  registry, <$TARBALL>, the package of
  <https://hl7.org/fhir/smart-app-launch/STU2.2/> (FHIR 4.0.1)
- Pin: the tarball's sha256 \`$TARBALL_SHA256\`; the script refuses any
  other bytes
- Document: the Task profile for requesting a SMART launch
  (\`task-ehr-launch\`, \`task-standalone-launch\`), its two published
  examples, the \`smart-codes\` CodeSystem the profiles' codes come from, and
  the package manifest
- Retrieved: ${RETRIEVED}
- Upstream licence: CC-BY-4.0, the \`license\` of \`package/package.json\`:
  SMART App Launch 2.2.0 by HL7 International / FHIR Infrastructure,
  licensed CC BY 4.0, vendored unchanged. \`REUSE.toml\` declares the tree
  \`CC-BY-4.0\`.
- What is taken: the files below, each whole, at their path in the package
- Fetched by: \`scripts/vendor/smart-app-launch.sh\`; never hand-edit,
  re-run the script to update (.claude/rules/vendored-corpora.md)

| File | sha256 |
|---|---|
$(pinned_rows "${pins[@]}")
PROV

pinned_install "$work/tree" "$DEST"
say "${#FILES[@]} files into docs/specs/smart-app-launch/"
