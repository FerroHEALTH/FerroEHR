#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor the two Microsoft Entra ID provisioning articles the SCIM endpoint and
# the Entra walk-through cite into docs/specs/microsoft-entra/: "Develop a SCIM
# endpoint for user provisioning to apps from Microsoft Entra ID" and
# "Customize Microsoft Entra attribute mappings in Application Provisioning",
# with the figures they reference and the repository's licence files.
#
# The source is the MicrosoftDocs/entra-docs repository at a pinned commit,
# each file checked against its sha256. The copy is verbatim; move the pin
# and re-run (.claude/rules/vendored-corpora.md).
#
# Usage:
#   scripts/vendor/entra-scim-docs.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=scripts/vendor/lib/pinned.sh
. "$ROOT/scripts/vendor/lib/pinned.sh"

REPO="MicrosoftDocs/entra-docs"
COMMIT="219d8d57b01779e72e6d5449dc9341334c1fb906"
COMMIT_DATE="2026-11-05T13:36:57Z"
BASE="https://raw.githubusercontent.com/${REPO}/${COMMIT}"
DEST="$ROOT/docs/specs/microsoft-entra"
RETRIEVED="2026-11-05"
ARTICLES="docs/identity/app-provisioning"
MEDIA="$ARTICLES/media"

# `path sha256` per file, by its path in the repository.
PINS=(
  "LICENSE c2cfccb812fe482101a8f04597dfc5a9991a6b2748266c47ac91b6a5aae15383"
  "LICENSE-CODE 91f7258203fd43ba6d6f7f72b644eaedee9e3430f035c053beb6f34052f045e2"
  "README.md 32355274e726eb8981b4478aef7917313395ff7ca7be1904488c4adf8b818584"
  "$ARTICLES/use-scim-to-provision-users-and-groups.md c413a8dba21bfa7392f571bd9ebd1a42c569a834f4961339648aae5654f6e6b4"
  "$ARTICLES/customize-application-attributes.md 326616d9ae71fe04965bb32b34601532b3662adeb1b41e970e94156776d2eb0e"
  "$MEDIA/customize-application-attributes/add-roles.png d2a0a68da5374a1a42a152a07f70a099bafb33795ea7fe2337ce25dfea5c9065"
  "$MEDIA/customize-application-attributes/edit-attribute-approleassignmentscomplex.png e5b15c34d7d19f44e4b872bee7b6bfef257bb0c31cea1e670a198b9d42111ce2"
  "$MEDIA/customize-application-attributes/edit-attribute-assertiveapproleassignmentscomplex.png b1b2e35cf6d16ccdebabc329ea8290b70a1d3c3c3d7be4ee09c4be27a7ef97a4"
  "$MEDIA/customize-application-attributes/edit-attribute-singleapproleassignment.png c094c26b729343f17bf758c960602446837685108cf2d9393b023f50f32d658e"
  "$MEDIA/use-scim-to-provision-users-and-groups/oauth2.png e6c3869619792884b5dc703f1acab0b44cecb38f8143d3829cbe25e83f2f778b"
  "$MEDIA/use-scim-to-provision-users-and-groups/process.png c2c336d45ed5f6b664f6fb355bbf080a8396ec08351005afaf6da3ee5d5f9d55"
  "$MEDIA/use-scim-to-provision-users-and-groups/scim-figure-2b-1.png a28d4e6234cd04b1c5dfa5b91e6dcf37926bae92704ffbc4bd0e25066bfa3cc7"
  "$MEDIA/use-scim-to-provision-users-and-groups/scim-figure-3.png 8827ba1646381ce95f8fef3c8642c25c417da67a54fd4ffa447611181dd4a825"
  "$MEDIA/use-scim-to-provision-users-and-groups/scim-figure-4.png a12bbbe83270e8f10d906ddbbc1a7dd341e231e46815af3d58d86d14095a4bd0"
  "$MEDIA/use-scim-to-provision-users-and-groups/scim-figure-5.png aa4ddedc8a9d2d6bb4ceeec64620f9b5f35a98386f13292c47de76f01ca923f7"
  "$MEDIA/use-scim-to-provision-users-and-groups/scim-provisioning-overview.png 2d307f01860e79438addfaf6b1c072f7cd66c5e62a50246491d59c579360ff49"
)

pinned_require curl shasum

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

pinned_fetch_all "$BASE" "$work/tree" "${PINS[@]}"

cat >"$work/tree/PROVENANCE.md" <<EOF
# Provenance: Microsoft Entra ID provisioning articles

- Source: <https://github.com/${REPO}>, the repository Microsoft Learn
  publishes the Entra documentation from
- Pin: commit \`${COMMIT}\` (${COMMIT_DATE}), the default branch's head when
  retrieved, each file checked against the sha256 below
- Retrieved: ${RETRIEVED}
- Articles: \`${ARTICLES}/use-scim-to-provision-users-and-groups.md\`
  ("Tutorial - Develop a SCIM endpoint for user provisioning to apps from
  Microsoft Entra ID", \`ms.date\` 09/15/2026) and
  \`${ARTICLES}/customize-application-attributes.md\` ("Tutorial - Customize
  Microsoft Entra attribute mappings in Application Provisioning",
  \`ms.date\` 08/06/2026), with every figure they reference
- Upstream licence: the repository states two grants, both recorded here as
  read ${RETRIEVED}. The \`README.md\` "Legal Notices" section reads:
  "Microsoft and any contributors grant you a license to the Microsoft
  documentation and other content in this repository under the Creative
  Commons Attribution 4.0 International Public License, see the LICENSE
  file, and grant you a license to any code in the repository under the MIT
  License, see the LICENSE-CODE file." The \`LICENSE\` file it points at
  reads the MIT License ("Copyright (c) Microsoft Corporation. Permission is
  hereby granted, free of charge, to any person obtaining a copy of this
  software and associated documentation files ..."), as does
  \`LICENSE-CODE\`. Both grants permit redistribution with the notice kept;
  the three files are vendored beside the articles, and the attribution is
  this file: the material is Microsoft Corporation's. \`REUSE.toml\`
  declares the tree \`CC-BY-4.0 OR MIT\`, one expression for the two grants.
- Layout: the upstream paths, unchanged
- Fetched by: \`scripts/vendor/entra-scim-docs.sh\`; never hand-edit,
  re-run the script to update (.claude/rules/vendored-corpora.md)

| File | sha256 |
|---|---|
$(pinned_rows "${PINS[@]}")
EOF

pinned_install "$work/tree" "$DEST"
say "${#PINS[@]} files into docs/specs/microsoft-entra/"
