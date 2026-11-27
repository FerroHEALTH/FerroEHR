#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The measured-results badges (#6763): shields.io endpoint documents under
# conformance/badges/, each written from the committed record of one
# measurement.
#
# - openehr-base.json, openehr-options.json: the BASE and OPTIONS rows of the
#   Summary table of the committed released-profile record,
#   docs/conformance/ferroehr/summary.md, which scripts/conformance-record.sh
#   holds to every run. The count is the rows passed of the rows judged
#   (passed, failed, warning, errored), the red rows are named with the
#   ground of the deviations docs/conformance/party/ferroehr/deviations.json
#   lists, and the rows not run are named apart.
# - atna-interop.json: the newest run under docs/conformance/interop/atna/,
#   written by hand with scripts/interop/atna.sh: its verdict, the
#   transactions it compared and the date it was measured.
# - claims-review.json: the newest review of
#   website/book/src/compliance/claims-review.md, read by hand once per
#   release: its date, its release and the texts it changed and left open.
#
# A hand measurement carries no N/M, so its badge is the informational blue
# and names its date. No label or message may say certified, compliant or
# conformant, and every badge carries a logo from scripts/checks/badge-logos/.
#
# Usage: scripts/render/measured-badges.sh              rewrite the documents
#        scripts/render/measured-badges.sh --check      fail on a stale document
#        scripts/render/measured-badges.sh --self-test  the counting on fixtures
# Needs jq. No specification governs the badges: our own design.
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
GRAMMAR="[--check | --self-test]"
mode="write"
case "${1:-}" in
  "") ;;
  --check) mode=check ;;
  --self-test) mode=self-test ;;
  *) guard_usage "$GRAMMAR" ;;
esac
[[ "$#" -le 1 ]] || guard_usage "$GRAMMAR"
cd "$(dirname "$0")/../.."
# shellcheck source=scripts/checks/badge-logos/logo.sh
. scripts/checks/badge-logos/logo.sh

OUT=conformance/badges
NAMES="openehr-base.json openehr-options.json atna-interop.json claims-review.json"
SUMMARY=docs/conformance/ferroehr/summary.md
DEVIATIONS=docs/conformance/party/ferroehr/deviations.json
ATNA=docs/conformance/interop/atna
CLAIMS=website/book/src/compliance/claims-review.md
FORBIDDEN='certif|complian|conforman'

# The short ground of a deviation decision, by its issue URL. A deviation
# under a decision this map does not name fails the render, so a new ground
# reaches the badge only by a reviewed line here.
ground_of() {
  case "$1" in
    https://github.com/FerroHEALTH/FerroEHR/issues/5002) echo "the authenticated committer, EHDS Annex II 3.1" ;;
    *) return 1 ;;
  esac
}

# Writes the endpoint document $1: label $2, message $3, colour $4, logo $5.
badge() {
  jq -cn --arg label "$2" --arg message "$3" --arg color "$4" --argjson logo "$(badge_logo_fields "$5")" \
    '{schemaVersion: 1, label: $label, message: $message, color: $color} + $logo' > "$1"
}

# Prints "passed failed warning errored skipped" of the Summary row $2 of the
# record summary $1.
summary_row() {
  awk -F'|' -v part="$2" '
    { cell = $2; gsub(/^[ \t]+|[ \t]+$/, "", cell) }
    cell == part { gsub(/[ \t]/, ""); split($0, c, "|"); print c[3], c[4], c[5], c[6], c[7]; found = 1; exit }
    END { if (!found) exit 1 }
  ' "$1"
}

