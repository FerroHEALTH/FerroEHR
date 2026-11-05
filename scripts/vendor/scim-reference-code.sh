#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor Microsoft's "Workload Identity Federation for SCIM Provisioning" guide
# from the AzureAD/SCIMReferenceCode repository into
# docs/specs/scim-reference-code/, with the figures it references and the
# repository's LICENSE. The SCIM endpoint's Entra authentication path and the
# Entra walk-through cite it for the client assertion Entra presents.
#
# The source is pinned to a commit, each file checked against its sha256. The
# copy is verbatim; move the pin and re-run (.claude/rules/vendored-corpora.md).
#
# Usage:
#   scripts/vendor/scim-reference-code.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=scripts/vendor/lib/pinned.sh
. "$ROOT/scripts/vendor/lib/pinned.sh"

REPO="AzureAD/SCIMReferenceCode"
COMMIT="21b92c8f28383e30f8c304908b126682af2370d7"
COMMIT_DATE="2026-10-26T18:43:38Z"
GUIDE_COMMIT="a209a8e2bc1af22a6859721d92560ae5e9b977cf"
GUIDE_DATE="2026-10-01T03:06:05Z"
BASE="https://raw.githubusercontent.com/${REPO}/${COMMIT}"
DEST="$ROOT/docs/specs/scim-reference-code"
RETRIEVED="2026-11-05"
GUIDE="Workload-Identity-Federation-for-SCIM-Provisioning.md"
MEDIA="media/workload-identity-federation"

# `path sha256` per file, by its path in the repository.
PINS=(
  "LICENSE 9906940f61b1f0b533fa7d99baf55178b2808fbe113ea51dfbfad8572ccd5f2b"
  "$GUIDE f60c5208637503ceed38a6a49aea11a6cee90049dafa8ad81e09676198d89d0f"
  "$MEDIA/complete-entra-connectivity-settings.png 2a9988fb61dff1c3b998eaeaafa9090a080ecdacd6503a069443e7526f70bac6"
  "$MEDIA/copy-wif-claims-to-isv.png 0baec141aaf1de8fec9d3598b5a500844a793b46d135bb3d6700ef9cfde99583"
  "$MEDIA/register-new-workload-identity.png 84e6ac8cbedfbd5c9d76df6457ebbc28a5678069c46129590ff3bf4d7bfdd70e"
  "$MEDIA/select-existing-workload-identity.jpg 37743d3d5580f8083ab21be24286bccb72de2560691b266646ab2072822550bf"
  "$MEDIA/switch-to-workload-identity-authentication.png 8ea7bd817d2e45922f6f0921fa829c6141aa4078bf1b4b72e62e33c52de450f9"
  "$MEDIA/wif-configuration-flow.png f64c81ad3ef020f6a2a41cbc42f54ed26f40e0d8e8161b018e973dcd26d209b7"
)

pinned_require curl shasum

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

pinned_fetch_all "$BASE" "$work/tree" "${PINS[@]}"

cat >"$work/tree/PROVENANCE.md" <<EOF
# Provenance: Workload Identity Federation for SCIM Provisioning

- Source: <https://github.com/${REPO}>, the file \`${GUIDE}\` with the figures
  it references under \`${MEDIA}/\` and the repository's \`LICENSE\`
- Pin: commit \`${COMMIT}\` (${COMMIT_DATE}), the default branch's head when
  retrieved, each file checked against the sha256 below; the newest commit
  touching the guide at the pin is \`${GUIDE_COMMIT}\` (${GUIDE_DATE})
- Retrieved: ${RETRIEVED}
- Upstream licence: the repository's \`LICENSE\`, vendored beside this file
  and read ${RETRIEVED}, is the MIT License: "Copyright (c) Microsoft
  Corporation. Permission is hereby granted, free of charge, to any person
  obtaining a copy of this software and associated documentation files (the
  "Software"), to deal in the Software without restriction, including
  without limitation the rights to use, copy, modify, merge, publish,
  distribute, sublicense, and/or sell copies of the Software ... The above
  copyright notice and this permission notice shall be included in all
  copies or substantial portions of the Software." The guide is a
  documentation file of that repository, and the notice travels with it here.
  The repository states no other licence for its documentation.
- Layout: the upstream paths, unchanged
- Fetched by: \`scripts/vendor/scim-reference-code.sh\`; never hand-edit,
  re-run the script to update (.claude/rules/vendored-corpora.md)

| File | sha256 |
|---|---|
$(pinned_rows "${PINS[@]}")
EOF

pinned_install "$work/tree" "$DEST"
say "${#PINS[@]} files into docs/specs/scim-reference-code/"
