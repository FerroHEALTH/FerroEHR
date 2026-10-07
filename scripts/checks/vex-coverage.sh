#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The per-release exploitability record: every finding of a full Trivy scan
# carries an OpenVEX statement, or the release is refused.
#
# CRA Annex I Part I(2)(a) has products made available "without known
# exploitable vulnerabilities" (docs/law/eu/cra/text.html). The image gate
# (trivy.yaml) blocks only HIGH and CRITICAL findings that have a fix, so a
# lower or unfixed finding would ship with no recorded judgement. The release
# lane therefore scans every image and binary with no severity floor and no
# `ignore-unfixed`, and hands the JSON reports here.
#
# A finding is covered when a statement in an OpenVEX document under the VEX
# directory names its id (the statement's `vulnerability.name` or one of its
# `aliases`, against Trivy's `VulnerabilityID` or one of its `VendorIDs`) for
# the scanned product (the purl the caller gives, compared without version,
# qualifiers or subpath), and the statement is a judgement:
#   * `not_affected` with a `justification` or an `impact_statement`;
#   * `affected` with an `action_statement`;
#   * `fixed` (a backported fix the scanner cannot see).
# `under_investigation` is not a judgement, and neither is a statement missing
# the field the OpenVEX specification requires for its status
# (https://github.com/openvex/spec/blob/main/OPENVEX-SPEC.md, §Status
# Justifications). A finding that is fixed is absent from the scan, so it needs
# nothing.
#
# The record (`--record`) is the joined result: each finding with the statement
# that covers it, or with none. The release attaches it beside the raw reports.
#
# Usage:
#   scripts/checks/vex-coverage.sh [--record <out.json>] [--vex-dir <dir>] \
#     <purl>=<trivy-report.json> [...]
#   scripts/checks/vex-coverage.sh --self-test
#
# Exit codes: 0 every finding covered, 1 a finding uncovered, 2 a usage error
# or an input that is not what it claims to be.
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
readonly GRAMMAR="[--record <out.json>] [--vex-dir <dir>] <purl>=<trivy-report.json>... | --self-test"
guard_known_flags "$GRAMMAR" "--record --vex-dir --self-test" "$@"

die() {
  echo "error: $0: $*" >&2
  exit 2
}

