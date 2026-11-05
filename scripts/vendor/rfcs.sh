#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor the IETF RFCs the party binding and provisioning cite into
# docs/specs/rfc/: SCIM 2.0, the identifier and string rules, the HTTP status
# codes and the OAuth 2.0 documents, the text form of each from the RFC Editor.
#
# Each RFC is pinned by the sha256 of its published text. The RFC Editor's
# metadata is read too, and an RFC it reports obsoleted refuses the run,
# because a pin on an obsoleted document is a defect. The copy is verbatim;
# change a pin and re-run (.claude/rules/vendored-corpora.md).
#
# Usage:
#   scripts/vendor/rfcs.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=scripts/vendor/lib/pinned.sh
. "$ROOT/scripts/vendor/lib/pinned.sh"

EDITOR="https://www.rfc-editor.org/rfc"
DEST="$ROOT/docs/specs/rfc"
RETRIEVED="2026-11-05"

# `number sha256` per RFC.
PINS=(
  "6585 f6d55d1b491cd515c35827cf9181753b23b2a68c4df14e56d83dc445b3876e58"
  "6749 f204fc8661d6c92d2ec6e0b54808f961a9ad26e792f57f312d9528335519bd71"
  "7523 ae24f77a8fc4338903c805c6ace38def1f23d40194aea87b123b13c5b3d2d915"
  "7642 4abbf1bf65926800df6604d627ac0e1d8a0efa784ddcb63b5b59ca8cf4a93e56"
  "7643 df799c112a3fa5be3c0fe054c08b1f4eb5d07590c4c8343a014c4537bf638ae3"
  "7644 b9078e016c98b86934a23c80926b74fc8b59885ab2943729ec2493c17488a09b"
  "8141 9e3296eaac5641d356f580696894878dddc525124436d1da7005f0748035d41e"
  "8265 ec987e39b242228e870b8d1df836451e19cdb2bd66b1ac105355adeae36ff32f"
  "8693 358d658014738bf4dc56c8aae2094ce716ede23b6d5a80488a7dc8e8d72da609"
  "9068 68d3ff3910c0f2340fd7b93b430e00053da845b6eebe095ebef0c8bfda5e14c5"
  "9449 3842c58e1f6043389416023b9bb8d765048266024982fbbd90640e05943f4e13"
  "9700 9919d061d40a97886ca866b51c69389b6c81c65cd4b979056c14fdbebfcf622a"
)

pinned_require curl shasum jq

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/rfc"

rows=()
for pin in "${PINS[@]}"; do
  number="${pin% *}"
  sha="${pin##* }"
  pinned_fetch "$EDITOR/rfc$number.txt" "$work/rfc/rfc$number.txt" "$sha"
  curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
    --retry 3 -o "$work/rfc$number.json" "$EDITOR/rfc$number.json" ||
    die "download of the RFC $number metadata failed"
  title="$(jq -r '.title // empty' "$work/rfc$number.json")"
  published="$(jq -r '.pub_date // empty' "$work/rfc$number.json")"
  status="$(jq -r '.status // empty' "$work/rfc$number.json")"
  obsoleted="$(jq -r '(.obsoleted_by // []) | join(", ")' "$work/rfc$number.json")"
  updated="$(jq -r '(.updated_by // []) | join(", ")' "$work/rfc$number.json")"
  [[ -n "$title" && -n "$published" && -n "$status" ]] || die "the RFC $number metadata is incomplete"
  [[ -z "$obsoleted" ]] || die "RFC $number is obsoleted by $obsoleted; move the pin"
  rows+=("| RFC $number | $title | $published | $status | ${updated:-none} | \`rfc$number.txt\` | \`$sha\` |")
  say "RFC $number"
done

cat >"$work/rfc/PROVENANCE.md" <<EOF
# Provenance: IETF RFCs

- Source: the RFC Editor, <${EDITOR}/rfcNNNN.txt>, the text form of each RFC
  as published; an RFC never changes once published, so its number is its
  version
- Retrieved: ${RETRIEVED}
- Pin: the sha256 of each file, in \`scripts/vendor/rfcs.sh\` and the table
  below; the script refuses any other bytes and any RFC the RFC Editor
  reports obsoleted
- Upstream licence: each RFC's Copyright Notice reads "Copyright (c) <year>
  IETF Trust and the persons identified as the document authors. All rights
  reserved." and "This document is subject to BCP 78 and the IETF Trust's
  Legal Provisions Relating to IETF Documents
  (https://trustee.ietf.org/license-info) in effect on the date of
  publication of this document." Section 3.c of the Trust Legal Provisions
  grants "to each person who wishes to exercise such rights, to the greatest
  extent that it is permitted to do so, a non-exclusive, royalty-free,
  worldwide right and license under all copyrights and rights of authors: i.
  to copy, publish, display and distribute IETF Contributions and IETF
  Documents in full and without modification". RFC 6585 and RFC 6749,
  published in 2012, fall under TLP 4.0 (effective 2009-12-28,
  <https://trustee.ietf.org/license-info/IETF-TLP-4.htm>); the others under
  TLP 5.0 (effective 2015-03-25,
  <https://trustee.ietf.org/documents/trust-legal-provisions/tlp-5/>). Both
  carry the grant in the same words, read ${RETRIEVED}. Each file here is
  the whole document, unmodified, with every notice it carries. The
  Revised BSD licence of section 4 covers only Code Components and is not
  relied on. \`LICENSES/LicenseRef-IETF-Trust-Legal-Provisions.txt\` holds
  the same record for REUSE.
- Fetched by: \`scripts/vendor/rfcs.sh\`; never hand-edit, re-run the script
  to update (.claude/rules/vendored-corpora.md)

| Document | Title | Published | Status | Updated by | File | sha256 |
|---|---|---|---|---|---|---|
$(printf '%s\n' "${rows[@]}")

An "Updated by" entry names an RFC that changes the document without
replacing it. The updates are not vendored here; a citation that depends on
one names it and vendors it first.
EOF

pinned_install "$work/rfc" "$DEST"
say "${#PINS[@]} RFCs into docs/specs/rfc/"
