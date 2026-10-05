#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor FerroPULSE's report v1 contract as its machine-readable half: the
# envelope and FerroEHR-metrics JSON Schemas and their valid/invalid examples.
#
# The usage report client (app/ferroehr/src/usage_report) serializes against
# these schemas, and its integration tests validate every payload it renders
# against them. FerroPULSE owns the contract; this copy is never hand-edited —
# bump the pin and re-run (.claude/rules/vendored-corpora.md).
#
# The repository is private, so the fetch goes through the authenticated GitHub
# API (`gh api`, raw media type) rather than raw.githubusercontent.com.
#
# Usage:
#   scripts/vendor/ferropulse-schemas.sh
set -euo pipefail

REPO="FerroHEALTH/FerroPULSE"
COMMIT="6e136a96abcda45d6bceb1c75ba4543a5119254c"
DEST="$(cd "$(dirname "$0")/../.." && pwd)/corpus/ferropulse"

command -v gh >/dev/null 2>&1 || { echo "gh not found on PATH" >&2; exit 1; }

fetch() {
  local path="$1" out="$2"
  mkdir -p "$(dirname "$out")"
  gh api -H 'Accept: application/vnd.github.raw' \
    "repos/${REPO}/contents/${path}?ref=${COMMIT}" >"$out"
  echo "vendored ${path} @ ${COMMIT}"
}

# The files of one directory at the pin, by name.
list() {
  gh api "repos/${REPO}/contents/$1?ref=${COMMIT}" --jq '.[] | select(.type == "file") | .name'
}

rm -rf "$DEST"
fetch schemas/report-envelope.v1.json "$DEST/schemas/report-envelope.v1.json"
fetch schemas/ferroehr-metrics.v1.json "$DEST/schemas/ferroehr-metrics.v1.json"
for schema in report-envelope.v1 ferroehr-metrics.v1; do
  for validity in valid invalid; do
    dir="schemas/examples/${schema}/${validity}"
    while IFS= read -r name; do
      fetch "${dir}/${name}" "$DEST/${dir}/${name}"
    done < <(list "$dir")
  done
done

cat >"$DEST/PROVENANCE.md" <<EOF
# Provenance

- Upstream: https://github.com/${REPO} (private; Cadasto B.V., the copyright
  holder of this repository too)
- Ref: commit \`${COMMIT}\`
- Files: \`schemas/report-envelope.v1.json\` and
  \`schemas/ferroehr-metrics.v1.json\` (JSON Schema 2020-12), and
  \`schemas/examples/\` (the contract's own valid and invalid examples)
- Licence: upstream declares \`LicenseRef-Proprietary\`; the copyright holder is
  Cadasto B.V., which also holds this repository, and \`REUSE.toml\` declares
  this subtree under the repository's BUSL-1.1
- Fetched by: \`scripts/vendor/ferropulse-schemas.sh\`; never hand-edit, re-run
  the script to update (.claude/rules/vendored-corpora.md)
- Consumer: \`app/ferroehr/tests/it/usage_report.rs\` validates every payload
  the usage report client renders against the two schemas, and checks the
  validator against the examples first
EOF
echo "wrote $DEST/PROVENANCE.md"
