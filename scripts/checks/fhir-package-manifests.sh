#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# No committed package.json is a FHIR package manifest or names a FHIR package.
#
# A FHIR package manifest has the npm package.json shape (FHIR NPM Package
# Specification, https://confluence.hl7.org/display/FHIR/NPM+Package+Specification),
# and its dependencies name FHIR packages such as hl7.fhir.r4.core. On the npm
# registry that name is a squatted malicious package (GHSA-qg8x-2hq5-hwf3), so
# a committed FHIR manifest is a trap for anyone who runs npm install beside
# it, and dependency scanners report it as a critical finding (#5337). A
# vendor script keeps a FHIR package as its published tarball in the
# git-ignored .vendor-cache/ and extracts it into a scratch directory instead.
#
# A package.json is refused when it:
#   1. declares `fhirVersions` or `fhirVersion`, the keys of a FHIR manifest;
#   2. names, in `dependencies`, `devDependencies`, `peerDependencies` or
#      `optionalDependencies`, a FHIR package id: one starting with `hl7.` or
#      with `fhir` as a dot-separated part (hl7.fhir.r4.core,
#      hl7.terminology.r4, nictiz.fhir.nl.r4.zib2020, fhir.base);
#   3. is not a JSON object, so neither rule can be read off it.
#
# Usage:
#   scripts/checks/fhir-package-manifests.sh              every committed package.json
#   scripts/checks/fhir-package-manifests.sh --self-test  the detector on synthetic files
#
# Exit codes: 0 clean, 1 a manifest refused, 2 a usage error.
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
cd "$(dirname "$0")/../.."

# The jq program: one line per reason the manifest on stdin is refused.
# shellcheck disable=SC2016 # $deps is a jq variable, not a shell expansion
readonly REASONS='
  if type != "object" then "is not a JSON object"
  else
    (if has("fhirVersions") or has("fhirVersion")
     then "is a FHIR package manifest (it declares fhirVersions)" else empty end),
    ([.dependencies, .devDependencies, .peerDependencies, .optionalDependencies]
     | map(select(type == "object") | keys[]) | unique[] as $deps
     | select($deps | test("^hl7\\.|(^|\\.)fhir(\\.|$)"))
     | "names the FHIR package \($deps) as a dependency")
  end'

# Checks each file argument; prints one line per finding and returns 1 on any.
check() {
  local file reasons status=0
  for file in "$@"; do
    if ! reasons="$(jq -r "$REASONS" "$file" 2>/dev/null)"; then
      reasons="is not valid JSON"
    fi
    while IFS= read -r reason; do
      [[ -n "$reason" ]] || continue
      echo "fhir-package-manifests: $file $reason" >&2
      status=1
    done <<<"$reasons"
  done
  return "$status"
}

self_test() {
  local tmp failed=0
  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand now: $tmp is local and gone at EXIT time
  trap "rm -rf '$tmp'" EXIT
  # Runs check over one synthetic manifest $3 and expects exit $2 with $4 in the output.
  expect() {
    local name="$1" want="$2" body="$3" needle="${4:-}" got out
    printf '%s\n' "$body" >"$tmp/package.json"
    set +e
    out="$(check "$tmp/package.json" 2>&1)"
    got=$?
    set -e
    if [[ "$got" -ne "$want" || ( -n "$needle" && "$out" != *"$needle"* ) ]]; then
      echo "FAIL self-test: $name (exit $got, want $want): $out" >&2
      failed=1
    else
      echo "  self-test: $name"
    fi
  }
  expect "an npm manifest with npm dependencies passes" 0 \
    '{"name":"site","dependencies":{"lodash":"4.17.21"},"devDependencies":{"vite":"7.0.0"}}'
  expect "a manifest with no dependencies passes" 0 '{"name":"site","private":true}'
  expect "a FHIR package manifest is refused" 1 \
    '{"name":"nictiz.fhir.nl.r4.zib2020","fhirVersions":["4.0.1"]}' "declares fhirVersions"
  expect "hl7.fhir.r4.core as a dependency is refused" 1 \
    '{"name":"x","dependencies":{"hl7.fhir.r4.core":"4.0.1"}}' "names the FHIR package hl7.fhir.r4.core"
  expect "hl7.terminology as a dev dependency is refused" 1 \
    '{"name":"x","devDependencies":{"hl7.terminology.r4":"6.0.2"}}' "hl7.terminology.r4"
  expect "a national FHIR package as a peer dependency is refused" 1 \
    '{"name":"x","peerDependencies":{"nictiz.fhir.nl.r4.zib2020":"0.12.1"}}' "nictiz.fhir.nl.r4.zib2020"
  expect "fhir.base as an optional dependency is refused" 1 \
    '{"name":"x","optionalDependencies":{"fhir.base":"1.0.0"}}' "fhir.base"
  expect "a name that only contains fhir passes" 0 \
    '{"name":"x","dependencies":{"fhirpath":"4.0.0","node-fhir-server-core":"2.2.0"}}'
  expect "a manifest that is not JSON is refused" 1 '{"name":' "is not valid JSON"
  expect "a manifest that is not an object is refused" 1 '["hl7.fhir.r4.core"]' "is not a JSON object"
  [[ "$failed" -eq 0 ]] || return 1
  echo "fhir-package-manifests --self-test: every FHIR manifest shape refused, every npm manifest passed: OK."
}

command -v jq >/dev/null || { echo "fhir-package-manifests: jq is required" >&2; exit 2; }

case "${1:-}" in
  --self-test)
    [[ "$#" -eq 1 ]] || guard_usage "[--self-test]"
    self_test
    exit
    ;;
  "") ;;
  *) guard_usage "[--self-test]" ;;
esac

manifests=()
while IFS= read -r -d '' file; do
  manifests+=("$file")
done < <(git ls-files -z -- 'package.json' '**/package.json')

if [[ "${#manifests[@]}" -eq 0 ]]; then
  echo "fhir-package-manifests: no package.json is committed."
  exit 0
fi
if check "${manifests[@]}"; then
  echo "fhir-package-manifests: ${#manifests[@]} package.json file(s), none names a FHIR package."
else
  echo "fhir-package-manifests: keep a FHIR package as its tarball in .vendor-cache/ (scripts/vendor/nl-role-codes.sh)." >&2
  exit 1
fi
