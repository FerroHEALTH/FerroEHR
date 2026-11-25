#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The post-quantum inventory and its badge (#6602).
#
# conformance/post-quantum.tsv lists every cryptographic use, after its `#`
# comment lines, in five tab-separated columns: use, algorithm, safe (yes or
# no), pq_standard and location. The script holds the file's shape and every
# repository path a location names to the tree, and writes the table of
# docs/post-quantum.md and the shields.io endpoint document
# conformance/badges/post-quantum.json from it.
#
# A row counts as safe when its safe column is yes and its pq_standard names
# a standard; a `no standard yet` row counts as not safe. The badge colour is
# the shared count rule of scripts/checks/badge-logos/logo.sh, its logo the
# key pictogram there, and neither its label nor the README badge may say
# certified, compliant or conformant.
#
# Usage: scripts/render/post-quantum.sh              rewrite the table and the badge
#        scripts/render/post-quantum.sh --check      fail on a defect or a stale table or badge
#        scripts/render/post-quantum.sh --self-test  the rules on fixtures
# Needs jq. No specification governs the inventory format: our own design,
# after FerroSYS's.
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

INVENTORY=conformance/post-quantum.tsv
PAGE=docs/post-quantum.md
BADGE=conformance/badges/post-quantum.json
HEADER=$'use\talgorithm\tsafe\tpq_standard\tlocation'
LABEL="post-quantum safe uses"
NO_STANDARD="no standard yet"
FORBIDDEN='certif|complian|conforman'

