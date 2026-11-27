#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The measured-results badges (#6763, #6769): shields.io endpoint documents under
# conformance/badges/, each written from the committed record of one
# measurement.
#
# - openehr-base.json, openehr-options.json: the BASE and OPTIONS rows of the
#   Summary table of the committed released-profile record,
#   docs/conformance/ferroehr/summary.md, which scripts/conformance-record.sh
#   holds to every run; openehr-base-development.json and
#   openehr-options-development.json the same of the development record
#   under development/. The count is the rows passed of the rows judged
#   (passed, failed, warning, errored). Each red row the summary details
#   under the part's heading is a deviation when
#   docs/conformance/party/ferroehr/deviations.json lists its case, named
#   with the ground of its decision, and otherwise a row failed beside the
#   deviations; the rows not run are named apart.
# - atna-interop.json: the newest run under docs/conformance/interop/atna/,
#   written by hand with scripts/interop/atna.sh: the transactions that
#   agree of those it compared, and the date it was measured.
# - claims-review.json: the newest review of
#   website/book/src/compliance/claims-review.md, read by hand once per
#   release: its date.
#
# A badge shows its count, or the date of a hand measurement, within the
# family's length limits (owner decision, 2026-11-27); the claims review
# carries no N/M, so its badge is the informational blue. docs/conformance/badges.md,
# written from the same records, holds the rest: the deviations with their
# ground, the rows failed beside them and not run, each ATNA transaction
# and the review's release and texts. No label or message may say
# certified, compliant or conformant, and every badge carries a logo from
# scripts/checks/badge-logos/.
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
NAMES="openehr-base.json openehr-options.json openehr-base-development.json openehr-options-development.json atna-interop.json claims-review.json"
SUMMARY=docs/conformance/ferroehr/summary.md
DEV_SUMMARY=docs/conformance/ferroehr/development/summary.md
DEVIATIONS=docs/conformance/party/ferroehr/deviations.json
ATNA=docs/conformance/interop/atna
CLAIMS=website/book/src/compliance/claims-review.md
FORBIDDEN='certif|complian|conforman'
PAGE=docs/conformance/badges.md

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

# Prints the case of each red row the summary $1 details under its heading
# `## $2`, one per line.
red_cases() {
  awk -v part="## $2" '
    /^## / { inside = ($0 == part); next }
    inside && /<summary><b>(Failed|Errored|Inconclusive)<\/b>: <code>/ {
      line = $0
      sub(/.*<code>/, "", line)
      sub(/<\/code>.*/, "", line)
      print line
    }
  ' "$1"
}

# Writes the openEHR badge $1 of the Summary row $2 of summary $3, with the
# deviations of the list $4, labelled with the record $5, and appends its
# row to the page rows $6, linking the record page $7.
openehr_badge() {
  local out="$1" part="$2" summary="$3" deviations="$4" record="$5" rows="$6" link="$7"
  local passed failed warning errored skipped judged red message cases listed deviant=0 other=0 case
  local decisions decision grounds="" ground
  read -r passed failed warning errored skipped < <(summary_row "$summary" "$part") || {
    echo "measured-badges: $summary has no $part row in its Summary table" >&2
    return 1
  }
  judged=$((passed + failed + warning + errored))
  red=$((failed + errored))
  cases="$(red_cases "$summary" "$part")"
  listed="$(jq -r '.deviations[].case' "$deviations")"
  while IFS= read -r case; do
    [[ -n "$case" ]] || continue
    if grep -qxF -- "$case" <<< "$listed"; then
      deviant=$((deviant + 1))
      decisions+="$(jq -r --arg c "$case" '.deviations[] | select(.case == $c) | .decision' "$deviations")"$'\n'
    else
      other=$((other + 1))
    fi
  done <<< "$cases"
  [[ $((deviant + other)) -eq "$red" ]] || {
    echo "measured-badges: $summary details $((deviant + other)) red $part rows and counts $red" >&2
    return 1
  }
  local rest=""
  if [[ "$deviant" -gt 0 ]]; then
    decisions="$(LC_ALL=C sort -u <<< "$decisions")"
    while IFS= read -r decision; do
      [[ -n "$decision" ]] || continue
      ground="$(ground_of "$decision")" || {
        echo "measured-badges: $deviations names the decision $decision, which ground_of does not know" >&2
        return 1
      }
      grounds+="${grounds:+; }$ground"
    done <<< "$decisions"
    rest+="${rest:+; }$deviant red by deliberate deviation ($grounds)"
  fi
  [[ "$other" -gt 0 ]] && rest+="${rest:+; }$other failed beside the deviations"
  [[ "$skipped" -gt 0 ]] && rest+="${rest:+; }$skipped not run"
  badge "$out" "openEHR $part $record" "$passed/$judged passed" "$(badge_count_color "$passed" "$judged")" award
  printf '| openEHR %s %s | %s of %s rows passed | %s | [the %s record](%s) |\n' \
    "$part" "$record" "$passed" "$judged" "${rest:--}" "$record" "$link" >> "$rows"
}

