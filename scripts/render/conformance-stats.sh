#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Derive every website conformance number from the committed Veredictum record
# (docs/conformance/ferroehr/results.json) at BUILD time. The site sources
# carry no hand-typed conformance number (scripts/checks/conformance-numbers.sh
# fails one in CI), so a page can never show a count the record does not hold.
#
#   conformance-stats.sh includes          write website/book/generated/conformance-stats.md
#   conformance-stats.sh fill-html FILE    fill the data-cnf markers in FILE in place
#
# The record counts every row in one of two parts, as `veredictum export`
# counts them in its Markdown summary (Veredictum book, `export`): OPTIONS
# holds the rows of a case an option arm names or that rests on TRIAL text,
# BASE every other row. The summary committed beside the record is checked
# against the counts read here, so the two can never disagree.
#
# Consumed by scripts/site/build.sh; runnable standalone for local previews.
set -euo pipefail
cd "$(dirname "$0")/../.."

ART="docs/conformance/ferroehr"
RECORD="$ART/results.json"
SUMMARY="$ART/summary.md"
command -v jq >/dev/null 2>&1 || {
  echo "conformance-stats: jq is required (brew install jq; preinstalled on CI runners)" >&2
  exit 1
}
for f in "$RECORD" "$SUMMARY"; do
  [[ -f "$f" ]] || {
    echo "conformance-stats: $f missing; commit the released-profile views of a conformance run" >&2
    exit 1
  }
done

# One line per part, one for every row and one for rows of a case the outline
# does not carry: part passed failed warning errored skipped total.
counts=$(jq -r '
  def tally($rows): [
    ($rows | map(select(.status == "passed")) | length),
    ($rows | map(select(.status == "failed")) | length),
    ($rows | map(select(.status == "warning")) | length),
    ($rows | map(select(.status == "errored")) | length),
    ($rows | map(select(.status == "skipped")) | length),
    ($rows | length)
  ];
  .outline.cases as $cases
  | [.rows[] | . + {part: (
      $cases[.case] as $c
      | if $c == null then "UNKNOWN"
        elif $c.standing.kind == "option" or $c.trial == true then "OPTIONS"
        else "BASE" end)}] as $rows
  | (["BASE"] + tally($rows | map(select(.part == "BASE")))),
    (["OPTIONS"] + tally($rows | map(select(.part == "OPTIONS")))),
    (["ALL"] + tally($rows)),
    (["UNKNOWN"] + tally($rows | map(select(.part == "UNKNOWN"))))
  | @tsv' "$RECORD")

part() { awk -F'\t' -v p="$1" '$1 == p { print $2, $3, $4, $5, $6, $7 }' <<<"$counts"; }
read -r base_passed base_failed base_warning base_errored base_skipped base_total <<<"$(part BASE)"
read -r opt_passed opt_failed opt_warning opt_errored opt_skipped opt_total <<<"$(part OPTIONS)"
read -r all_passed all_failed all_warning all_errored all_skipped all_total <<<"$(part ALL)"
read -r _ _ _ _ _ unknown_total <<<"$(part UNKNOWN)"

# Refuse to render nonsense: a truncated record or a row of a case the outline
# does not carry fails the build.
for v in "$base_passed" "$base_failed" "$base_warning" "$base_errored" "$base_skipped" "$base_total" \
  "$opt_passed" "$opt_failed" "$opt_warning" "$opt_errored" "$opt_skipped" "$opt_total" \
  "$all_passed" "$all_failed" "$all_warning" "$all_errored" "$all_skipped" "$all_total"; do
  [[ "$v" =~ ^[0-9]+$ ]] || { echo "conformance-stats: non-numeric count '$v' in $RECORD" >&2; exit 1; }
done
if [[ "$unknown_total" != 0 ]]; then
  echo "conformance-stats: $unknown_total row(s) of a case the record's outline does not carry" >&2
  exit 1
fi
if [[ "$all_total" == 0 ]]; then
  echo "conformance-stats: $RECORD holds no rows" >&2
  exit 1
fi

# The summary's own table, read back as the same six numbers per part.
summary_row() {
  awk -F'|' -v p="$1" '
    { name = $2; gsub(/[ *]/, "", name) }
    name == p {
      out = ""
      for (i = 3; i <= 8; i++) { v = $i; gsub(/[ *]/, "", v); out = out (i > 3 ? " " : "") v }
      print out
      exit
    }' "$SUMMARY"
}
for check in "BASE|$base_passed $base_failed $base_warning $base_errored $base_skipped $base_total" \
  "OPTIONS|$opt_passed $opt_failed $opt_warning $opt_errored $opt_skipped $opt_total" \
  "Everyrow|$all_passed $all_failed $all_warning $all_errored $all_skipped $all_total"; do
  name="${check%%|*}"
  want="${check#*|}"
  got=$(summary_row "$name")
  if [[ "$got" != "$want" ]]; then
    echo "conformance-stats: $SUMMARY row '$name' reads '$got' where the record counts '$want'" >&2
    exit 1
  fi
done

instrument=$(jq -r '"\(.engine.name) \(.engine.version)"' "$RECORD")
profile=$(jq -r '.specification.profile' "$RECORD")
server=$(jq -r '.server.answer.options | "\(.solution) \(.solution_version)"' "$RECORD")
for v in "$instrument" "$profile" "$server"; do
  [[ -n "$v" && "$v" != *null* ]] || { echo "conformance-stats: $RECORD names no instrument, profile or server" >&2; exit 1; }
done

# The record carries no clock, so the run date is the AUTHOR date of the
# record's last commit (fallback: the file's mtime). A rebase or a re-signing
# of history restamps the committer date and leaves the author date alone.
run_date=$(git log -1 --format=%as -- "$RECORD" 2>/dev/null || true)
[[ -n "$run_date" ]] || run_date=$(date -r "$RECORD" +%Y-%m-%d)

ran=$((all_total - all_skipped))
against=$((all_failed + all_errored))

case "${1:-includes}" in
  includes)
    mkdir -p website/book/generated
    cat > website/book/generated/conformance-stats.md <<EOF