# Writes the openEHR badge $1 of the Summary row $2 of summary $3, with the
# deviations of the list $4.
openehr_badge() {
  local out="$1" part="$2" summary="$3" deviations="$4" passed failed warning errored skipped judged red message
  local decisions decision grounds="" ground
  read -r passed failed warning errored skipped < <(summary_row "$summary" "$part") || {
    echo "measured-badges: $summary has no $part row in its Summary table" >&2
    return 1
  }
  judged=$((passed + failed + warning + errored))
  red=$((failed + errored))
  message="$passed/$judged rows passed"
  if [[ "$red" -gt 0 ]]; then
    decisions="$(jq -r '[.deviations[].decision] | unique | .[]' "$deviations")"
    while IFS= read -r decision; do
      [[ -n "$decision" ]] || continue
      ground="$(ground_of "$decision")" || {
        echo "measured-badges: $deviations names the decision $decision, which ground_of does not know" >&2
        return 1
      }
      grounds+="${grounds:+; }$ground"
    done <<< "$decisions"
    [[ -n "$grounds" ]] || {
      echo "measured-badges: $summary has $red red $part rows and $deviations lists no deviation" >&2
      return 1
    }
    message+=", $red red by deliberate deviation ($grounds)"
  fi
  [[ "$skipped" -gt 0 ]] && message+=", $skipped not run"
  badge "$out" "openEHR $part · released record" "$message" "$(badge_count_color "$passed" "$judged")" award
}

# Writes the ATNA interoperability badge $1 from the newest run under $2.
atna_badge() {
  local out="$1" dir="$2" run date verdict transactions
  run="$(find "$dir" -maxdepth 1 -name '20[0-9][0-9]-[0-9][0-9]-[0-9][0-9].md' | LC_ALL=C sort | tail -1)"
  [[ -n "$run" ]] || { echo "measured-badges: $dir holds no dated run" >&2; return 1; }
  date="$(basename "$run" .md)"
  verdict="$(sed -n 's/^VERDICT: //p' "$run")"
  verdict="$(sed -n 1p <<< "$verdict")"
  [[ -n "$verdict" ]] || { echo "measured-badges: $run has no VERDICT line" >&2; return 1; }
  transactions="$(awk '
    /^\| Transaction \|/ { on = 1; next }
    on && /^\|---/ { next }
    on && /^\| / { n++; next }
    on { exit }
    END { print n + 0 }
  ' "$run")"
  [[ "$transactions" -gt 0 ]] || { echo "measured-badges: $run compares no transaction" >&2; return 1; }
  badge "$out" "IHE ATNA interoperability · HAPI FHIR, syslog-ng" \
    "$verdict over $transactions transactions, measured $date" blue exchange
}

# Writes the claims-review badge $1 from the newest review of $2.
claims_badge() {
  local out="$1" page="$2" counts
  counts="$(awk '
    function item() { return ($0 ~ /^- / || $0 ~ /^\| [0-9]+ \|/) }
    /^## The review of / { title = $0; sub(/^## The review of /, "", title); changed = 0; open = 0; section = ""; next }
    /^### Changed/ { section = "changed"; next }
    /^### Left open/ { section = "open"; next }
    /^#/ { section = ""; next }
    section == "changed" && item() { changed++ }
    section == "open" && item() { open++ }
    END { if (title == "") exit 1; print title "\t" changed + 0 "\t" open + 0 }
  ' "$page")" || { echo "measured-badges: $page holds no review" >&2; return 1; }
  IFS=$'\t' read -r title changed open <<< "$counts"
  badge "$out" "EHDS Art. 28 claims review" "$title: $changed texts changed, $open left open" blue magnifier
}

# Writes every document into the directory $1 from the tree in the current
# directory.
render() {
  local dir="$1"
  mkdir -p "$dir"
  openehr_badge "$dir/openehr-base.json" BASE "$SUMMARY" "$DEVIATIONS"
  openehr_badge "$dir/openehr-options.json" OPTIONS "$SUMMARY" "$DEVIATIONS"
  atna_badge "$dir/atna-interop.json" "$ATNA"
  claims_badge "$dir/claims-review.json" "$CLAIMS"
}

# Fails for each rendered badge that says certified, compliant or conformant,
# carries no logo, or has a colour its message does not ask for.
guards() {
  local dir="$1" fail=0 name words
  for name in $NAMES; do
    words="$(jq -r '.label, .message' "$dir/$name")"
    if grep -qiE "$FORBIDDEN" <<< "$words"; then
      echo "::error::$dir/$name says certified, compliant or conformant" >&2
      fail=1
    fi
    badge_logo_check "$dir/$name" || fail=1
    badge_color_check "$dir/$name" || fail=1
  done
  return "$fail"
}

