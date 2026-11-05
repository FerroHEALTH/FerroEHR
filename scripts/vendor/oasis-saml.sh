#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor the OASIS SAML V2.0 Profiles into docs/specs/oasis-saml/, the
# document whose §4.1 Web Browser SSO Profile the identity-provider pages cite
# for a SAML 2.0 identity provider brokered by an authorization server.
#
# The OASIS Standard PDF is pinned by the sha256 of its published bytes. The
# copy is verbatim; change the pin and re-run
# (.claude/rules/vendored-corpora.md).
#
# Usage:
#   scripts/vendor/oasis-saml.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=scripts/vendor/lib/pinned.sh
. "$ROOT/scripts/vendor/lib/pinned.sh"

BASE="https://docs.oasis-open.org/security/saml/v2.0"
DEST="$ROOT/docs/specs/oasis-saml"
RETRIEVED="2026-11-05"

# `path sha256` per file, by its path under the base URL.
PINS=(
  "saml-profiles-2.0-os.pdf 5df9b874551941c7f03cb9270e67ea44f426b44a8fa642b8cf261064cc6f7de2"
)

pinned_require curl shasum

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

pinned_fetch_all "$BASE" "$work/tree" "${PINS[@]}"

cat >"$work/tree/PROVENANCE.md" <<EOF
# Provenance: the OASIS SAML V2.0 Profiles

- Source: <${BASE}/saml-profiles-2.0-os.pdf>, published by the OASIS
  Security Services Technical Committee
- Document: "Profiles for the OASIS Security Assertion Markup Language
  (SAML) V2.0", OASIS Standard, 15 March 2005; §4.1 is the Web Browser SSO
  Profile
- Retrieved: ${RETRIEVED}, checked against the sha256 below
- Upstream licence: Appendix B, Notices, reads, as read ${RETRIEVED}:
  "Copyright © OASIS Open 2005. All Rights Reserved. This document and
  translations of it may be copied and furnished to others, and derivative
  works that comment on or otherwise explain it or assist in its
  implementation may be prepared, copied, published and distributed, in
  whole or in part, without restriction of any kind, provided that the
  above copyright notice and this paragraph are included on all such copies
  and derivative works. However, this document itself may not be modified
  in any way, such as by removing the copyright notice or references to
  OASIS, ..." The OASIS IPR Policy
  (<https://www.oasis-open.org/policies-guidelines/ipr/>, read ${RETRIEVED})
  states the same copyright grant for OASIS documents. The PDF is copied
  unmodified with its notice, so the grant is met. \`REUSE.toml\` declares
  it \`LicenseRef-OASIS-Copyright\`.
- Layout: the file name OASIS publishes, unchanged
- Fetched by: \`scripts/vendor/oasis-saml.sh\`; never hand-edit, re-run the
  script to update (.claude/rules/vendored-corpora.md)

| File | sha256 |
|---|---|
$(pinned_rows "${PINS[@]}")
EOF

pinned_install "$work/tree" "$DEST"
say "${#PINS[@]} file into docs/specs/oasis-saml/"
