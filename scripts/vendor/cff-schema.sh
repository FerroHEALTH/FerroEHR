#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor the Citation File Format 1.2.0 JSON Schema (#2791).
#
# The citation-guard CI job validates CITATION.cff against this schema with a
# pinned Rust jsonschema-cli over the yq-converted document — the replacement
# for `pipx run cffconvert --validate`, which contradicted the no-Python hard
# rule (.claude/rules/rust-style.md §No Python). cffconvert validates against
# this SAME schema file, so the swap changes the validator, not the contract.
#
# Upstream: github.com/citation-file-format/citation-file-format, tag 1.2.0,
# CC-BY-4.0 (LICENSE vendored alongside).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)/.github/cff-schema"

REPO="citation-file-format/citation-file-format"
COMMIT="396f738fb025b1d8acdb02a56ffc923f95dc8999" # tag 1.2.0

mkdir -p "$ROOT"
fetch() {
  local path="$1" out="$2"
  curl -fsSL --proto '=https' --proto-redir '=https' \
    "https://raw.githubusercontent.com/${REPO}/${COMMIT}/${path}" -o "${out}"
  echo "vendored ${out} @ ${COMMIT}"
}

fetch schema.json "$ROOT/schema.json"
fetch LICENSE "$ROOT/LICENSE"

cat > "$ROOT/PROVENANCE.md" <<EOF
# Provenance

- Upstream: https://github.com/${REPO}
- Ref: tag \`1.2.0\` (commit \`${COMMIT}\`)
- Files: \`schema.json\` (the CFF 1.2.0 JSON Schema, draft-07), \`LICENSE\`
  (CC-BY-4.0)
- Fetched by: \`scripts/vendor/cff-schema.sh\` — never hand-edit; re-run the
  script to update (.claude/rules/vendored-corpora.md)
- Consumer: the \`citation-guard\` CI job (\`.github/workflows/ci.yml\`)
  validates \`CITATION.cff\` against it with a pinned \`jsonschema-cli\` over
  the \`yq -o=json\` conversion (#2791)
EOF
echo "wrote $ROOT/PROVENANCE.md"
