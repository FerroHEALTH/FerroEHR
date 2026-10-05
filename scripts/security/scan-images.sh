#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Rerun the published-image vulnerability scan locally — the same scan
# image-scan.yml runs on Mondays, byte-for-byte in configuration: trivy over
# `trivy.yaml` (HIGH/CRITICAL floor, ignore-unfixed, `.trivyignore.yaml`) with
# every OpenVEX document under security/vex/ applied.
#
# Two modes:
#
#   scripts/security/scan-images.sh
#       scan the three PUBLISHED images at ghcr.io (tag $SCAN_TAG, default
#       `latest`), BOTH platform variants each (the published index is
#       dual-arch and trivy reads one variant per invocation, #2412) —
#       reproduces the scheduled lane's verdict on demand.
#
#   scripts/security/scan-images.sh --candidate IMAGE [IMAGE...]
#       scan locally built or explicitly named image refs instead — the fix
#       loop for a finding: rebuild, scan the candidate, merge only at 0.
#       A local candidate is single-platform by construction (whatever the
#       builder produced); build per-arch to cover both.
#       e.g.  docker build -t ferroehr-postgres:candidate docker/postgres/
#             scripts/security/scan-images.sh --candidate ferroehr-postgres:candidate
#
# Exit status: non-zero when any scanned image carries a fixable HIGH/CRITICAL
# finding the adjudications do not cover — the same red the scheduled lane
# shows. Remediation law: .claude/rules/image-security.md.
set -euo pipefail
cd "$(dirname "$0")/../.."

command -v trivy >/dev/null || { echo "trivy is required (brew install trivy)" >&2; exit 2; }
command -v jq >/dev/null || { echo "jq is required" >&2; exit 2; }

SCAN_TAG=${SCAN_TAG:-latest}
OWNER=${OWNER:-ferrohealth}

# Each entry is "ref|platform"; an empty platform scans the ref as built.
targets=()
if [[ "${1:-}" = "--candidate" ]]; then
  shift
  [[ $# -ge 1 ]] || { echo "--candidate needs at least one image ref" >&2; exit 2; }
  for ref in "$@"; do
    targets+=("${ref}|")
  done
elif [[ $# -gt 0 ]]; then
  echo "unknown argument: $1 (only --candidate IMAGE... is accepted)" >&2
  exit 2
else
  for image in ferroehr ferroehr-postgres ferroehr-viewer; do
    for platform in linux/amd64 linux/arm64; do
      targets+=("ghcr.io/${OWNER}/${image}:${SCAN_TAG}|${platform}")
    done
  done
fi

# Every VEX document, exactly as the scheduled lane passes them.
vex_args=()
for doc in security/vex/*.json; do
  [[ -e "$doc" ]] || continue
  vex_args+=(--vex "$doc")
done

out_dir=$(mktemp -d)
trap 'rm -rf "$out_dir"' EXIT

total=0
for target in "${targets[@]}"; do
  ref=${target%|*}
  platform=${target##*|}
  platform_args=()
  [[ -n "$platform" ]] && platform_args=(--platform "$platform")
  safe=$(printf '%s' "${ref}_${platform}" | tr '/:@' '___')
  report="$out_dir/${safe}.json"
  echo "── scanning ${ref} ${platform:-(as built)}"
  trivy image --skip-version-check --config trivy.yaml --scanners vuln \
    ${platform_args[@]+"${platform_args[@]}"} ${vex_args[@]+"${vex_args[@]}"} \
    -f json -o "$report" "$ref"
  count=$(jq '[.Results[]?.Vulnerabilities // [] | .[]] | length' "$report")
  if [[ "$count" -gt 0 ]]; then
    jq -r '.Results[]? | .Target as $t | (.Vulnerabilities // [])[]
           | "  \($t) | \(.PkgName) \(.InstalledVersion) \(.VulnerabilityID) \(.Severity) -> \(.FixedVersion)"' \
      "$report"
  fi
  echo "   findings: ${count}"
  total=$((total + count))
done

echo "total fixable HIGH/CRITICAL findings: ${total}"
[[ "$total" -eq 0 ]]