# ── The check ────────────────────────────────────────────────────────────────
check() {
  local record="" vex_dir="security/vex"
  local -a targets=()
  while [[ "$#" -gt 0 ]]; do
    case "$1" in
    --record)
      [[ "$#" -ge 2 ]] || guard_usage "$GRAMMAR"
      record="$2"
      shift 2
      ;;
    --vex-dir)
      [[ "$#" -ge 2 ]] || guard_usage "$GRAMMAR"
      vex_dir="$2"
      shift 2
      ;;
    *=*)
      targets+=("$1")
      shift
      ;;
    *) guard_usage "$GRAMMAR" ;;
    esac
  done
  [[ "${#targets[@]}" -gt 0 ]] || guard_usage "$GRAMMAR"
  [[ -d "$vex_dir" ]] || die "no VEX directory at $vex_dir"

  local work
  work="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand now: $work is local and gone at EXIT time
  trap "rm -rf '$work'" EXIT

  # Every statement of every OpenVEX document, one line each.
  local doc
  local -a docs=()
  : > "$work/statements.jsonl"
  for doc in "$vex_dir"/*.json; do
    [[ -e "$doc" ]] || continue
    jq -e '(."@context" // "") | startswith("https://openvex.dev/ns")' "$doc" >/dev/null 2>&1 \
      || die "$doc is not an OpenVEX document (no openvex.dev @context)"
    docs+=("$doc")
    jq -c --arg doc "$doc" '
      .statements[]? | {
        document: $doc,
        ids: ([.vulnerability.name] + (.vulnerability.aliases // []) | map(select(. != null))),
        products: [.products[]?."@id" | sub("[@?#].*$"; "")],
        status: (.status // ""),
        justification: (.justification // ""),
        impact_statement: (.impact_statement // ""),
        action_statement: (.action_statement // "")
      }' "$doc" >> "$work/statements.jsonl"
  done

  # Every finding of every report, one line each, tagged with its product.
  local target product report
  : > "$work/findings.jsonl"
  : > "$work/targets.jsonl"
  for target in "${targets[@]}"; do
    product="${target%%=*}"
    report="${target#*=}"
    [[ "$product" == pkg:* ]] || die "'$product' is not a purl (expected pkg:<type>/<name>)"
    [[ -f "$report" ]] || die "no report at $report"
    jq -e 'has("SchemaVersion") and has("ArtifactName")' "$report" >/dev/null 2>&1 \
      || die "$report is not a Trivy JSON report (no SchemaVersion/ArtifactName)"
    jq -c --arg product "$product" --arg source "$report" '
      .Results[]? | .Target as $t | (.Vulnerabilities // [])[] | {
        product: $product,
        report: $source,
        target: $t,
        id: .VulnerabilityID,
        vendor_ids: (.VendorIDs // []),
        package: .PkgName,
        installed: .InstalledVersion,
        fixed_version: (.FixedVersion // ""),
        severity: .Severity,
        scanner_status: (.Status // "")
      }' "$report" >> "$work/findings.jsonl"
    jq -c --arg product "$product" --arg source "$report" \
      '{product: $product, report: $source, artifact: .ArtifactName}' \
      "$report" >> "$work/targets.jsonl"
  done

  jq -n \
    --slurpfile statements "$work/statements.jsonl" \
    --slurpfile findings "$work/findings.jsonl" \
    --slurpfile targets "$work/targets.jsonl" \
    --args '
    def judgement:
      (.status == "not_affected" and (.justification != "" or .impact_statement != ""))
      or (.status == "affected" and .action_statement != "")
      or (.status == "fixed");
    [ $findings[] as $f
      | ([$f.id] + $f.vendor_ids) as $names
      | [ $statements[]
          | select(.products | index($f.product))
          | select(. as $s | $names | any(. as $n | $s.ids | index($n))) ] as $matched
      | $f + {
          statement: ([$matched[] | select(judgement)
                       | {document, status, justification, impact_statement, action_statement}]
                      | first),
          refused_statements: [$matched[] | select(judgement | not) | {document, status}]
        } ] as $joined
    | {
        generated_by: "scripts/checks/vex-coverage.sh",
        vex_documents: $ARGS.positional,
        targets: $targets,
        findings: ($joined | length),
        covered: ([$joined[] | select(.statement != null)] | length),
        uncovered: ([$joined[] | select(.statement == null)] | length),
        records: $joined
      }' "${docs[@]}" > "$work/record.json"

  if [[ -n "$record" ]]; then
    cp "$work/record.json" "$record"
  fi

  local findings uncovered
  findings="$(jq '.findings' "$work/record.json")"
  uncovered="$(jq '.uncovered' "$work/record.json")"
  echo "vex-coverage: ${findings} finding(s) across ${#targets[@]} report(s), ${uncovered} without a judgement"
  if [[ "$uncovered" -eq 0 ]]; then
    return 0
  fi
  jq -r '
    .records[] | select(.statement == null)
    | "  UNCOVERED \(.product) \(.id) in \(.package) \(.installed) (\(.severity)), \(.report)"
      + (if (.refused_statements | length) > 0
         then "\n    a statement exists but is no judgement: "
              + ([.refused_statements[] | "\(.document) status=\(.status)"] | join("; "))
         else "" end)' "$work/record.json" | sort -u
  echo "Each finding needs an OpenVEX statement under ${vex_dir}/ (security/vex/README.md says how), or the fix." >&2
  return 1
}

# ── The self-test: the detector, proven on synthetic input ───────────────────
self_test() {
  local tmp
  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand now: $tmp is local and gone at EXIT time
  trap "rm -rf '$tmp'" EXIT
  mkdir -p "$tmp/vex"

  cat > "$tmp/vex/doc.openvex.json" <<'JSON'
{
  "@context": "https://openvex.dev/ns/v0.2.0",
  "@id": "https://example.invalid/vex/self-test",
  "author": "self-test",
  "timestamp": "2026-01-01T00:00:00Z",
  "version": 1,
  "statements": [
    {"vulnerability": {"name": "CVE-0000-0001"},
     "products": [{"@id": "pkg:oci/app"}],
     "status": "not_affected", "justification": "vulnerable_code_not_present"},
    {"vulnerability": {"name": "RUSTSEC-0000-0002", "aliases": ["GHSA-aaaa-bbbb-cccc"]},
     "products": [{"@id": "pkg:cargo/server"}],
     "status": "not_affected", "impact_statement": "never reached"},
    {"vulnerability": {"name": "CVE-0000-0003"},
     "products": [{"@id": "pkg:oci/other"}],
     "status": "not_affected", "justification": "vulnerable_code_not_present"},
    {"vulnerability": {"name": "CVE-0000-0004"},
     "products": [{"@id": "pkg:oci/app"}],
     "status": "under_investigation"},
    {"vulnerability": {"name": "CVE-0000-0005"},
     "products": [{"@id": "pkg:oci/app"}],
     "status": "not_affected"},
    {"vulnerability": {"name": "CVE-0000-0006"},
     "products": [{"@id": "pkg:oci/app"}],
     "status": "affected", "action_statement": "upgrade when upstream ships"},
    {"vulnerability": {"name": "CVE-0000-0007"},
     "products": [{"@id": "pkg:oci/app"}],
     "status": "affected"},
    {"vulnerability": {"name": "CVE-0000-0008"},
     "products": [{"@id": "pkg:oci/app@sha256:0123?arch=amd64"}],
     "status": "fixed"}
  ]
}
JSON

  # report <file> <id> [vendor-id]: a one-finding Trivy report.
  report() {
    local vendor="null"
    [[ -z "${3:-}" ]] || vendor="[\"$3\"]"
    printf '{"SchemaVersion":2,"ArtifactName":"self-test","Results":[{"Target":"t","Class":"os-pkgs","Vulnerabilities":[{"VulnerabilityID":"%s","VendorIDs":%s,"PkgName":"p","InstalledVersion":"1","FixedVersion":"","Severity":"LOW","Status":"affected"}]}]}\n' \
      "$2" "$vendor" > "$1"
  }

  local failed=0 label want args out status
  # <label>|<expected exit>|<expected output fragment or ->|<product>=<report spec>
  # A report spec is <id>[/<vendor-id>], or `clean` (no findings) or `junk`.
  local -a cases=(
    "a covered finding is accepted|0|-|pkg:oci/app=CVE-0000-0001"
    "a statement matched through a vendor id is accepted|0|-|pkg:cargo/server=GHSA-1/GHSA-aaaa-bbbb-cccc"
    "a finding with no statement is refused|1|UNCOVERED pkg:oci/app CVE-0000-0099|pkg:oci/app=CVE-0000-0099"
    "a statement about another product is refused|1|UNCOVERED pkg:oci/app CVE-0000-0003|pkg:oci/app=CVE-0000-0003"
    "under_investigation is refused|1|status=under_investigation|pkg:oci/app=CVE-0000-0004"
    "not_affected without a justification is refused|1|status=not_affected|pkg:oci/app=CVE-0000-0005"
    "affected with an action statement is accepted|0|-|pkg:oci/app=CVE-0000-0006"
    "affected without an action statement is refused|1|status=affected|pkg:oci/app=CVE-0000-0007"
    "a product purl is compared without version and qualifiers|0|-|pkg:oci/app=CVE-0000-0008"
    "a report with no findings is accepted|0|-|pkg:oci/app=clean"
    "a file that is not a Trivy report is a usage error|2|not a Trivy JSON report|pkg:oci/app=junk"
    "a target that is not a purl is a usage error|2|is not a purl|app=CVE-0000-0001"
  )
  local spec product id vendor file
  for entry in "${cases[@]}"; do
    IFS='|' read -r label status want args <<<"$entry"
    product="${args%%=*}"
    spec="${args#*=}"
    file="$tmp/report.json"
    case "$spec" in
    clean) printf '{"SchemaVersion":2,"ArtifactName":"self-test","Results":[{"Target":"t","Class":"os-pkgs"}]}\n' > "$file" ;;
    junk) printf '{"not":"trivy"}\n' > "$file" ;;
    */*)
      id="${spec%%/*}"
      vendor="${spec#*/}"
      report "$file" "$id" "$vendor"
      ;;
    *) report "$file" "$spec" ;;
    esac
    local rc=0
    out="$(bash "$0" --vex-dir "$tmp/vex" "${product}=${file}" 2>&1)" || rc=$?
    if [[ "$rc" -ne "$status" ]]; then
      echo "self-test FAILED: ${label}: exit ${rc}, expected ${status}" >&2
      printf '%s\n' "$out" | sed 's/^/    /' >&2
      failed=1
    elif [[ "$want" != "-" && "$out" != *"$want"* ]]; then
      echo "self-test FAILED: ${label}: output does not name '${want}'" >&2
      printf '%s\n' "$out" | sed 's/^/    /' >&2
      failed=1
    else
      echo "  self-test: ${label}"
    fi
  done

  # The record carries every finding, the covered one with its statement.
  report "$tmp/a.json" CVE-0000-0001
  report "$tmp/b.json" CVE-0000-0099
  local rc=0
  bash "$0" --vex-dir "$tmp/vex" --record "$tmp/record.json" \
    "pkg:oci/app=$tmp/a.json" "pkg:oci/app=$tmp/b.json" >/dev/null 2>&1 || rc=$?
  if [[ "$rc" -ne 1 ]] \
    || ! jq -e '.findings == 2 and .covered == 1 and .uncovered == 1
                and ([.records[] | select(.id == "CVE-0000-0001") | .statement.justification]
                     == ["vulnerable_code_not_present"])' "$tmp/record.json" >/dev/null 2>&1; then
    echo "self-test FAILED: the record does not carry both findings and the covering statement" >&2
    failed=1
  else
    echo "  self-test: the record carries every finding, covered or not"
  fi

  # An unknown flag is refused by name before any checking.
  rc=0
  bash "$0" --bogus >/dev/null 2>&1 || rc=$?
  if [[ "$rc" -ne 2 ]]; then
    echo "self-test FAILED: an unknown flag exited ${rc}, expected 2" >&2
    failed=1
  else
    echo "  self-test: an unknown flag is a usage error"
  fi

  if [[ "$failed" -ne 0 ]]; then
    echo "vex-coverage --self-test: FAILED" >&2
    return 1
  fi
  echo "vex-coverage --self-test: every uncovered shape refused, every judgement accepted — OK."
}

if [[ "${1:-}" == "--self-test" ]]; then
  [[ "$#" -eq 1 ]] || guard_usage "$GRAMMAR"
  self_test
else
  check "$@"
fi