The record of ${run_date}: ${server}, judged by ${instrument} under the
\`${profile}\` profile.

| Part | Passed | Failed | Warning | Errored | Skipped | Total |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| BASE | ${base_passed} | ${base_failed} | ${base_warning} | ${base_errored} | ${base_skipped} | ${base_total} |
| OPTIONS | ${opt_passed} | ${opt_failed} | ${opt_warning} | ${opt_errored} | ${opt_skipped} | ${opt_total} |
| **Every row** | **${all_passed}** | **${all_failed}** | **${all_warning}** | **${all_errored}** | **${all_skipped}** | **${all_total}** |
EOF
    echo "conformance-stats: wrote website/book/generated/conformance-stats.md (${all_passed} of ${ran} rows passed, record of ${run_date})"
    ;;
  fill-html)
    file="${2:?fill-html needs a file argument}"
    perl -pi -e "
      s/(data-cnf=\"ran\"[^>]*>)[^<]*/\${1}${ran}/g;
      s/(data-cnf=\"passed\"[^>]*>)[^<]*/\${1}${all_passed}/g;
      s/(data-cnf=\"failed\"[^>]*>)[^<]*/\${1}${against}/g;
      s/(data-cnf=\"skipped\"[^>]*>)[^<]*/\${1}${all_skipped}/g;
      s/(data-cnf=\"record\"[^>]*>)[^<]*/\${1}${server}, ${instrument}, ${profile} profile, ${run_date}/g;
    " "$file"
    # A marker that survived filling means the HTML and this script drifted.
    if grep -qE 'data-cnf="[^"]*"[^>]*>—<' "$file"; then
      echo "conformance-stats: unfilled data-cnf marker left in $file" >&2
      exit 1
    fi
    echo "conformance-stats: filled $file"
    ;;
  *)
    echo "usage: $0 [includes | fill-html FILE]" >&2
    exit 2
    ;;
esac
