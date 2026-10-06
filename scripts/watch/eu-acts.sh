#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# EUR-Lex watcher: acts adopted under, supplementing, amending or correcting
# Regulation (EU) 2025/327 (EHDS, CELEX 32025R0327) and Regulation (EU)
# 2024/2847 (CRA, CELEX 32024R2847). No specification governs this: our own
# design.
#
# The source is the Publications Office's Cellar, the repository EUR-Lex
# serves, through its SPARQL endpoint: every legislative work (CELEX sector 3)
# linked to either base by `resource_legal_based_on_resource_legal`,
# `…_completes_…`, `…_amends_…` or `…_corrects_…`, with the legal-basis article
# Cellar annotates on the link. An act is known when
# scripts/watch/eu-acts-known.tsv lists it; each other act files one issue
# through the watcher family's engine, deduplicated on its CELEX number over
# open and closed issues.
#
# Run colour: Cellar not answering, an answer of the wrong shape, or an answer
# missing an act the baseline lists is a RED run (the probe cannot be
# trusted); a new act files an issue and the run stays GREEN.
#
# Usage:
#   scripts/watch/eu-acts.sh              # file one issue per new act
#   scripts/watch/eu-acts.sh --list       # print the classification, file nothing
#   scripts/watch/eu-acts.sh --self-test  # offline checks of the probe
# Env: DRY_RUN=1 (the engine reports, creates nothing) · GH_TOKEN for gh ·
# EU_ACTS_SPARQL overrides the endpoint. Needs awk, curl, jq (and gh to file).
set -euo pipefail
cd "$(dirname "$0")/../.."

readonly UA='ferroehr-watch/1.0 (+https://github.com/FerroHEALTH/FerroEHR)'
readonly SPARQL="${EU_ACTS_SPARQL:-https://publications.europa.eu/webapi/rdf/sparql}"
readonly KNOWN_FILE=scripts/watch/eu-acts-known.tsv
readonly FILE_ISSUE=.github/actions/file-watcher-issue/file-issue.sh

readonly QUERY='PREFIX cdm: <http://publications.europa.eu/ontology/cdm#>
PREFIX ann: <http://publications.europa.eu/ontology/annotation#>
PREFIX owl: <http://www.w3.org/2002/07/owl#>
SELECT ?celex (GROUP_CONCAT(DISTINCT ?link; separator=",") AS ?links)
       (MIN(STR(?d)) AS ?date) (GROUP_CONCAT(DISTINCT ?code; separator=",") AS ?codes)
       (SAMPLE(?t) AS ?title)
WHERE {
  {
    SELECT DISTINCT ?act ?b ?rel ?celex ?link WHERE {
      VALUES (?base ?rel ?kind) {
        ("32025R0327" cdm:resource_legal_based_on_resource_legal "based_on")
        ("32025R0327" cdm:resource_legal_completes_resource_legal "supplements")
        ("32025R0327" cdm:resource_legal_amends_resource_legal "amends")
        ("32025R0327" cdm:resource_legal_corrects_resource_legal "corrects")
        ("32024R2847" cdm:resource_legal_based_on_resource_legal "based_on")
        ("32024R2847" cdm:resource_legal_completes_resource_legal "supplements")
        ("32024R2847" cdm:resource_legal_amends_resource_legal "amends")
        ("32024R2847" cdm:resource_legal_corrects_resource_legal "corrects")
      }
      ?b cdm:resource_legal_id_celex ?bc .
      FILTER(STR(?bc) = ?base)
      ?act ?rel ?b ;
           cdm:resource_legal_id_celex ?celex .
      FILTER(STRSTARTS(STR(?celex), "3"))
      BIND(CONCAT(?kind, ":", ?base) AS ?link)
    }
  }
  OPTIONAL { ?act cdm:work_date_document ?d }
  OPTIONAL {
    ?axiom owl:annotatedSource ?act ;
           owl:annotatedProperty ?rel ;
           owl:annotatedTarget ?b ;
           ann:comment_on_legal_basis ?code .
  }
  OPTIONAL {
    ?expression cdm:expression_belongs_to_work ?act ;
                cdm:expression_uses_language <http://publications.europa.eu/resource/authority/language/ENG> ;
                cdm:expression_title ?t .
  }
}
GROUP BY ?celex
ORDER BY ?celex'

