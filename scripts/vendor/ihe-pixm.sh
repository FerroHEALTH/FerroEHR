#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor the parts of the IHE PIXm 3.1.0 FHIR package the EHR index's identity
# source is tested against: the $ihe-pix OperationDefinition, the PIX Manager
# CapabilityStatement, and the IG's example ITI-83 answers.
#
# FerroEHR is a PIXm Patient Identifier Cross-reference Consumer
# (app/ferroehr/src/service/ehr_index/ihe.rs). Its contract tests serve these
# IHE-specified answers from a stub PIX Manager, so the client is held to what
# the profile publishes rather than to one product's behaviour. The copy is
# verbatim; bump the pin and re-run (.claude/rules/vendored-corpora.md).
#
# Usage:
#   scripts/vendor/ihe-pixm.sh
set -euo pipefail

PACKAGE="ihe.iti.pixm"
VERSION="3.1.0"
SHA256="19e2e8eaf3030ac7b4d809c5e1eeb8face02c8635318aeb6d35bc2bb889de0d0"
URL="https://packages.fhir.org/${PACKAGE}/${VERSION}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DEST="${ROOT}/corpus/ihe-pixm"

# The files taken from the package, by their path inside the tarball.
FILES=(
  package/OperationDefinition-IHE.PIXm.pix.json
  package/CapabilityStatement-IHE.PIXm.Manager.json
  package/example/Parameters-pixm-request-mohralice-red-to-blue.json
  package/example/Parameters-pixm-response-mohralice-red-to-blue.json
  package/example/Parameters-pixm-response-mohralice-red-all.json
  package/example/OperationOutcome-pixm-response-error-not-found.json
)

command -v curl >/dev/null 2>&1 || { echo "curl not found on PATH" >&2; exit 1; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

curl --proto =https --fail --silent --show-error --location -o "$work/package.tgz" "$URL"
actual="$(shasum -a 256 "$work/package.tgz" | cut -d' ' -f1)"
if [[ "$actual" != "$SHA256" ]]; then
  echo "error: ${URL} has sha256 ${actual}, the pin is ${SHA256}" >&2
  exit 1
fi
tar -xzf "$work/package.tgz" -C "$work" "${FILES[@]}"

rm -rf "$DEST"
for file in "${FILES[@]}"; do
  mkdir -p "$DEST/$(dirname "$file")"
  cp "$work/$file" "$DEST/$file"
  echo "vendored ${file}"
done

cat >"$DEST/PROVENANCE.md" <<EOF
# Provenance: the IHE PIXm FHIR package (excerpt)

- Source: <${URL}>, the FHIR package registry's copy of the IG published at
  <https://profiles.ihe.net/ITI/PIXm/${VERSION}/>
- Pin: package \`${PACKAGE}\` version \`${VERSION}\`, tarball sha256 \`${SHA256}\`
- Upstream licence: Creative Commons Attribution 4.0 International
  (\`CC-BY-4.0\`, the \`license\` of the package manifest;
  <https://creativecommons.org/licenses/by/4.0/>). The package ships no
  licence file of its own. Attribution: IHE International, IT Infrastructure
  Technical Committee, *Patient Identifier Cross-referencing for Mobile
  (PIXm)* ${VERSION}.
- Layout: the upstream paths inside the package, unchanged
- Files: $(printf '%s ' "${FILES[@]}")
- Fetched by: \`scripts/vendor/ihe-pixm.sh\`; never hand-edit, re-run the
  script to update (.claude/rules/vendored-corpora.md)
- Consumer: \`app/ferroehr/tests/it/ehr_index_ihe.rs\` serves the example
  answers from a stub PIX Manager to the EHR index's identity source
EOF
echo "wrote $DEST/PROVENANCE.md"
