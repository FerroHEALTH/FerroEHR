#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Record the Independent Submission RFCs the openEHR access log design cites
# in docs/specs/rfc-independent/: RFC 3881, the healthcare audit message the
# DICOM and IHE audit records descend from, and RFC 8785, the JSON
# canonicalization scheme the design's hash chain names.
#
# The IETF Trust's grant to copy and distribute a document in full (Trust
# Legal Provisions 5.0, section 3.c) does not apply to the Independent
# Submission Stream (section 8.c.iii), so these are cache only: the script
# fetches each into the git-ignored .vendor-cache/rfc-independent/, refuses it
# unless it hashes to its pin, checks the RFC Editor's metadata, and writes the
# record (.claude/rules/vendored-corpora.md).
#
# Usage:
#   scripts/vendor/rfcs-independent.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=scripts/vendor/lib/pinned.sh
. "$ROOT/scripts/vendor/lib/pinned.sh"

EDITOR="https://www.rfc-editor.org/rfc"
DEST="$ROOT/docs/specs/rfc-independent"
RETRIEVED="2026-11-07"

# `file sha256` per cache-only RFC.
CACHED=(
  "rfc3881.txt 3dea919cf0b565890a47089560a2f22dbbb9ba1af34442c56cee9372fd7b4308"
  "rfc8785.txt 63d52294eb0e3f0014174288186d388b4ddbf2c67d1ce8af1d9726eb0c3ab240"
)

pinned_require curl shasum jq

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/tree"

rows=()
for pin in "${CACHED[@]}"; do
  file="${pin% *}"
  number="${file#rfc}"
  number="${number%.txt}"
  pinned_cache "$EDITOR/$file" rfc-independent "$file" "${pin##* }"
  curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
    --retry 3 -o "$work/rfc$number.json" "$EDITOR/rfc$number.json" ||
    die "download of the RFC $number metadata failed"
  title="$(jq -r '.title // empty' "$work/rfc$number.json")"
  published="$(jq -r '.pub_date // empty' "$work/rfc$number.json")"
  stream="$(jq -r '.source // empty' "$work/rfc$number.json")"
  obsoleted="$(jq -r '(.obsoleted_by // []) | join(", ")' "$work/rfc$number.json")"
  [[ "$stream" == "INDEPENDENT" ]] ||
    die "RFC $number reports stream '$stream'; it may belong in scripts/vendor/rfcs.sh"
  [[ -z "$obsoleted" ]] || die "RFC $number is obsoleted by $obsoleted; move the record"
  rows+=("| RFC $number | $title | $published |")
done

cat >"$work/tree/PROVENANCE.md" <<EOF
# Provenance: Independent Submission RFCs, cache only

**This directory holds this record and nothing else.** The documents are read
from the cache and never committed.

- Source: the RFC Editor, <${EDITOR}/rfcNNNN.txt>
- Retrieved: ${RETRIEVED}, each checked against the sha256 below; the script
  refuses an RFC the RFC Editor reports obsoleted or moved out of the
  Independent Submission Stream

| Document | Title | Published |
|---|---|---|
$(printf '%s\n' "${rows[@]}")

- Upstream licence: RFC 3881 reads "Copyright (C) The Internet Society
  (2004). This document is subject to the rights, licenses and restrictions
  contained in BCP 78"; RFC 8785 is "subject to BCP 78 and the IETF Trust's
  Legal Provisions Relating to IETF Documents". The Trust Legal Provisions 5.0
  grant third parties the right to copy and distribute IETF Documents in full
  (section 3.c.i) and state that section 3.c does not apply to the Alternate
  Streams, the Independent Submission Stream among them (section 8.c.iii),
  read ${RETRIEVED} at
  <https://trustee.ietf.org/documents/trust-legal-provisions/tlp-5/>. With no
  grant that reaches these documents, the script fetches them into the
  git-ignored \`.vendor-cache/rfc-independent/\` and this repository carries
  none of their content.
- Fetched by: \`scripts/vendor/rfcs-independent.sh\`; never add a file here,
  re-run the script to update (.claude/rules/vendored-corpora.md)

| File | sha256 | Status |
|---|---|---|
$(pinned_status_rows "cache only" "${CACHED[@]}")
EOF

pinned_install "$work/tree" "$DEST"
say "${#CACHED[@]} RFCs cache only, recorded in docs/specs/rfc-independent/"
