#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor the Okta documents the `okta` issuer preset cites into
# docs/specs/okta/: the OAuth 2.0 and the management OpenAPI documents of
# okta/okta-management-openapi-spec at the newest release in its dist/, with
# the README that states their licence, and the "OpenID Connect & OAuth 2.0
# API" overview page of developer.okta.com, cache only.
#
# The repository files are pinned by commit and sha256 and copied verbatim.
# The overview page grants no licence to copy it, so it is fetched into the
# git-ignored .vendor-cache/okta/ and recorded by URL and sha256 only
# (.claude/rules/vendored-corpora.md).
#
# Usage:
#   scripts/vendor/okta.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=scripts/vendor/lib/pinned.sh
. "$ROOT/scripts/vendor/lib/pinned.sh"

REPO="okta/okta-management-openapi-spec"
COMMIT="ec20a8e9f2f204912d9b0f03a866ea5d181e69d5"
COMMIT_DATE="2026-11-05T05:34:03Z"
RELEASE="2026.44.0"
BASE="https://raw.githubusercontent.com/${REPO}/${COMMIT}"
OVERVIEW="https://developer.okta.com/docs/api/openapi/okta-oauth/guides/overview/"
DEST="$ROOT/docs/specs/okta"
RETRIEVED="2026-11-05"

# `path sha256` per committed file, by its path in the repository.
PINS=(
  "README.md 84b5118d547c6d3e88feb9a338630aa94dfd206d33f8d32e8a9f1994927ba8ab"
  "dist/${RELEASE}/oauth-minimal.yaml 36f06d3adcba39bd36f9bfd3a04f137b0c8980d6116707616281a936101b0a49"
  "dist/${RELEASE}/management-minimal.yaml 0e1ccf7a1c12f4e77e2e5cebf2d625f8869f7603746360a54e0360673644049f"
)
# `file sha256` per cache-only document.
CACHED=(
  "okta-oauth-overview.html c278d86c510d60d27ffe073a068d6ac8763358749a6017f449e9bc21f0d39ea9"
)

pinned_require curl shasum

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

pinned_fetch_all "$BASE" "$work/tree" "${PINS[@]}"
pinned_cache "$OVERVIEW" okta "${CACHED[0]% *}" "${CACHED[0]##* }"

cat >"$work/tree/PROVENANCE.md" <<EOF
# Provenance: the Okta OpenAPI documents and the OAuth 2.0 API overview

- Source: <https://github.com/${REPO}>, the OpenAPI documents Okta builds
  its management SDKs from, one directory per Okta release under \`dist/\`
- Pin: commit \`${COMMIT}\` (${COMMIT_DATE}), the default branch's head when
  retrieved, release \`${RELEASE}\`, the newest under \`dist/\` (identical to
  \`dist/current/\` at the commit); each file checked against the sha256 below
- Retrieved: ${RETRIEVED}
- Documents: \`dist/${RELEASE}/oauth-minimal.yaml\` ("Okta OpenID Connect &
  OAuth 2.0", the authorization server endpoints, their parameters and the
  token responses) and \`dist/${RELEASE}/management-minimal.yaml\` ("Okta
  Admin Management API", the User and Application objects), each
  \`info.version\` ${RELEASE}. The other variants under \`dist/${RELEASE}/\`
  (the \`noEnums\`, \`oneOfInheritance\` and \`noExamples\` renderings of the same
  APIs, and the Governance, MyAccount, Access Gateway and Privileged Access
  documents) serve no reader here and are not taken.
- Upstream licence: the repository has no licence file and GitHub reports no
  licence for it. Three statements name Apache-2.0, read ${RETRIEVED}: each
  document's \`info.license\` ("name: Apache-2.0", "url:
  https://www.apache.org/licenses/LICENSE-2.0.html"), the licence badge of
  the \`README.md\` ("License-Apache 2.0"), and the \`license\` field of the
  npm package \`@okta/openapi\` it names ("Apache-2.0"). Apache-2.0 permits
  redistribution with the licence text, which \`LICENSES/Apache-2.0.txt\`
  carries; the repository has no NOTICE file. The \`README.md\` is vendored
  as the record of the badge, and \`REUSE.toml\` declares the tree
  \`Apache-2.0\`. Each document's \`termsOfService\` points at
  <https://developer.okta.com/terms/>, which governs use of the Okta service.
- Layout: the upstream paths, unchanged
- Fetched by: \`scripts/vendor/okta.sh\`; never hand-edit, re-run the script
  to update (.claude/rules/vendored-corpora.md)

| File | sha256 |
|---|---|
$(pinned_rows "${PINS[@]}")

## Cache only

\`okta-oauth-overview.html\` is the "OpenID Connect & OAuth 2.0" API
overview at <${OVERVIEW}>, as served ${RETRIEVED}, the page that lists the
reserved claims of an Okta access token: \`uid\` ("A unique identifier for
the user") and \`cid\` ("Client ID of the client that requested the access
token"). Its footer states "Copyright © 2026 Okta. All rights reserved.",
and <https://developer.okta.com/copyright/> grants no licence to copy it, so
the script fetches it into the git-ignored \`.vendor-cache/okta/\` and this
repository carries none of its content. The page is rebuilt without notice:
FerroHEALTH/FerroAUTH recorded another sha256 for it earlier the same day, so
a refused re-run means the page moved and the pin is moved after reading the
change.

| File | sha256 | Status |
|---|---|---|
$(pinned_status_rows "cache only" "${CACHED[@]}")
EOF

pinned_install "$work/tree" "$DEST"
say "${#PINS[@]} files into docs/specs/okta/, ${#CACHED[@]} cache only"
