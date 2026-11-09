#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor the Dutch code lists the Wabvpz Art. 15f check reads into
# docs/specs/nl-role-codes/: the care provider types (`organization-type`,
# `OrganisatieTypeCodelijst`) of Nictiz's zib 2020 FHIR package, and record the
# UZI role codes (`uzi-rolcode`) of that package and the UZI-register CPS v11.7
# they come from.
#
# The package is CC0-1.0, so Nictiz's own files are copied verbatim. The CPS
# is "Copyright CIBG 2024" with no reproduction without written permission, so
# it and the `uzi-rolcode` CodeSystem that reproduces its codes and titles are
# cache only: written into the git-ignored .vendor-cache/nl-role-codes/ and
# never committed (.claude/rules/vendored-corpora.md).
#
# The published tarball is kept as fetched in that cache, since it carries the
# `uzi-rolcode` CodeSystem, and its members are extracted only into a scratch
# directory. Its manifest, package.json, is never committed: a FHIR package
# manifest has the npm shape, and its `hl7.fhir.r4.core` dependency names a
# malicious npm package (GHSA-qg8x-2hq5-hwf3, #5337). The script reads the
# manifest's `license` instead, and refuses the run unless it is CC0-1.0.
#
# Usage:
#   scripts/vendor/nl-role-codes.sh               the tree and the cache
#   scripts/vendor/nl-role-codes.sh --test-inputs only the cached CodeSystem
#                                                 the Wabvpz tests read
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TEST_INPUTS=false
case "${1:-}" in
  "") ;;
  --test-inputs) TEST_INPUTS=true ;;
  *) echo "usage: $0 [--test-inputs]" >&2; exit 2 ;;
esac
# shellcheck source=scripts/vendor/lib/pinned.sh
. "$ROOT/scripts/vendor/lib/pinned.sh"

PACKAGE="nictiz.fhir.nl.r4.zib2020"
VERSION="0.12.1-beta.1"
PACKAGE_URL="https://packages.fhir.org/${PACKAGE}/${VERSION}"
PACKAGE_SHA256="c4fc5561cd592c8dc7760a5674c2c7998334f6197821096155ce357d2e7a0a8a"
CPS_PAGE="https://www.uziregister.nl/documenten/publicaties/2024/07/29/cps-uzi-register-v11.7"
CPS_BASE="https://www.uziregister.nl/site/binaries/site-content/collections/documents/2024/07/29/cps-uzi-register-v11.7"
DEST="$ROOT/docs/specs/nl-role-codes"
RETRIEVED="2026-11-08"

# The package tarball as published, kept in the cache.
TARBALL="${PACKAGE}-${VERSION}.tgz"
# The manifest, read for its licence and never copied.
MANIFEST_SHA256="b57d7f94a7400c07365959e5b245483acd5c4f2e8bd444e30de9e17e8a18e116"
LICENSE="CC0-1.0"

# `path sha256` per file taken from the package, by its path under package/.
PINS=(
  "CodeSystem-RoleCodeNLZorgaanbiederType-organization-type.json f6b21a531b28dc7feccfb5b1381ab877c831dc4f9bd42dec5154c140b7c0fa1e"
  "ValueSet-OrganisatieTypeCodelijst-2.16.840.1.113883.2.4.3.11.60.40.2.17.2.3--20200901000000.json 838a64beb99897854ffab8882b9fe9d9d204f80ea36320bf907e4e55030b0786"
)

# `path sha256` per cache-only file taken from the package, by its path under
# package/.
CACHED_PINS=(
  "CodeSystem-RoleCodeNLZorgverlenertypen-uzi-rolcode.json 676b3edb095f375a35f12a65f828cd74903f71fe4dfb4e7a1aebe9b58901805e"
)

# `file sha256` per cache-only document, by its name on the UZI-register site.
CACHED=(
  "29-01-24+RK1+CPS+UZI-register+V11.7.pdf dc9889a3d592a14409850d53bda9ddf1c61398e6873d40d78cfa0de47086d4a1"
)
CACHE="$ROOT/.vendor-cache/nl-role-codes"

pinned_require curl shasum tar jq

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/tree"

pinned_cache "$PACKAGE_URL" nl-role-codes "$TARBALL" "$PACKAGE_SHA256"
members=(package/package.json)
for pin in "${PINS[@]}" "${CACHED_PINS[@]}"; do
  members+=("package/${pin% *}")
done
tar -xzf "$CACHE/$TARBALL" -C "$work" "${members[@]}"
# Refuses the run unless package/$1 hashes to $2.
check_member() {
  local got
  got="$(pinned_sha256 "$work/package/$1")"
  [[ "$got" == "$2" ]] || die "package/$1 hashes to $got, the pin records $2"
}
check_member package.json "$MANIFEST_SHA256"
got_license="$(jq -r '.license' "$work/package/package.json")"
[[ "$got_license" == "$LICENSE" ]] || die "package/package.json names the licence $got_license, the script records $LICENSE"
for pin in "${CACHED_PINS[@]}"; do
  path="${pin% *}"
  check_member "$path" "${pin##* }"
  cp "$work/package/$path" "$CACHE/$path"
  say "$path (cache only)"
