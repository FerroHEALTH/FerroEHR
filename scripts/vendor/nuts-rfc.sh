#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor Nuts RFC021, VP Token Grant Type, into docs/specs/nuts-rfc/: the
# OAuth 2.0 grant a Nuts participant sends with a Verifiable Presentation,
# which the party binding's delegated callers cite.
#
# The source is nuts-foundation/nuts-specification at a pinned commit, the
# file checked against its sha256. The copy is verbatim; move the pin and
# re-run (.claude/rules/vendored-corpora.md).
#
# Usage:
#   scripts/vendor/nuts-rfc.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=scripts/vendor/lib/pinned.sh
. "$ROOT/scripts/vendor/lib/pinned.sh"

REPO="nuts-foundation/nuts-specification"
COMMIT="7c0de533b8cce537812b6647542e5719c93433a8"
BASE="https://raw.githubusercontent.com/${REPO}/${COMMIT}"
DEST="$ROOT/docs/specs/nuts-rfc"
RETRIEVED="2026-11-05"

# `path sha256` per file, by its path in the repository.
PINS=(
  "rfc/rfc021-vp_token-grant-type.md b21c77f293f5dfc9b282dd78e3c17a7e3fb4e2dea21f259ffa754730a1045fe8"
)

pinned_require curl shasum

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

pinned_fetch_all "$BASE" "$work/tree" "${PINS[@]}"

cat >"$work/tree/PROVENANCE.md" <<EOF
# Provenance: Nuts RFC021

- Source: <https://github.com/${REPO}>
- Pin: commit \`${COMMIT}\`, the file checked against the sha256 below, the
  same bytes as FerroHEALTH/FerroFED vendors in \`docs/specs/nuts-rfc/\`
- Document: *RFC021 VP Token Grant Type*, Nuts foundation Request for
  Comments 021, W.M. Slakhorst (Nedap), September 2023, status "currently in
  draft"
- Retrieved: ${RETRIEVED}
- Upstream licence: read again for this copy on ${RETRIEVED}. The
  repository has no licence file. The document states its licence in its
  Copyright Notice: "This document is released under the
  Attribution-ShareAlike 4.0 International (CC BY-SA 4.0) license"
  (<https://creativecommons.org/licenses/by-sa/4.0/legalcode>). The file is
  redistributed unmodified with that notice, so no adaptation is made, and
  the attribution is this file: the document is the Nuts foundation's.
  \`REUSE.toml\` declares the tree \`CC-BY-SA-4.0\`.
- What is taken: RFC021 alone. The licence badge it links
  (\`../.gitbook/assets/license.png\`) is not taken, so that one image link
  does not resolve here; the notice text beside it is the licence.
- Layout: the upstream paths, unchanged
- Fetched by: \`scripts/vendor/nuts-rfc.sh\`; never hand-edit, re-run the
  script to update (.claude/rules/vendored-corpora.md)

| File | sha256 |
|---|---|
$(pinned_rows "${PINS[@]}")
EOF

pinned_install "$work/tree" "$DEST"
say "${#PINS[@]} file into docs/specs/nuts-rfc/"