# Writes the ATNA interoperability badge $1 from the newest run under $2,
# and appends its row to the page rows $3.
atna_badge() {
  local out="$1" dir="$2" rows="$3" run date verdict transactions disagree compared
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
  case "$verdict" in
    "no disagreement") disagree=0 ;;
    *" disagreements") disagree="${verdict%% *}" ;;
    *) echo "measured-badges: $run has the verdict '$verdict', which scripts/interop/atna.sh does not write" >&2; return 1 ;;
  esac
  [[ "$disagree" =~ ^[0-9]+$ && "$disagree" -le "$transactions" ]] || {
    echo "measured-badges: $run counts $disagree disagreements over $transactions transactions" >&2
    return 1
  }
  badge "$out" "IHE ATNA interoperability" "$((transactions - disagree))/$transactions $date" \
    "$(badge_count_color "$((transactions - disagree))" "$transactions")" exchange
  compared="$(awk -F'|' '
    /^\| Transaction \|/ { on = 1; next }
    on && /^\|---/ { next }
    on && /^\| / { gsub(/^ +| +$/, "", $2); gsub(/^ +| +$/, "", $3); gsub(/^ +| +$/, "", $4); out = out (out == "" ? "" : "<br>") $2 " against " $3 ": " $4; next }
    on { exit }
    END { print out }
  ' "$run")"
  printf '| IHE ATNA interoperability | %s of %s transactions agree, measured %s | %s, %s | [the run](interop/atna/%s.md) |\n' \
    "$((transactions - disagree))" "$transactions" "$date" "$verdict" "$compared" "$date" >> "$rows"
}

# Writes the claims-review badge $1 from the newest review of $2, and
# appends its row to the page rows $3.
claims_badge() {
  local out="$1" page="$2" rows="$3" counts
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
  badge "$out" "EHDS Art. 28 claims review" "${title%%,*}" blue magnifier
  printf '| EHDS Art. 28 claims review | %s | %s texts changed, %s left open | [the review](https://ferroehr.eu/docs/dev/compliance/claims-review.html) |\n' \
    "$title" "$changed" "$open" >> "$rows"
}

# Writes every document into the directory $1 from the tree in the current
# directory, with the page that breaks the badges down as $1/badges.md.
render() {
  local dir="$1" rows="$1/page.rows"
  mkdir -p "$dir"
  : > "$rows"
  openehr_badge "$dir/openehr-base.json" BASE "$SUMMARY" "$DEVIATIONS" released "$rows" ferroehr/summary.md
  openehr_badge "$dir/openehr-options.json" OPTIONS "$SUMMARY" "$DEVIATIONS" released "$rows" ferroehr/summary.md
  openehr_badge "$dir/openehr-base-development.json" BASE "$DEV_SUMMARY" "$DEVIATIONS" development "$rows" \
    ferroehr/development/summary.md
  openehr_badge "$dir/openehr-options-development.json" OPTIONS "$DEV_SUMMARY" "$DEVIATIONS" development "$rows" \
    ferroehr/development/summary.md
  atna_badge "$dir/atna-interop.json" "$ATNA" "$rows"
  claims_badge "$dir/claims-review.json" "$CLAIMS" "$rows"
  {
    printf '<!-- SPDX-FileCopyrightText: Cadasto B.V. -->\n<!-- SPDX-License-Identifier: BUSL-1.1 -->\n'
    printf '<!-- Generated by scripts/render/measured-badges.sh from the committed records. Do not edit. -->\n\n'
    printf '# The measured badges\n\n'
    printf 'Each README badge of a measured result shows one count, or the date of a\n'
    printf 'hand measurement. This page holds the rest, written from the same committed\n'
    printf "records; \`scripts/render/measured-badges.sh --check\` fails when it disagrees\n"
    printf 'with them.\n\n'
    printf '| Badge | Count | The rest | Record |\n|---|---|---|---|\n'
    cat "$rows"
  } > "$dir/badges.md"
  rm -f "$rows"
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
    badge_length_check "$dir/$name" || fail=1
  done
  return "$fail"
}

