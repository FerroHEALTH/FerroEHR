#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Whether trivy applies each OpenVEX judgement it should: every finding a
# `not_affected` or `fixed` statement under security/vex/ names for an image
# must be gone from the scan that reads those documents (#5153).
#
# The image gates (image-scan.yml, scan-and-tag.yml) hand the documents to
# `trivy --vex` and trust it to drop the judged findings. Trivy 0.73 to 0.76
# keeps a finding whose package has a dependency cycle among its dependents
# (#5158), so a judgement can be silently ignored. This scans an image twice
# with the same flags, once without and once with the documents, and lists
# every statement-and-finding pair the second scan still reports. A pair is
# matched by the statement's `vulnerability.name` or `aliases` against the
# finding's id, by the product purl without version or qualifiers, and by a
# subcomponent purl against the finding's package purl the same way (a
# statement with no subcomponents covers every package).
#
# Usage:
#   scripts/security/vex-applied.sh [--vex-dir <dir>] --image <name>=<ref>...
#       scan <ref> for linux/amd64 and linux/arm64, product pkg:oci/<name>
#   scripts/security/vex-applied.sh [--vex-dir <dir>] \
#       --compare <purl> <report-without-vex.json> <report-with-vex.json>
#   scripts/security/vex-applied.sh --self-test
#
# Exit codes: 0 every judgement applied, 1 a judgement trivy did not apply,
# 2 a usage error or a scan that could not run.
set -euo pipefail

readonly GRAMMAR="[--vex-dir <dir>] (--image <name>=<ref>... | --compare <purl> <plain.json> <vexed.json>) | --self-test"

usage() {
  echo "usage: $0 $GRAMMAR" >&2
  exit 2
}

die() {
  echo "error: $0: $*" >&2
  exit 2
}

