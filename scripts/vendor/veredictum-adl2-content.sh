#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor the Veredictum catalogue's ADL 2 content-validation cases: the setup
# workflow that holds the ADL 2 operational templates and every
# `content/adl2_*` workflow that commits a composition against one of them.
#
# The openehr-sdt and ferroehr tests read the templates and the compositions
# straight out of these Arazzo documents, so the fixtures are the catalogue's
# own bytes and never a hand copy (.claude/rules/vendored-corpora.md). Bump the
# commit deliberately and re-run the tests that read the tree.
#
# Usage:
#   scripts/vendor/veredictum-adl2-content.sh
set -euo pipefail

REPO="Vernum-Projecten/Veredictum"
TAG="v0.3.3"
COMMIT="bfcb62d0514eb065396f3786a8476de306fd13b3"
DEST="$(cd "$(dirname "$0")/../.." && pwd)/corpus/veredictum/adl2-content"

command -v gh >/dev/null 2>&1 || { echo "gh not found on PATH" >&2; exit 1; }

fetch() {
  local path="$1" out="$2"
  mkdir -p "$(dirname "$out")"
  gh api -H 'Accept: application/vnd.github.raw' \
    "repos/${REPO}/contents/${path}?ref=${COMMIT}" >"$out"
  echo "vendored ${path} @ ${COMMIT}"
}

rm -rf "$DEST"
fetch LICENSE "$DEST/LICENSE"
fetch NOTICE "$DEST/NOTICE"
fetch catalogue/cases/_setup/content_templates_adl2.arazzo.yaml \
  "$DEST/_setup/content_templates_adl2.arazzo.yaml"
count=0
while IFS= read -r name; do
  fetch "catalogue/cases/content/${name}" "$DEST/content/${name}"
  count=$((count + 1))
done < <(gh api "repos/${REPO}/contents/catalogue/cases/content?ref=${COMMIT}" \
  --jq '.[] | select(.type == "file") | .name | select(startswith("adl2_"))')
if (( count < 26 )); then
  echo "expected at least 26 content/adl2_* cases, fetched ${count}" >&2
  exit 1
fi

cat >"$DEST/PROVENANCE.md" <<MD
# Provenance

- Upstream: https://github.com/${REPO}
- Ref: tag \`${TAG}\`, commit \`${COMMIT}\`
- Files: \`catalogue/cases/_setup/content_templates_adl2.arazzo.yaml\` (the ADL 2
  operational templates, one upload workflow each) and the ${count}
  \`catalogue/cases/content/adl2_*.arazzo.yaml\` documents (each commit step's
  request body is a composition against one of those templates), with the
  upstream \`LICENSE\` and \`NOTICE\`
- Licence: Apache-2.0 (the upstream \`LICENSE\`, with its \`NOTICE\` carried per
  its section 4(d)); \`REUSE.toml\` declares this subtree under it
- Fetched by: \`scripts/vendor/veredictum-adl2-content.sh\`; never hand-edit,
  re-run the script to update (.claude/rules/vendored-corpora.md)
- Consumers: \`crates/openehr-sdt/tests/it/adl2_content_constraints.rs\` (the
  archetype-conformance pass over every workflow) and
  \`app/ferroehr/tests/it/adl2_content_commit.rs\` (the canonical, FLAT and
  STRUCTURED commit paths)
MD
echo "wrote $DEST/PROVENANCE.md"