# Prints each defect of inventory $1 under the tree $2; status 1 when any.
shape_of() {
  local inventory="$1" tree="$2" failed=0 line=0 use algorithm safe standard location extra item path rest
  if [[ "$(grep -v '^#' "$inventory" | sed -n 1p)" != "$HEADER" ]]; then
    echo "post-quantum: $inventory does not open with the header line" >&2
    failed=1
  fi
  while IFS=$'\t' read -r use algorithm safe standard location extra; do
    line=$((line + 1))
    [[ "$line" -eq 1 ]] && continue
    if [[ -z "$use" || -z "$algorithm" || -z "$standard" || -z "$location" || -n "$extra" ]]; then
      echo "post-quantum: $inventory row $line does not hold five filled columns" >&2
      failed=1
      continue
    fi
    if [[ "$safe" != yes && "$safe" != no ]]; then
      echo "post-quantum: $inventory row $line says safe '$safe'; it is yes or no" >&2
      failed=1
    fi
    rest="$location"
    while [[ -n "$rest" ]]; do
      item="${rest%%, *}"
      if [[ "$rest" == *", "* ]]; then rest="${rest#*, }"; else rest=""; fi
      path="${item%% (*}"
      if [[ "$path" == */* ]] && [[ ! -e "$tree/$path" ]]; then
        echo "post-quantum: $inventory row $line names $path, which the tree does not hold" >&2
        failed=1
      fi
    done
  done < <(grep -v '^#' "$inventory")
  if [[ "$line" -lt 2 ]]; then
    echo "post-quantum: $inventory lists no use" >&2
    failed=1
  fi
  return "$failed"
}

# The rows of inventory $1 that count as safe, and all its rows: "N M".
count_of() {
  awk -F'\t' -v none="$NO_STANDARD" '
    /^#/ { next }
    seen++ { total++; if ($3 == "yes" && $4 != none) safe++ }
    END { printf "%d %d\n", safe, total }
  ' "$1"
}

# The endpoint document inventory $1 implies.
badge_of() {
  local safe total
  read -r safe total < <(count_of "$1")
  jq -cn --arg label "$LABEL" --arg message "$safe/$total" \
    --arg color "$(badge_count_color "$safe" "$total")" --argjson logo "$(badge_logo_fields key)" \
    '{schemaVersion: 1, label: $label, message: $message, color: $color} + $logo'
}

# The markdown table inventory $1 implies.
table_of() {
  echo '| Use | Algorithm | Safe | Post-quantum standard | Location |'
  echo '|---|---|---|---|---|'
  awk -F'\t' '/^#/ { next } seen++ { printf "| %s | %s | %s | %s | `%s` |\n", $1, $2, $3, $4, $5 }' "$1"
}

# The table of page $1: the lines from its header row to the first line that
# is no table row.
page_table() {
  awk '/^\| Use \| Algorithm \|/ { inside = 1 } inside && !/^\|/ { exit } inside { print }' "$1"
}

# Page $1 with its table replaced by the one inventory $2 implies.
page_with_table() {
  local rendered
  rendered="$(mktemp)"
  table_of "$2" > "$rendered"
  awk -v table="$rendered" '
    /^\| Use \| Algorithm \|/ && !done { while ((getline line < table) > 0) print line; skipping = 1; done = 1; next }
    skipping && /^\|/ { next }
    { skipping = 0; print }
  ' "$1"
  rm -f "$rendered"
}

# Prints the badge label of document $1 and the post-quantum badge alt text
# of README $2 when either uses a forbidden word.
forbidden_words() {
  jq -r '.label, .message' "$1" | grep -iE "$FORBIDDEN" || true
  grep -oE '^\[!\[[^]]*[Pp]ost-quantum[^]]*\]' "$2" | grep -iE "$FORBIDDEN" || true
}

# Checks inventory $1 under tree $2 against page $3, badge $4 and README $5;
# status 1 on any finding.
check() {
  local inventory="$1" tree="$2" page="$3" badge="$4" readme="$5" failed=0 words
  shape_of "$inventory" "$tree" || failed=1
  if [[ -z "$(page_table "$page")" ]]; then
    echo "post-quantum: $page has no inventory table" >&2
    failed=1
  elif [[ "$(page_table "$page")" != "$(table_of "$inventory")" ]]; then
    echo "::error::the $page table is not what $inventory says: run scripts/render/post-quantum.sh and commit it" >&2
    failed=1
  fi
  if [[ ! -f "$badge" ]] || [[ "$(cat "$badge")" != "$(badge_of "$inventory")" ]]; then
    echo "::error::$badge is not what $inventory counts: run scripts/render/post-quantum.sh and commit it" >&2
    failed=1
  else
    badge_logo_check "$badge" || failed=1
    badge_color_check "$badge" || failed=1
  fi
  if ! grep -qF 'conformance%2Fbadges%2Fpost-quantum.json' "$readme"; then
    echo "post-quantum: $readme carries no post-quantum badge" >&2
    failed=1
  fi
  words="$(forbidden_words "$badge" "$readme" 2> /dev/null)"
  if [[ -n "$words" ]]; then
    echo "$words" >&2
    echo "::error::the post-quantum badge says certified, compliant or conformant, which no assessment supports" >&2
    failed=1
  fi
  return "$failed"
}

if [[ "$mode" == self-test ]]; then
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  fail() { echo "post-quantum: self-test failed: $1" >&2; exit 1; }
  mkdir -p "$tmp/tree/src"
  : > "$tmp/tree/src/kx.rs"
  {
    printf '# SPDX-FileCopyrightText: Cadasto B.V.\n'
    printf '%s\n' "$HEADER"
    printf 'Key exchange\tX25519MLKEM768\tyes\tRFC 10024\tsrc/kx.rs (provider), a-crate tls\n'
    printf 'Record protection\tAES-256-GCM\tyes\tNIST IR 8547 4.1.3\ta-crate tls\n'
    printf 'Certificates\tECDSA\tyes\t%s\ta-crate tls\n' "$NO_STANDARD"
    printf 'Tokens\tES256\tno\tRFC 9964\ta-crate resource\n'
  } > "$tmp/inventory.tsv"
  shape_of "$tmp/inventory.tsv" "$tmp/tree" || fail "a well formed inventory was refused"
  sed 's#src/kx.rs#src/gone.rs#' "$tmp/inventory.tsv" > "$tmp/missing.tsv"
  if shape_of "$tmp/missing.tsv" "$tmp/tree" 2> /dev/null; then fail "a location the tree does not hold passed"; fi
  sed $'s#\tno\t#\tmaybe\t#' "$tmp/inventory.tsv" > "$tmp/maybe.tsv"
  if shape_of "$tmp/maybe.tsv" "$tmp/tree" 2> /dev/null; then fail "a safe column of maybe passed"; fi
  sed $'$s#\ta-crate resource$##' "$tmp/inventory.tsv" > "$tmp/short.tsv"
  if shape_of "$tmp/short.tsv" "$tmp/tree" 2> /dev/null; then fail "a row of four columns passed"; fi
  sed '2s#^use#purpose#' "$tmp/inventory.tsv" > "$tmp/header.tsv"
  if shape_of "$tmp/header.tsv" "$tmp/tree" 2> /dev/null; then fail "a file without the header passed"; fi
  [[ "$(count_of "$tmp/inventory.tsv")" == "2 4" ]] || fail "a 'no standard yet' row counted as safe"
  badge_of "$tmp/inventory.tsv" > "$tmp/badge.json"
  [[ "$(jq -c 'del(.logoSvg)' "$tmp/badge.json")" == "{\"schemaVersion\":1,\"label\":\"$LABEL\",\"message\":\"2/4\",\"color\":\"yellow\"}" ]] \
    || fail "the badge of two safe rows of four is $(cat "$tmp/badge.json")"
  badge_logo_check "$tmp/badge.json" || fail "the badge carries no logo"
  badge_color_check "$tmp/badge.json" || fail "the badge colour does not follow its count"
  printf '%s\n' "$HEADER" > "$tmp/none.tsv"
  awk -F'\t' '$3 == "no"' "$tmp/inventory.tsv" >> "$tmp/none.tsv"
  [[ "$(badge_of "$tmp/none.tsv" | jq -r .color)" == red ]] || fail "an inventory with no safe row is not red"
  printf '%s\n' "$HEADER" > "$tmp/all.tsv"
  sed -n '3p' "$tmp/inventory.tsv" >> "$tmp/all.tsv"
  [[ "$(badge_of "$tmp/all.tsv" | jq -r .color)" == brightgreen ]] || fail "an inventory of safe rows only is not green"
  printf '# Page\n\nintro\n\n| Use | Algorithm | Safe | Post-quantum standard | Location |\n|---|---|---|---|---|\n| stale | x | no | y | z |\n\nrest\n' > "$tmp/page.md"
  printf '[![Post-quantum safe uses](https://img.shields.io/endpoint?url=x%%2Fconformance%%2Fbadges%%2Fpost-quantum.json)](docs/post-quantum.md)\n' > "$tmp/README.md"
  if check "$tmp/inventory.tsv" "$tmp/tree" "$tmp/page.md" "$tmp/badge.json" "$tmp/README.md" 2> /dev/null; then
    fail "a stale table passed"
  fi
  page_with_table "$tmp/page.md" "$tmp/inventory.tsv" > "$tmp/fresh.md"
  [[ "$(sed -n 3p "$tmp/fresh.md")" == intro && "$(tail -n 1 "$tmp/fresh.md")" == rest ]] \
    || fail "the rewrite touched text outside the table"
  check "$tmp/inventory.tsv" "$tmp/tree" "$tmp/fresh.md" "$tmp/badge.json" "$tmp/README.md" \
    || fail "a fresh table, badge and README were refused"
  jq '.color = "brightgreen"' "$tmp/badge.json" > "$tmp/green.json"
  if check "$tmp/inventory.tsv" "$tmp/tree" "$tmp/fresh.md" "$tmp/green.json" "$tmp/README.md" 2> /dev/null; then
    fail "a stale badge passed"
  fi
  printf '# Page\n' > "$tmp/bare.md"
  if check "$tmp/inventory.tsv" "$tmp/tree" "$tmp/bare.md" "$tmp/badge.json" "$tmp/README.md" 2> /dev/null; then
    fail "a page without a table passed"
  fi
  printf 'no badge\n' > "$tmp/plain.md"
  if check "$tmp/inventory.tsv" "$tmp/tree" "$tmp/fresh.md" "$tmp/badge.json" "$tmp/plain.md" 2> /dev/null; then
    fail "a README without the badge passed"
  fi
  printf '[![Post-quantum certified](https://img.shields.io/endpoint?url=x%%2Fconformance%%2Fbadges%%2Fpost-quantum.json)](y)\n' > "$tmp/claims.md"
  if check "$tmp/inventory.tsv" "$tmp/tree" "$tmp/fresh.md" "$tmp/badge.json" "$tmp/claims.md" 2> /dev/null; then
    fail "a README badge that says certified passed"
  fi
  jq '.label = "post-quantum compliant"' "$tmp/badge.json" > "$tmp/compliant.json"
  [[ -n "$(forbidden_words "$tmp/compliant.json" "$tmp/README.md")" ]] || fail "a compliant label passed"
  echo "post-quantum: self-test passed"
  exit 0
fi

if [[ "$mode" == write ]]; then
  shape_of "$INVENTORY" .
  fresh="$(page_with_table "$PAGE" "$INVENTORY")"
  printf '%s\n' "$fresh" > "$PAGE"
  mkdir -p "$(dirname "$BADGE")"
  badge_of "$INVENTORY" > "$BADGE"
  echo "post-quantum: wrote the $PAGE table and $BADGE"
  exit 0
fi

check "$INVENTORY" . "$PAGE" "$BADGE" README.md
read -r safe total < <(count_of "$INVENTORY")
echo "post-quantum: $safe of $total uses safe; the table and the badge match the inventory"
