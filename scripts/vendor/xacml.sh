#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor the OASIS eXtensible Access Control Markup Language (XACML) Version
# 2.0 core specification into docs/specs/xacml/, the standard IHE APPC
# Privacy Consent Documents are written in (APPC TF-3 5.6.1), whose policy
# structure and combining algorithms the ABAC mapping cites.
#
# The OASIS Standard PDF is pinned by the sha256 of its published bytes. The
# copy is verbatim; change the pin and re-run
# (.claude/rules/vendored-corpora.md).
#
# Usage:
#   scripts/vendor/xacml.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=scripts/vendor/lib/pinned.sh
. "$ROOT/scripts/vendor/lib/pinned.sh"

BASE="https://docs.oasis-open.org/xacml/2.0"
DEST="$ROOT/docs/specs/xacml"
RETRIEVED="2026-11-13"

# `path sha256` per file, by its path under the base URL.
PINS=(
  "access_control-xacml-2.0-core-spec-os.pdf 7424acfe6dc2406e3efdf88ee83200885ab11951b0a1f721ae3505125213b4d0"
)

pinned_require curl shasum

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

pinned_fetch_all "$BASE" "$work/tree" "${PINS[@]}"

cat >"$work/tree/PROVENANCE.md" <<EOF
# Provenance: the OASIS XACML 2.0 core specification

- Source: <${BASE}/access_control-xacml-2.0-core-spec-os.pdf>, published
  by the OASIS Access Control (XACML) Technical Committee; the PDF names
  the same path over \`http\` as its Location
- Document: "eXtensible Access Control Markup Language (XACML) Version
  2.0", OASIS Standard, 1 Feb 2005, document identifier
  \`oasis-access_control-xacml-2.0-core-spec-os\`, editor Tim Moses. The
  ABAC mapping of IHE APPC cites §5.1 (\`<PolicySet>\`), §5.22
  (\`<Policy>\`), §5.29 (\`<Rule>\`), §5.30 (\`EffectType\`), §10.2.3
  (the mandatory algorithms), §B.10 (the combining algorithm identifiers)
  and Appendix C (the combining algorithms)
- Retrieved: ${RETRIEVED}, checked against the sha256 below
- Upstream licence: Appendix E, Notices, reads, as read ${RETRIEVED}:
  "Copyright © OASIS Open 2004-2005. All Rights Reserved. This document and
  translations of it may be copied and furnished to others, and derivative
  works that comment on or otherwise explain it or assist in its
  implementation may be prepared, copied, published and distributed, in
  whole or in part, without restriction of any kind, provided that the
  above copyright notice and this paragraph are included on all such copies
  and derivative works. However, this document itself does not be modified
  in any way, such as by removing the copyright notice or references to
  OASIS, ... The limited permissions granted above are perpetual and will
  not be revoked by OASIS or its successors or assigns." The OASIS IPR
  Policy (<https://www.oasis-open.org/policies-guidelines/ipr/>, read
  ${RETRIEVED}) requires the same notice on every OASIS Deliverable
  (section 14.1). The PDF is copied unmodified with its notice, so the
  grant is met. \`REUSE.toml\` declares it \`LicenseRef-OASIS-Copyright\`.
- Patents: the same appendix states that OASIS "takes no position regarding
  the validity or scope of any intellectual property or other rights that
  might be claimed to pertain to the implementation or use of the
  technology described in this document", and the Status section refers to
  the Intellectual Property Rights section of the Access Control TC web page
  for disclosed claims and licensing offers. The copyright grant above
  covers the text only.
- What is taken: the core specification alone. The XML schemas, the
  profiles and the errata published beside it under \`${BASE}/\` are not
  taken; the schema fragments the mapping cites are quoted in the core
  specification itself.
- Layout: the file name OASIS publishes, unchanged
- Fetched by: \`scripts/vendor/xacml.sh\`; never hand-edit, re-run the
  script to update (.claude/rules/vendored-corpora.md)

| File | sha256 |
|---|---|
$(pinned_rows "${PINS[@]}")
EOF

pinned_install "$work/tree" "$DEST"
say "${#PINS[@]} file into docs/specs/xacml/"