done
if [[ "$TEST_INPUTS" == true ]]; then
  exit 0
fi
for pin in "${PINS[@]}"; do
  path="${pin% *}"
  check_member "$path" "${pin##* }"
  cp "$work/package/$path" "$work/tree/$path"
  say "$path"
done

for pin in "${CACHED[@]}"; do
  pinned_cache "${CPS_BASE}/${pin% *}" nl-role-codes "${pin% *}" "${pin##* }"
done

cat >"$work/tree/PROVENANCE.md" <<EOF
# Provenance: the Dutch role and organisation type code lists

The Wabvpz Art. 15f check of a federation request under an NL agreement reads
these lists (\`app/ferroehr-rest/src/extensions/access/authn/wabvpz.rs\`).

## Nictiz zib 2020 package (committed)

- Source: <${PACKAGE_URL}>, the FHIR package registry's copy of Nictiz's
  package \`${PACKAGE}\`; every published version of it is a beta, and this is
  the newest on ${RETRIEVED}
- Pin: version \`${VERSION}\`, tarball sha256 \`${PACKAGE_SHA256}\`, each file
  below pinned by its own sha256
- Upstream licence: CC0 1.0 (\`${LICENSE}\`, the \`license\` of the package
  manifest \`package/package.json\`, sha256 \`${MANIFEST_SHA256}\`, which
  the script checks on every run;
  <https://creativecommons.org/publicdomain/zero/1.0/>). The package ships no
  licence file of its own.
- The tarball (\`${TARBALL}\`) is kept as published in the git-ignored
  \`.vendor-cache/nl-role-codes/\`, because it carries the cache-only
  CodeSystem below, and the script extracts its members into a scratch
  directory only. The manifest is not committed: a FHIR package manifest has
  the npm \`package.json\` shape, and its dependency \`hl7.fhir.r4.core\`
  names a malicious npm package (GHSA-qg8x-2hq5-hwf3).
- What is taken: the \`organization-type\` CodeSystem
  (\`http://nictiz.nl/fhir/NamingSystem/organization-type\`, OID
  2.16.840.1.113883.2.4.15.1060) and the \`OrganisatieTypeCodelijst\` ValueSet
  (2.16.840.1.113883.2.4.3.11.60.40.2.17.2.3, 2020-09-01), the care provider
  types of the zib Zorgaanbieder
- Cache only: the \`uzi-rolcode\` CodeSystem
  (\`http://fhir.nl/fhir/NamingSystem/uzi-rolcode\`, version 11.7.0, OID
  2.16.840.1.113883.2.4.15.111). Its codes and titles reproduce Bijlage 3 of
  the UZI-register CPS below, which the package's CC0 cannot license, so the
  script writes it into the git-ignored \`.vendor-cache/nl-role-codes/\`, the
  Wabvpz tests read it there, and this repository carries none of it
- Layout: the file names inside the package's \`package/\` directory,
  unchanged

## UZI-register CPS v11.7 (cache only)

- Source: the UZI-register publication page <${CPS_PAGE}>, downloaded from
  <${CPS_BASE}/${CACHED[0]% *}>
- Document: Certificate Practice Statement UZI-register, version 11.7,
  23-01-2024; Bijlage 3 "Beroepstitels, opleidingstitels en specialismen"
  lists the role codes, 01.008 Bedrijfsarts and 01.048 Verzekeringsarts among
  them
- Retrieved: ${RETRIEVED}, checked against the sha256 below
- Upstream licence: none. The document reads "Copyright CIBG 2024© te Den
  Haag" and forbids reproduction or publication without CIBG's prior written
  permission, so the script fetches it into the git-ignored
  \`.vendor-cache/nl-role-codes/\` and this repository carries none of its
  content.

- Fetched by: \`scripts/vendor/nl-role-codes.sh\`; never hand-edit, re-run the
  script to update (.claude/rules/vendored-corpora.md)

| File | sha256 |
|---|---|
$(pinned_rows "${PINS[@]}")

| File | sha256 | Status |
|---|---|---|
$(pinned_status_rows "cache only" "$TARBALL $PACKAGE_SHA256" "${CACHED_PINS[@]}" "${CACHED[@]}")
EOF

pinned_install "$work/tree" "$DEST"
say "${#PINS[@]} files vendored in docs/specs/nl-role-codes/, and $((${#CACHED_PINS[@]} + ${#CACHED[@]} + 1)) cache only in .vendor-cache/nl-role-codes/"