# Prints the regulation a base CELEX number names.
base_name() {
  case "$1" in
    32025R0327) echo "Regulation (EU) 2025/327 (EHDS)" ;;
    32024R2847) echo "Regulation (EU) 2024/2847 (CRA)" ;;
    *) echo "CELEX $1" ;;
  esac
}

# Prints a Cellar legal-basis code (`A23P4`, `A02P5L2`) as `Art 23(4)`, any
# further qualifier kept verbatim in brackets.
basis_text() {
  awk -v code="$1" 'BEGIN {
    if (!match(code, /^A[0-9]+/)) { print "[" code "]"; exit }
    text = "Art " (substr(code, 2, RLENGTH - 1) + 0); rest = substr(code, RLENGTH + 1)
    if (match(rest, /^P[0-9]+/)) { text = text "(" (substr(rest, 2, RLENGTH - 1) + 0) ")"; rest = substr(rest, RLENGTH + 1) }
    if (rest != "") text = text " [" code "]"
    print text
  }'
}

# Prints the articles a comma-separated code list names, or `-` for none.
articles_text() {
  local codes="$1" code out=""
  [[ "$codes" != "-" ]] || { echo "-"; return; }
  for code in ${codes//,/ }; do
    out="${out:+$out, }$(basis_text "$code")"
  done
  echo "$out"
}

# Prints the relation a `kind:base` link names.
link_text() {
  local kind="${1%%:*}" base="${1#*:}" verb
  case "$kind" in
    based_on) verb="adopted under" ;;
    supplements) verb="supplements" ;;
    amends) verb="amends" ;;
    corrects) verb="corrects" ;;
    *) verb="$kind" ;;
  esac
  echo "$verb $(base_name "$base")"
}