if [[ "$mode" == self-test ]]; then
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  mkdir -p "$tmp/interop"
  printf '| Part | Passed | Failed | Warning | Errored | Skipped | Total |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: |\n| BASE | 10 | 1 | 0 | 1 | 3 | 15 |\n| OPTIONS | 4 | 0 | 0 | 0 | 0 | 4 |\n' > "$tmp/summary.md"
  printf '{"deviations":[{"case":"a","decision":"https://github.com/FerroHEALTH/FerroEHR/issues/5002"}]}' > "$tmp/deviations.json"
  openehr_badge "$tmp/base.json" BASE "$tmp/summary.md" "$tmp/deviations.json"
  openehr_badge "$tmp/options.json" OPTIONS "$tmp/summary.md" "$tmp/deviations.json"
  [[ "$(jq -r .message "$tmp/base.json")" == "10/12 rows passed, 2 red by deliberate deviation (the authenticated committer, EHDS Annex II 3.1), 3 not run" ]] ||
    { echo "self-test: the BASE badge says $(jq -r .message "$tmp/base.json")" >&2; exit 1; }
  [[ "$(jq -r .color "$tmp/base.json")" == yellow ]] || { echo "self-test: a BASE badge with red rows is not yellow" >&2; exit 1; }
  [[ "$(jq -r .message "$tmp/options.json")" == "4/4 rows passed" ]] ||
    { echo "self-test: the OPTIONS badge says $(jq -r .message "$tmp/options.json")" >&2; exit 1; }
  printf '{"deviations":[{"case":"a","decision":"https://example.org/other"}]}' > "$tmp/other.json"
  if openehr_badge "$tmp/x.json" BASE "$tmp/summary.md" "$tmp/other.json" 2> /dev/null; then
    echo "self-test: a deviation with an unknown ground passed" >&2
    exit 1
  fi
  printf '{"deviations":[]}' > "$tmp/none.json"
  if openehr_badge "$tmp/x.json" BASE "$tmp/summary.md" "$tmp/none.json" 2> /dev/null; then
    echo "self-test: red rows with no listed deviation passed" >&2
    exit 1
  fi
  printf '| Transaction | Independent implementation | Result |\n|---|---|---|\n| a | b | c |\n| d | e | f |\n\nVERDICT: no disagreement\n' > "$tmp/interop/2026-01-02.md"
  printf 'VERDICT: old\n' > "$tmp/interop/2025-12-31.md"
  atna_badge "$tmp/atna.json" "$tmp/interop"
  [[ "$(jq -r .message "$tmp/atna.json")" == "no disagreement over 2 transactions, measured 2026-01-02" ]] ||
    { echo "self-test: the ATNA badge says $(jq -r .message "$tmp/atna.json")" >&2; exit 1; }
  printf '## The review of 2026-01-01, for 1.0\n\n### Changed on 2026-01-01\n\n| # | a |\n|---|---|\n| 1 | x |\n\n### Left open\n\n- y\n\n## The review of 2026-02-01, for 1.1\n\n### Changed on 2026-02-01\n\n- a\n- b\n\n### Read and kept\n\n- c\n\n### Left open on 2026-02-01\n\n- d\n' > "$tmp/claims.md"
  claims_badge "$tmp/claims.json" "$tmp/claims.md"
  [[ "$(jq -r .message "$tmp/claims.json")" == "2026-02-01, for 1.1: 2 texts changed, 1 left open" ]] ||
    { echo "self-test: the claims badge says $(jq -r .message "$tmp/claims.json")" >&2; exit 1; }
  for name in base options atna claims; do
    badge_logo_check "$tmp/$name.json" || { echo "self-test: $name carries no logo" >&2; exit 1; }
    badge_color_check "$tmp/$name.json" || { echo "self-test: $name has the wrong colour" >&2; exit 1; }
  done
  echo "measured-badges: self-test passed"
  exit 0
fi

if [[ "$mode" == write ]]; then
  render "$OUT"
  guards "$OUT"
  echo "measured-badges: wrote $OUT"
  exit 0
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
render "$tmp"
fail=0
for name in $NAMES; do
  if ! cmp -s "$tmp/$name" "$OUT/$name"; then
    echo "::error::$OUT/$name is not what its record says: run scripts/render/measured-badges.sh and commit it" >&2
    fail=1
  fi
done
guards "$OUT" || fail=1
[[ "$fail" -eq 0 ]] && echo "measured-badges: every badge matches its record"
exit "$fail"