# Prints each judged pair `plain` reports and `vexed` still reports for the
# product `purl`, and the count applied; returns 1 when one is left.
compare() {
  local purl="$1" plain="$2" vexed="$3" vex_dir="$4"
  local report
  for report in "$plain" "$vexed"; do
    jq -e 'has("SchemaVersion") and has("ArtifactName")' "$report" >/dev/null 2>&1 \
      || die "$report is not a Trivy JSON report"
  done
  local -a docs=("$vex_dir"/*.json)
  [[ -e "${docs[0]}" ]] || die "no OpenVEX document under $vex_dir"
  jq -n --arg product "${purl%%[@?#]*}" \
    --slurpfile plain "$plain" --slurpfile vexed "$vexed" \
    --slurpfile docs <(jq -c . "${docs[@]}") '
    def base: sub("[@?#].*$"; "");
    def findings($r):
      [$r[0].Results[]? | (.Vulnerabilities // [])[]
       | {id: .VulnerabilityID, ids: ([.VulnerabilityID] + (.VendorIDs // [])),
          package: .PkgName, purl: ((.PkgIdentifier.PURL // "") | base)}];
    ($docs | map(.statements[]?)
      | map(select(.status == "not_affected" or .status == "fixed"))) as $judged
    | (findings($vexed) | map("\(.id) \(.package)")) as $left
    | [findings($plain)[] as $f
       | $judged[]
       | select(([.vulnerability.name] + (.vulnerability.aliases // []))
           | any(. as $n | $f.ids | index($n)))
       | select(.products[]? | (."@id" | base) == $product
           and ((.subcomponents // []) | length == 0
                or any(.[]; (."@id" | base) == $f.purl)))
       | {pair: "\($f.id) \($f.package)", status}]
    | unique_by(.pair)
    | (map(select(.pair as $p | $left | index($p)))) as $missed
    | "applied: \(length - ($missed | length)) of \(length) judged findings",
      ($missed[] | "not applied: \(.pair) (\(.status))")' -r \
    | tee "$WORK/out"
  ! grep -q '^not applied: ' "$WORK/out"
}

scan() {
  local vex_dir="$1"
  shift
  command -v trivy >/dev/null || die "trivy is required"
  mkdir -p "$WORK/empty"
  : >"$WORK/empty/trivy.yaml"
  : >"$WORK/empty/trivyignore"
  local -a flags=(--config "$WORK/empty/trivy.yaml" --ignorefile "$WORK/empty/trivyignore"
    --scanners vuln --severity "UNKNOWN,LOW,MEDIUM,HIGH,CRITICAL"
    --ignore-unfixed=false --format json --quiet)
  local -a vex=()
  local doc
  for doc in "$vex_dir"/*.json; do
    vex+=(--vex "$doc")
  done
  local target name ref arch status=0
  for target in "$@"; do
    name="${target%%=*}"
    ref="${target#*=}"
    for arch in amd64 arm64; do
      echo "== pkg:oci/${name} linux/${arch}"
      trivy image "${flags[@]}" --platform "linux/${arch}" --output "$WORK/plain.json" "$ref" \
        || die "the scan of $ref without VEX did not run"
      trivy image "${flags[@]}" "${vex[@]}" --platform "linux/${arch}" \
        --output "$WORK/vexed.json" "$ref" || die "the scan of $ref with VEX did not run"
      compare "pkg:oci/${name}" "$WORK/plain.json" "$WORK/vexed.json" "$vex_dir" || status=1
    done
  done
  return "$status"
}

self_test() {
  mkdir -p "$WORK/vex"
  cat >"$WORK/vex/doc.openvex.json" <<'JSON'
{"@context": "https://openvex.dev/ns/v0.2.0", "@id": "https://example.invalid/vex",
 "author": "self-test", "timestamp": "2026-01-01T00:00:00Z", "version": 1,
 "statements": [
  {"vulnerability": {"name": "CVE-0000-0001"},
   "products": [{"@id": "pkg:oci/db", "subcomponents": [{"@id": "pkg:deb/debian/libxml2"}]}],
   "status": "not_affected", "justification": "vulnerable_code_not_present"},
  {"vulnerability": {"name": "CVE-0000-0002"},
   "products": [{"@id": "pkg:oci/db", "subcomponents": [{"@id": "pkg:deb/debian/zlib1g"}]}],
   "status": "not_affected", "justification": "vulnerable_code_not_present"},
  {"vulnerability": {"name": "CVE-0000-0003"},
   "products": [{"@id": "pkg:oci/db", "subcomponents": [{"@id": "pkg:deb/debian/zlib1g"}]}],
   "status": "affected", "action_statement": "upgrade"},
  {"vulnerability": {"name": "CVE-0000-0004"},
   "products": [{"@id": "pkg:oci/app", "subcomponents": [{"@id": "pkg:deb/debian/zlib1g"}]}],
   "status": "not_affected", "justification": "vulnerable_code_not_present"}
 ]}
JSON
  report() {
    jq -n --args '{SchemaVersion: 2, ArtifactName: "db", Results: [{Target: "db",
      Vulnerabilities: [$ARGS.positional[] | split(" ") | {VulnerabilityID: .[0], PkgName: .[1],
        PkgIdentifier: {PURL: "pkg:deb/debian/\(.[1])@1.0?arch=amd64&distro=debian-13"}}]}]}' "$@"
  }
  local all=("CVE-0000-0001 libxml2" "CVE-0000-0002 zlib1g" "CVE-0000-0003 zlib1g" "CVE-0000-0004 zlib1g")
  report "${all[@]}" >"$WORK/plain.json"
  local failed=0
  # Trivy applied both judgements for pkg:oci/db: the affected finding and the other product's stay.
  report "CVE-0000-0003 zlib1g" "CVE-0000-0004 zlib1g" >"$WORK/applied.json"
  if ! compare pkg:oci/db "$WORK/plain.json" "$WORK/applied.json" "$WORK/vex" >/dev/null; then
    echo "vex-applied --self-test: two applied judgements were refused" >&2
    failed=1
  fi
  grep -qx 'applied: 2 of 2 judged findings' "$WORK/out" || {
    echo "vex-applied --self-test: the count is not 2 of 2" >&2
    failed=1
  }
  # The planted zlib1g statement, left in place as trivy leaves it below a dependency cycle.
  report "CVE-0000-0002 zlib1g" "CVE-0000-0003 zlib1g" >"$WORK/missed.json"
  if compare pkg:oci/db "$WORK/plain.json" "$WORK/missed.json" "$WORK/vex" >/dev/null; then
    echo "vex-applied --self-test: an unapplied judgement passed" >&2
    failed=1
  fi
  grep -qx 'not applied: CVE-0000-0002 zlib1g (not_affected)' "$WORK/out" || {
    echo "vex-applied --self-test: the unapplied judgement is not named" >&2
    failed=1
  }
  [[ "$failed" -eq 0 ]] || {
    echo "vex-applied --self-test: FAILED" >&2
    return 1
  }
  echo "vex-applied --self-test: an applied judgement passes, an unapplied one is named and refused: OK."
}

WORK="$(mktemp -d)"
readonly WORK
trap 'rm -rf "$WORK"' EXIT

if [[ "${1:-}" == "--self-test" ]]; then
  [[ "$#" -eq 1 ]] || usage
  self_test
  exit
fi
vex_dir="$(cd "$(dirname "$0")/../.." && pwd)/security/vex"
if [[ "${1:-}" == "--vex-dir" ]]; then
  [[ "$#" -ge 2 ]] || usage
  vex_dir="$2"
  shift 2
fi
[[ -d "$vex_dir" ]] || die "no VEX directory at $vex_dir"
case "${1:-}" in
--compare)
  [[ "$#" -eq 4 ]] || usage
  compare "$2" "$3" "$4" "$vex_dir"
  ;;
--image)
  shift
  [[ "$#" -gt 0 ]] || usage
  targets=()
  while [[ "$#" -gt 0 ]]; do
    [[ "$1" == *=* ]] || usage
    targets+=("$1")
    shift
    [[ "${1:-}" == "--image" ]] && shift
  done
  scan "$vex_dir" "${targets[@]}"
  ;;
*) usage ;;
esac