# Prints every relation of a comma-separated link list, joined by "; ".
links_text() {
  local link out=""
  for link in ${1//,/ }; do
    out="${out:+$out; }$(link_text "$link")"
  done
  echo "$out"
}

# Reads a SPARQL JSON answer on stdin and prints one
# celex<TAB>links<TAB>date<TAB>codes<TAB>title record per act, `-` for a field
# Cellar does not record; fails on any other shape or on no act at all.
parse_acts() {
  jq --raw-output --exit-status '.results.bindings
    | if type != "array" or length == 0 then error("no act in the answer") else . end
    | .[]
    | [ .celex.value,
        (.links.value // "-"),
        (.date.value // "-"),
        ((.codes.value // "") | if . == "" then "-" else . end),
        ((.title.value // "-") | gsub("[\t\n]+"; " ")) ]
    | @tsv'
}

# Queries the endpoint `$1` and prints the act records; fails loud when the
# endpoint does not answer or answers in another shape.
fetch_acts() {
  local url="$1" raw records
  if ! raw="$(curl --fail --silent --show-error --max-time 120 --retry 2 --user-agent "$UA" \
    --header 'Accept: application/sparql-results+json' --data-urlencode "query=$QUERY" "$url" 2>&1)"; then
    echo "eu-acts: Cellar did not answer at $url: $raw" >&2
    return 1
  fi
  if ! records="$(parse_acts <<<"$raw" 2>/dev/null)"; then
    echo "eu-acts: the answer from $url is not a SPARQL result listing acts: ${raw:0:200}" >&2
    return 1
  fi
  printf '%s\n' "$records"
}

# Prints the CELEX numbers the baseline file `$1` lists.
known_acts() {
  awk -F'\t' '!/^#/ && NF { print $1 }' "$1"
}

# Prints the records on stdin whose CELEX the list `$1` does not hold.
new_acts() {
  local known="$1" celex rest
  while IFS=$'\t' read -r celex rest; do
    [[ -n "$celex" ]] || continue
    grep -qxF "$celex" <<<"$known" || printf '%s\t%s\n' "$celex" "$rest"
  done
}

# Prints the CELEX numbers in the list `$1` that the records `$2` lack.
missing_known() {
  local known="$1" records="$2" celex
  while IFS= read -r celex; do
    [[ -n "$celex" ]] || continue
    awk -F'\t' -v c="$celex" '$1 == c { found = 1 } END { exit !found }' <<<"$records" ||
      printf '%s\n' "$celex"
  done <<<"$known"
}

# Prints the issue title for an act record and writes its body to file `$2`.
render_issue() {
  local celex links date codes title first articles url
  IFS=$'\t' read -r celex links date codes title <<<"$1"
  first="${links%%,*}"
  articles="$(articles_text "$codes")"
  [[ "$articles" != "-" ]] || articles="Cellar states no legal-basis article"
  [[ "$title" != "-" ]] || title="Cellar records no English title yet"
  url="https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX:$(printf '%s' "$celex" | sed 's/(/%28/g; s/)/%29/g')"
  # shellcheck disable=SC2016 # the backticks are literal markdown, never expansions
  {
    printf 'Cellar records an act this repository has not triaged.\n\n'
    printf -- '- **CELEX:** `%s` (%s)\n' "$celex" "$url"
    printf -- '- **Title:** %s\n' "$title"
    printf -- '- **Date of the document:** %s\n' "$date"
    printf -- '- **Relation:** %s\n' "$(links_text "$links")"
    printf -- '- **Article implemented:** %s\n\n' "$articles"
    printf '### Checklist\n\n'
    printf -- '- [ ] Read the act and decide whether it places an obligation on FerroEHR or its manufacturer\n'
    printf -- '- [ ] Vendor it under `docs/law/eu/` (`scripts/vendor/law-eu.sh`) if it does, or record why not\n'
    printf -- '- [ ] File the compliance work it creates\n'
    printf -- '- [ ] Add `%s` to `scripts/watch/eu-acts-known.tsv`\n\n' "$celex"
    printf '_Opened automatically by `.github/workflows/eu-acts-watcher.yml`._\n'
  } >"$2"
  echo "chore(law): EUR-Lex act $celex, $(link_text "$first")"
}

self_test() {
  local failed=0 got tmp known records status
  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand $tmp now, while it is still in scope
  trap "rm -rf '$tmp'" RETURN
  expect() {
    if [[ "$2" != "$1" ]]; then
      printf 'eu-acts: self-test failed: %s gave "%s", wanted "%s".\n' "$3" "$2" "$1" >&2
      failed=1
    fi
  }
  expect "Art 23(4)" "$(basis_text A23P4)" "a paragraph"
  expect "Art 2(5) [A02P5L2]" "$(basis_text A02P5L2)" "a qualified paragraph"
  expect "Art 16" "$(basis_text A16)" "an article"
  expect "[ANNEX]" "$(basis_text ANNEX)" "a code of another form"
  expect "Art 23(4), Art 23(8)" "$(articles_text A23P4,A23P8)" "two codes"

  known="$(known_acts "$KNOWN_FILE")"
  records="$(printf '32026R2083\tbased_on:32025R0327\t2026-09-18\tA23P4,A23P8\tOn MyHealth@EU\n32024R2847R(07)\tcorrects:32024R2847\t2026-08-06\t-\t-\n')"
  expect "" "$(new_acts "$known" <<<"$records")" "the baseline"
  records="$records"$'\n'"$(printf '32027R0101\tbased_on:32025R0327,supplements:32025R0327\t2027-01-04\tA15P1\tOn the exchange format\n')"
  expect "32027R0101" "$(new_acts "$known" <<<"$records" | cut -f1)" "a new act"
  expect "" "$(missing_known $'32026R2083\n32024R2847R(07)' "$records")" "an answer holding the baseline"
  expect "32026R2099" "$(missing_known $'32026R2083\n32026R2099' "$records")" "an answer lacking a known act"

  got="$(render_issue "$(new_acts "$known" <<<"$records")" "$tmp/body.md")"
  expect "chore(law): EUR-Lex act 32027R0101, adopted under Regulation (EU) 2025/327 (EHDS)" "$got" "the issue title"
  grep -qF -- '- **Article implemented:** Art 15(1)' "$tmp/body.md" || {
    echo "eu-acts: self-test failed: the issue body does not name Art 15(1)." >&2
    failed=1
  }
  got="$(render_issue $'32024R2847R(08)\tcorrects:32024R2847\t2027-02-01\t-\t-' "$tmp/body.md")"
  expect "chore(law): EUR-Lex act 32024R2847R(08), corrects Regulation (EU) 2024/2847 (CRA)" "$got" "a corrigendum title"
  grep -qF 'CELEX:32024R2847R%2808%29' "$tmp/body.md" || {
    echo "eu-acts: self-test failed: the corrigendum link is not percent-encoded." >&2
    failed=1
  }

  status=0
  parse_acts <<<'{"results":{"bindings":[]}}' >/dev/null 2>&1 || status=$?
  expect "fails" "$([[ "$status" -ne 0 ]] && echo fails || echo passes)" "an empty answer"
  status=0
  parse_acts <<<'<html>maintenance</html>' >/dev/null 2>&1 || status=$?
  expect "fails" "$([[ "$status" -ne 0 ]] && echo fails || echo passes)" "a non-JSON answer"

  # Port 9 (discard) on loopback refuses the connection at once, offline.
  status=0
  got="$(fetch_acts http://127.0.0.1:9/sparql 2>&1 >/dev/null)" || status=$?
  expect "fails" "$([[ "$status" -ne 0 ]] && echo fails || echo passes)" "an unreachable source"
  [[ "$got" == *"Cellar did not answer"* ]] || {
    echo "eu-acts: self-test failed: an unreachable source printed \"$got\"." >&2
    failed=1
  }

  [[ "$failed" -eq 0 ]] || return 1
  echo "eu-acts: self-test OK."
}

mode=filing
case "${1:-}" in
  --self-test)
    self_test
    exit 0
    ;;
  --list) mode=listing ;;
  "") ;;
  *)
    echo "usage: $0 [--list | --self-test]" >&2
    exit 2
    ;;
esac

for bin in awk curl jq; do
  command -v "$bin" >/dev/null 2>&1 || { echo "eu-acts: $bin is required" >&2; exit 1; }
done

records="$(fetch_acts "$SPARQL")" || exit 1
known="$(known_acts "$KNOWN_FILE")"
missing="$(missing_known "$known" "$records")"
if [[ -n "$missing" ]]; then
  echo "eu-acts: Cellar's answer lacks acts $KNOWN_FILE lists, so the query no longer reads what it did:" >&2
  while IFS= read -r celex; do printf '  %s\n' "$celex" >&2; done <<<"$missing"
  exit 1
fi
new="$(new_acts "$known" <<<"$records")"
count="$(printf '%s\n' "$records" | grep -c .)"
if [[ -z "$new" ]]; then
  echo "eu-acts: no new act ($count known)."
  exit 0
fi

if [[ "$mode" = filing ]]; then
  command -v gh >/dev/null 2>&1 || { echo "eu-acts: gh is required to file" >&2; exit 1; }
fi
engine_dry=()
[[ "${DRY_RUN:-0}" != "1" ]] || engine_dry=(--dry-run)
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
while IFS= read -r record; do
  [[ -n "$record" ]] || continue
  title="$(render_issue "$record" "$tmp/body.md")"
  if [[ "$mode" = listing ]]; then
    printf 'NEW  %s\n' "$title"
    continue
  fi
  "$FILE_ISSUE" file --title "$title" --body-file "$tmp/body.md" --type Task --labels regulation,chore \
    --dedup-key "${record%%$'\t'*}" --state all --on-existing skip "${engine_dry[@]+"${engine_dry[@]}"}"
done <<<"$new"