if [[ "$mode" == self-test ]]; then
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  mkdir -p "$tmp/interop"
  printf '| Part | Passed | Failed | Warning | Errored | Skipped | Total |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: |\n| BASE | 10 | 1 | 0 | 1 | 3 | 15 |\n| OPTIONS | 4 | 0 | 0 | 0 | 0 | 4 |\n\n## BASE\n\n<summary><b>Failed</b>: <code>a/x</code> row 0, BASE</summary>\n<summary><b>Errored</b>: <code>a/x</code> row 1, BASE</summary>\n\n## OPTIONS\n\n## Skipped rows\n' > "$tmp/summary.md"
  printf '{"deviations":[{"case":"a/x","decision":"https://github.com/FerroHEALTH/FerroEHR/issues/5002"}]}' > "$tmp/deviations.json"
  openehr_badge "$tmp/base.json" BASE "$tmp/summary.md" "$tmp/deviations.json" released "$tmp/rows" r.md
  openehr_badge "$tmp/options.json" OPTIONS "$tmp/summary.md" "$tmp/deviations.json" released "$tmp/rows" r.md
  [[ "$(jq -r .message "$tmp/base.json")" == "10/12 passed" ]] ||
    { echo "self-test: the BASE badge says $(jq -r .message "$tmp/base.json")" >&2; exit 1; }
  grep -qxF '| openEHR BASE released | 10 of 12 rows passed | 2 red by deliberate deviation (the authenticated committer, EHDS Annex II 3.1); 3 not run | [the released record](r.md) |' "$tmp/rows" ||
    { echo "self-test: the BASE page row says $(sed -n 1p "$tmp/rows")" >&2; exit 1; }
  [[ "$(jq -r .label "$tmp/base.json")" == "openEHR BASE released" ]] || { echo "self-test: the BASE label" >&2; exit 1; }
  [[ "$(jq -r .color "$tmp/base.json")" == yellow ]] || { echo "self-test: a BASE badge with red rows is not yellow" >&2; exit 1; }
  [[ "$(jq -r .message "$tmp/options.json")" == "4/4 passed" ]] ||
    { echo "self-test: the OPTIONS badge says $(jq -r .message "$tmp/options.json")" >&2; exit 1; }
  printf '| Part | Passed | Failed | Warning | Errored | Skipped | Total |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: |\n| BASE | 10 | 1 | 0 | 0 | 0 | 11 |\n| OPTIONS | 4 | 1 | 0 | 0 | 0 | 5 |\n\n## BASE\n\n<summary><b>Failed</b>: <code>a/x</code> row 0, BASE</summary>\n\n## OPTIONS\n\n<summary><b>Failed</b>: <code>c/y</code> row 0, option z</summary>\n' > "$tmp/dev.md"
  : > "$tmp/rows"
  openehr_badge "$tmp/devbase.json" BASE "$tmp/dev.md" "$tmp/deviations.json" development "$tmp/rows" d.md
  openehr_badge "$tmp/devoptions.json" OPTIONS "$tmp/dev.md" "$tmp/deviations.json" development "$tmp/rows" d.md
  [[ "$(jq -r .message "$tmp/devbase.json")" == "10/11 passed" ]] ||
    { echo "self-test: the development BASE badge says $(jq -r .message "$tmp/devbase.json")" >&2; exit 1; }
  [[ "$(jq -r .message "$tmp/devoptions.json")" == "4/5 passed" ]] ||
    { echo "self-test: the development OPTIONS badge says $(jq -r .message "$tmp/devoptions.json")" >&2; exit 1; }
  grep -qF '| 1 red by deliberate deviation (the authenticated committer, EHDS Annex II 3.1) |' "$tmp/rows" ||
    { echo "self-test: the development BASE page row names no deviation" >&2; exit 1; }
  grep -qF '| 1 failed beside the deviations |' "$tmp/rows" ||
    { echo "self-test: the development OPTIONS page row names no other failure" >&2; exit 1; }
  [[ "$(jq -r .label "$tmp/devoptions.json")" == "openEHR OPTIONS development" ]] || { echo "self-test: the development label" >&2; exit 1; }
  printf '{"deviations":[{"case":"a/x","decision":"https://example.org/other"}]}' > "$tmp/other.json"
  if openehr_badge "$tmp/x.json" BASE "$tmp/summary.md" "$tmp/other.json" released "$tmp/rows" r.md 2> /dev/null; then
    echo "self-test: a deviation with an unknown ground passed" >&2
    exit 1
  fi
  sed 's/| BASE | 10 | 1 | 0 | 1 |/| BASE | 10 | 2 | 0 | 1 |/' "$tmp/summary.md" > "$tmp/short.md"
  if openehr_badge "$tmp/x.json" BASE "$tmp/short.md" "$tmp/deviations.json" released "$tmp/rows" r.md 2> /dev/null; then
    echo "self-test: a summary that details fewer red rows than it counts passed" >&2
    exit 1
  fi
  printf '| Transaction | Independent implementation | Result |\n|---|---|---|\n| a | b | c |\n| d | e | f |\n\nVERDICT: no disagreement\n' > "$tmp/interop/2026-01-02.md"
  printf 'VERDICT: old\n' > "$tmp/interop/2025-12-31.md"
  : > "$tmp/rows"
  atna_badge "$tmp/atna.json" "$tmp/interop" "$tmp/rows"
  [[ "$(jq -r '.message + " " + .color' "$tmp/atna.json")" == "2/2 2026-01-02 brightgreen" ]] ||
    { echo "self-test: the ATNA badge says $(jq -r .message "$tmp/atna.json")" >&2; exit 1; }
  grep -qxF '| IHE ATNA interoperability | 2 of 2 transactions agree, measured 2026-01-02 | no disagreement, a against b: c<br>d against e: f | [the run](interop/atna/2026-01-02.md) |' "$tmp/rows" ||
    { echo "self-test: the ATNA page row says $(cat "$tmp/rows")" >&2; exit 1; }
  sed 's/^VERDICT: no disagreement$/VERDICT: 1 disagreements/' "$tmp/interop/2026-01-02.md" > "$tmp/interop/2026-01-03.md"
  atna_badge "$tmp/atna.json" "$tmp/interop" "$tmp/rows"
  [[ "$(jq -r '.message + " " + .color' "$tmp/atna.json")" == "1/2 2026-01-03 yellow" ]] ||
    { echo "self-test: an ATNA run with a disagreement says $(jq -r '.message + " " + .color' "$tmp/atna.json")" >&2; exit 1; }
  sed 's/^VERDICT: .*/VERDICT: fine/' "$tmp/interop/2026-01-02.md" > "$tmp/interop/2026-01-04.md"
  if atna_badge "$tmp/x.json" "$tmp/interop" "$tmp/rows" 2> /dev/null; then
    echo "self-test: an ATNA verdict atna.sh never writes passed" >&2
    exit 1
  fi
  rm "$tmp/interop/2026-01-03.md" "$tmp/interop/2026-01-04.md"
  atna_badge "$tmp/atna.json" "$tmp/interop" "$tmp/rows"
  printf '## The review of 2026-01-01, for 1.0\n\n### Changed on 2026-01-01\n\n| # | a |\n|---|---|\n| 1 | x |\n\n### Left open\n\n- y\n\n## The review of 2026-02-01, for 1.1\n\n### Changed on 2026-02-01\n\n- a\n- b\n\n### Read and kept\n\n- c\n\n### Left open on 2026-02-01\n\n- d\n' > "$tmp/claims.md"
  : > "$tmp/rows"
  claims_badge "$tmp/claims.json" "$tmp/claims.md" "$tmp/rows"
  [[ "$(jq -r .message "$tmp/claims.json")" == "2026-02-01" ]] ||
    { echo "self-test: the claims badge says $(jq -r .message "$tmp/claims.json")" >&2; exit 1; }
  grep -qF '| EHDS Art. 28 claims review | 2026-02-01, for 1.1 | 2 texts changed, 1 left open |' "$tmp/rows" ||
    { echo "self-test: the claims page row says $(cat "$tmp/rows")" >&2; exit 1; }
  for name in base options devbase devoptions atna claims; do
    badge_logo_check "$tmp/$name.json" || { echo "self-test: $name carries no logo" >&2; exit 1; }
    badge_color_check "$tmp/$name.json" || { echo "self-test: $name has the wrong colour" >&2; exit 1; }
    badge_length_check "$tmp/$name.json" || { echo "self-test: $name runs past the length limits" >&2; exit 1; }
  done
  jq -c '.label = "IHE ATNA interoperability · HAPI FHIR, syslog-ng"' "$tmp/atna.json" > "$tmp/long.json"
  if badge_length_check "$tmp/long.json" 2> /dev/null; then echo "self-test: a label past 30 characters passed" >&2; exit 1; fi
  badge_length_self_test > /dev/null || exit 1
  echo "measured-badges: self-test passed"
  exit 0
fi

if [[ "$mode" == write ]]; then
  render "$OUT"
  mv "$OUT/badges.md" "$PAGE"
  guards "$OUT"
  echo "measured-badges: wrote $OUT and $PAGE"
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
if ! cmp -s "$tmp/badges.md" "$PAGE"; then
  echo "::error::$PAGE is not what the records say: run scripts/render/measured-badges.sh and commit it" >&2
  fail=1
fi
guards "$OUT" || fail=1
[[ "$fail" -eq 0 ]] && echo "measured-badges: every badge matches its record"
exit "$fail"
