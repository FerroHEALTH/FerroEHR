#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor OpenID Connect Core 1.0 into docs/specs/openid-connect-core/: the
# errata 2 edition at its own immutable URL, which the party binding cites for
# the standard claims (§5.1) and the claim stability rules (§5.7).
#
# The script also reads the unversioned URL the OpenID Foundation serves the
# current edition at, and refuses the run when it no longer carries the same
# bytes, because that is a new errata set to re-pin. The copy is verbatim;
# change the pin and re-run (.claude/rules/vendored-corpora.md).
#
# Usage:
#   scripts/vendor/openid-connect-core.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=scripts/vendor/lib/pinned.sh
. "$ROOT/scripts/vendor/lib/pinned.sh"

SITE="https://openid.net/specs"
FILE="openid-connect-core-1_0-errata2.html"
CURRENT="openid-connect-core-1_0.html"
SHA256="4d016752a645e8a3c259baa4173b5a90cb6c82898ce868fc8444adde262a5aed"
DEST="$ROOT/docs/specs/openid-connect-core"
RETRIEVED="2026-11-05"

pinned_require curl shasum cmp

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/tree"

pinned_fetch "$SITE/$FILE" "$work/tree/$FILE" "$SHA256"
curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
  --retry 3 -o "$work/$CURRENT" "$SITE/$CURRENT" || die "download of $CURRENT failed"
cmp -s "$work/tree/$FILE" "$work/$CURRENT" ||
  die "$CURRENT no longer carries the bytes of $FILE: a newer edition is published, re-pin"

cat >"$work/tree/PROVENANCE.md" <<EOF
# Provenance: OpenID Connect Core 1.0

- Source: <${SITE}/${FILE}>, the edition's own immutable URL
- Version: OpenID Connect Core 1.0 incorporating errata set 2, dated
  December 15, 2023; the unversioned <${SITE}/${CURRENT}> served the same
  bytes when retrieved (the script checks the two are identical)
- Retrieved: ${RETRIEVED}
- sha256: \`${SHA256}\`
- Upstream licence: the specification's Appendix C, Notices, read
  ${RETRIEVED}: "Copyright (c) 2023 The OpenID Foundation. The OpenID
  Foundation (OIDF) grants to any Contributor, developer, implementer, or
  other interested party a non-exclusive, royalty free, worldwide copyright
  license to reproduce, prepare derivative works from, distribute, perform
  and display, this Implementers Draft or Final Specification solely for the
  purposes of (i) developing specifications, and (ii) implementing
  Implementers Drafts and Final Specifications based on such documents,
  provided that attribution be made to the OIDF as the source of the
  material, but that such attribution does not indicate an endorsement by the
  OIDF." The copy is distributed here to implement the specification, and
  this file is the attribution: the material is the OpenID Foundation's.
  FerroAUTH's record of the same edition states the same notice.
  \`LICENSES/LicenseRef-OpenID-Foundation-Specification.txt\` holds the
  same record for REUSE.
- Fetched by: \`scripts/vendor/openid-connect-core.sh\`; never hand-edit,
  re-run the script to update (.claude/rules/vendored-corpora.md)

| File | sha256 |
|---|---|
| \`${FILE}\` | \`${SHA256}\` |
EOF

pinned_install "$work/tree" "$DEST"
say "$FILE into docs/specs/openid-connect-core/"
