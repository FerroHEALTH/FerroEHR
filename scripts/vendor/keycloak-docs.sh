#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor the Keycloak 26 documentation sections the book's Keycloak example
# cites into docs/specs/keycloak/: standard and legacy token exchange, the
# audience mapper and service accounts, user registration, impersonation and
# the user profile, SAML identity brokering and the first login flow, the SCIM
# chapter, and the release notes that introduce and promote the SCIM API.
#
# The source is the AsciiDoc in the keycloak/keycloak repository at a release
# tag, resolved to its commit, each file checked against its sha256. Text
# only; the figures are not taken. The copy is verbatim; move the pin and
# re-run (.claude/rules/vendored-corpora.md).
#
# Usage:
#   scripts/vendor/keycloak-docs.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=scripts/vendor/lib/pinned.sh
. "$ROOT/scripts/vendor/lib/pinned.sh"

REPO="keycloak/keycloak"
TAG="26.8.2"
COMMIT="e514445f9dcdabc5bdcf67cab56c1e49655de067"
COMMIT_DATE="2026-11-05T15:27:24Z"
BASE="https://raw.githubusercontent.com/${REPO}/${COMMIT}"
DEST="$ROOT/docs/specs/keycloak"
RETRIEVED="2026-11-05"
ADMIN="docs/documentation/server_admin/topics"
NOTES="docs/documentation/release_notes/topics"

# `path sha256` per file, by its path in the repository.
PINS=(
  "LICENSE.txt cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30"
  "$ADMIN/License.adoc 68a5711624474f921c0e518ff493e3f0d5aad6bf75f51831d5857e706fa90216"
  "$NOTES/26_7_0.adoc 6415550f0c047d5d4400b991680e06d1075d0f532dd6e6e6713ce9d1d6b297ec"
  "$NOTES/26_8_0.adoc 651b61584ac2e9d92aca48211af99c8b4040a4d7b03d803d774a3bd731665051"
  "docs/guides/securing-apps/token-exchange.adoc 99d6d660675f2d4d35cbdc0825ffab7f5aeeb39d970c580340b1d472a72d7afa"
  "$ADMIN/assembly-managing-scim.adoc 44adf4d081429aaae0b4874380e52bfd7900e906e8b8ef903dce2269f8d39474"
  "$ADMIN/scim/accessing-endpoints.adoc 16d240d0a6731e9b4f139b54e1e817b41f69ffa3dc3e07abb22b6e127ff7d257"
  "$ADMIN/scim/accessing-service-config.adoc 872c5af7242095431e023527fa23b0f9648b319129942f651c079386a075a1df"
  "$ADMIN/scim/admin-resource-protection.adoc 19d5b1b420b2f7cd87a2b8f43d4e52d93c2e7d49d00a617ab312f6182f17501d"
  "$ADMIN/scim/enabling-scim.adoc e087c0c2c0891eaa2086701ff4abb48e63a8f4e40996b57d8837adab2ad411a9"
  "$ADMIN/scim/filtering-resources.adoc 03b963c5d958a7bfa585122e8173bdf173489fac3e9cb4c20512a128e3129644"
  "$ADMIN/scim/intro.adoc 9f25f9f1e1f948965c81910af57fb2a11e4bc9b44bb2e360a9e5141227d0b830"
  "$ADMIN/scim/managing-groups.adoc c2692720465aa7bdc9acb042c64e7cacb46ebe0c0e5f088e21af9f520ce47a44"
  "$ADMIN/scim/managing-schema-extensions.adoc bf16a9ca9547cd5d4b70b7a120ce0d8b4eb4d0aafaf87f5f4801c17149eae8e7"
  "$ADMIN/scim/managing-users.adoc c1b90f9a199ac1606c36de3c8784f756496e6420e2407c0ff794859f37450e57"
  "$ADMIN/scim/mapping-attributes.adoc 8a4ab3277a849594d047b4a98fa3bf92ebbae76c0edfd2424a312877d7f20f81"
  "$ADMIN/scim/understanding-scim-endpoints.adoc 2400036fbc41fc7f066285346ce854fe5c677de0395bdb139d7ec75e4f0ad7e2"
  "$ADMIN/users/user-profile.adoc b843b85be43466a960a1096d2fb7b81ddf473791f94a1c551b93d5b9ac502ed2"
  "$ADMIN/users/con-user-registration.adoc b6c998bf0bb9bfbcf7d9dd29b9acc6f3ad21c00cdf679a98bbf28a4b9cc6da95"
  "$ADMIN/users/proc-enabling-user-registration.adoc e4f1e2d74ea2708dbb237eab8a82c3805ccf827c5d80b098cb349a356797694e"
  "$ADMIN/users/con-user-impersonation.adoc 261e7a7207910f9759c1b57d57b89019763eb3400d7d822195e8a2774fbe9e34"
  "$ADMIN/identity-broker/overview.adoc 3aacdef256bbd82cc9964512a6353cc1f97541299b2e6e02e543246e3dc56356"
  "$ADMIN/identity-broker/saml.adoc 74d027736077bee4ba103c7ba3a476a3a016347ea1aa8bda2262e8189efe97f2"
  "$ADMIN/identity-broker/first-login-flow.adoc 1e7d08c9e4d1373b92f3cf8eae1d087760fb0603b6d2411caa30d72d18fd6c3a"
  "$ADMIN/identity-broker/mappers.adoc dc40786affd3dad049a6ab116317ad5dcbd536db3a4a5791e827874ede9247e9"
  "$ADMIN/clients/oidc/con-audience.adoc ff9f6c0adf79a2b16f94daac4596f1ed0aa67bae1bf4da6e55703df818882225"
  "$ADMIN/clients/oidc/proc-using-a-service-account.adoc 0ab4e3d6fc78b640abdfd8086af1e791d4ed89750a84211bd06ee8e7b24709ed"
)

pinned_require curl shasum

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

pinned_fetch_all "$BASE" "$work/tree" "${PINS[@]}"

cat >"$work/tree/PROVENANCE.md" <<EOF
# Provenance: Keycloak ${TAG} documentation (excerpt)

- Source: <https://github.com/${REPO}>, the AsciiDoc sources of the Server
  Administration Guide (\`${ADMIN}/\`), the Securing Applications guide
  (\`docs/guides/securing-apps/\`) and the release notes (\`${NOTES}/\`)
- Version: Keycloak ${TAG}, the newest 26.x release when retrieved; tag
  \`${TAG}\` resolves to commit \`${COMMIT}\` (${COMMIT_DATE}), which the
  script fetches, each file checked against the sha256 below
- Retrieved: ${RETRIEVED}
- Upstream licence: Apache License 2.0, the repository's \`LICENSE.txt\`,
  vendored beside this file. The guide's own licence chapter,
  \`${ADMIN}/License.adoc\`, also vendored, reads: "Keycloak codebase is
  distributed under the ASL 2.0 license." Read ${RETRIEVED}.
- Layout: the upstream paths, unchanged. Text only: the figures the sections
  reference under \`docs/documentation/server_admin/images/\` and
  \`docs/guides/images/\` are not taken; fetch them from the repository at
  the pinned commit
- Fetched by: \`scripts/vendor/keycloak-docs.sh\`; never hand-edit, re-run
  the script to update (.claude/rules/vendored-corpora.md)

| File | sha256 |
|---|---|
$(pinned_rows "${PINS[@]}")
EOF

pinned_install "$work/tree" "$DEST"
say "${#PINS[@]} files into docs/specs/keycloak/"
