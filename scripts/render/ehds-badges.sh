#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The EHDS badges (#6537): shields.io endpoint documents under
# docs/conformance/ehds/, counted from the book's evidence tables.
#
# - logging.json: the rows of compliance/logging-checklist.md whose state is
#   Evidenced and whose every cited test function exists.
# - annex-iii.json: the elements of compliance/eehrxf-annex-iii.md whose state
#   is Sourced; the page's table names those Partly sourced and those with No
#   RM source, which FerroBRIDGE supplies (#6750).
# - pdf-originals.json: the round trips of compliance/eehrxf-pdf-originals.md
#   whose state is Byte-identical and whose test function exists.
#
# A badge names its source and the source's date and shows N/M, within the
# family's length limits (owner decision, 2026-11-27): the rest of the count
# is the linked page's table. No label or
# message may say certified, compliant or conformant, and the README's EHDS
# badges are held to the same words. Every badge carries the EU emblem on EU
# blue, and every static badge of the README a logo, from
# scripts/checks/badge-logos/. The small caption under the README's badge
# heading says once what the badges count (owner decision, 2026-11-25).
#
# Usage: scripts/render/ehds-badges.sh              rewrite the documents
#        scripts/render/ehds-badges.sh --check      fail on a stale document, a forbidden word or a missing logo
#        scripts/render/ehds-badges.sh --self-test  the counting and the word guard on fixtures
# No specification governs the badges: our own design.
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

BOOK=website/book/src/compliance
OUT=docs/conformance/ehds
FORBIDDEN='certif|complian|conforman'

# Prints `<total> <counted>` for the table rows of $1 whose first cell is code:
# a row counts when its last cell is $2 and every test function it cites, as
# `path.rs` (`fn`, ...) or a bare `fn` beside $3, exists in that file. The
# tree is read from the current directory.
count_rows() {
  perl -e '
    my ($file, $want, $default) = @ARGV;
    open my $fh, "<", $file or die "$file: $!\n";
    my ($total, $counted) = (0, 0);
    while (my $row = <$fh>) {
      next unless $row =~ /^\|\s*`/;
      $total++;
      my @cells = split /\|/, $row;
      my $state = $cells[-2] // "";
      $state =~ s/^\s+|\s+$//g;
      next unless $state eq $want;
      my $ok = 1;
      my %cited;
      while ($row =~ /`((?:app|crates|tools)\/[^`]+\.rs)` \(([^)]*)\)/g) {
        my ($path, $list) = ($1, $2);
        push @{$cited{$path}}, $list =~ /`([a-z0-9_]+)`/g;
      }
      if ($default ne "" && !%cited) {
        push @{$cited{$default}}, $row =~ /`([a-z][a-z0-9_]*_[a-z0-9_]+)`/g;
      }
      for my $path (keys %cited) {
        my $src = do { local $/; open my $s, "<", $path or do { $ok = 0; next }; <$s> };
        for my $fn (@{$cited{$path}}) { $ok = 0 unless $src =~ /\bfn \Q$fn\E\b/ }
      }
      $counted++ if $ok;
    }
    print "$total $counted\n";
  ' "$1" "$2" "${3:-}"
}

# Writes the endpoint document $1 with label $2 and message $3, coloured by
# the count $4 of $5 (owner decision, 2026-11-25).
badge() {
  jq -n --arg label "$2" --arg message "$3" --arg color "$(badge_count_color "$4" "$5")" \
    --argjson logo "$(badge_logo_fields eu)" \
    '{schemaVersion: 1, label: $label, message: $message, color: $color} + $logo' > "$1"
}

# Writes the three documents into the directory $1 from the tree in the
# current directory.
render() {
  local dir="$1" total counted
  mkdir -p "$dir"
  read -r total counted < <(count_rows "$BOOK/logging-checklist.md" Evidenced)
  badge "$dir/logging.json" "Xt-EHR D8.2 2026-05-19" "$counted/$total evidenced" "$counted" "$total"
  read -r total counted < <(count_rows "$BOOK/eehrxf-annex-iii.md" Sourced)
  badge "$dir/annex-iii.json" "EEHRxF 2026-11-23 Annex III" "$counted/$total sourced" "$counted" "$total"
  read -r total counted < <(count_rows "$BOOK/eehrxf-pdf-originals.md" Byte-identical \
    app/ferroehr-rest/tests/it/pdf_original_http.rs)
  badge "$dir/pdf-originals.json" "EEHRxF 2026-11-23 PDF/A" "$counted/$total byte-identical" "$counted" "$total"
  badge_length_check "$dir"/*.json
}

# Prints each badge label or message under $1, and each README badge of the
# EHDS group, that uses a forbidden word.
forbidden_words() {
  local readme="$2"
  jq -r '.label, .message' "$1"/*.json | grep -iE "$FORBIDDEN" || true
  grep -E '^\[!\[[^]]*(EHDS|EEHRxF|xShare)' "$readme" | grep -ioE '^\[!\[[^]]*\]' \
    | grep -iE "$FORBIDDEN" || true
}

if [[ "$mode" == self-test ]]; then
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  mkdir -p "$tmp/$BOOK" "$tmp/app/x/src" "$tmp/app/ferroehr-rest/tests/it"
  printf 'fn a_test() {}\n' > "$tmp/app/x/src/lib.rs"
  printf 'fn kept_round_trip() {}\n' > "$tmp/app/ferroehr-rest/tests/it/pdf_original_http.rs"
  cat > "$tmp/$BOOK/logging-checklist.md" <<'EOF'
| Item | EHDS | Evidence | State |
|---|---|---|---|
| `one` | 3.2 | `app/x/src/lib.rs` (`a_test`) | Evidenced |
| `two` | 3.2 | `app/x/src/lib.rs` (`a_gone_test`) | Evidenced |
| `three` | 3.3 | a gap | Gap |
EOF
  cat > "$tmp/$BOOK/eehrxf-annex-iii.md" <<'EOF'
| C | `GET` | not an element row, its first cell is no code |
| `subject` | 1 | x | C | Sourced |
| `period` | 1 | x | C | Partly sourced |
| `custodian` | 1 | x | E | No RM source |
EOF
  cat > "$tmp/$BOOK/eehrxf-pdf-originals.md" <<'EOF'
| `inline` | `kept_round_trip` | Byte-identical |
| `xml` | `gone_round_trip` | Byte-identical |
EOF
  (cd "$tmp" && BOOK="$BOOK" render out)
  for want in 'logging.json 1/3 evidenced' 'annex-iii.json 1/3 sourced' \
    'pdf-originals.json 1/2 byte-identical'; do
    got=$(jq -r .message "$tmp/out/${want%% *}")
    [[ "$got" == "${want#* }" ]] || { echo "self-test: ${want%% *} says '$got', want '${want#* }'" >&2; exit 1; }
  done
  printf '[![EHDS logging](x)](y)\n' > "$tmp/README.md"
  [[ -z "$(forbidden_words "$tmp/out" "$tmp/README.md")" ]] || { echo "self-test: clean badges flagged" >&2; exit 1; }
  badge "$tmp/out/bad.json" "EHDS certified" "1/1" 1 1
  [[ -n "$(forbidden_words "$tmp/out" "$tmp/README.md")" ]] || { echo "self-test: a certified label passed" >&2; exit 1; }
  rm "$tmp/out/bad.json"
  badge_logo_check "$tmp"/out/*.json || { echo "self-test: a rendered badge carries no EU emblem on EU blue" >&2; exit 1; }
  jq 'del(.labelColor)' "$tmp/out/logging.json" > "$tmp/bare.json"
  if badge_logo_check "$tmp/bare.json" 2> /dev/null; then echo "self-test: an EU emblem without the EU blue label passed" >&2; exit 1; fi
  printf '[![x](https://img.shields.io/badge/x-y-blue)](x)\n' > "$tmp/README.md"
  if badge_logo_readme "$tmp/README.md" 2> /dev/null; then echo "self-test: a static badge without a logo passed" >&2; exit 1; fi
  printf '**EHDS**<br>\n%s<br>\n' "$badge_caption" > "$tmp/README.md"
  badge_caption_check "$tmp/README.md" || { echo "self-test: the conformance caption was not found" >&2; exit 1; }
  badge_color_check "$tmp"/out/*.json || { echo "self-test: a rendered badge's colour does not follow its count" >&2; exit 1; }
  for want in "0 2 red" "1 2 yellow" "2 2 brightgreen"; do
    read -r n m colour <<< "$want"
    badge "$tmp/colour.json" "EHDS logging checklist" "$n/$m evidenced" "$n" "$m"
    [[ "$(jq -r .color "$tmp/colour.json")" == "$colour" ]] || { echo "self-test: $n of $m is not $colour" >&2; exit 1; }
  done
  jq '.color = "blue"' "$tmp/colour.json" > "$tmp/blue.json"
  if badge_color_check "$tmp/blue.json" 2> /dev/null; then echo "self-test: a counted badge in blue passed" >&2; exit 1; fi
  printf '**EHDS**<br>\n' > "$tmp/README.md"
  if badge_caption_check "$tmp/README.md" 2> /dev/null; then echo "self-test: a README without the conformance caption passed" >&2; exit 1; fi
  printf '[![EEHRxF conformant](x)](y)\n' > "$tmp/README.md"
  [[ -n "$(forbidden_words "$tmp/out" "$tmp/README.md")" ]] || { echo "self-test: a conformant README badge passed" >&2; exit 1; }
  badge_length_check "$tmp"/out/*.json || { echo "self-test: a rendered badge runs past the length limits" >&2; exit 1; }
  badge "$tmp/long.json" "EEHRxF draft 2026-11-23 · Annex III from the RM" "1/3 sourced" 1 3
  if badge_length_check "$tmp/long.json" 2> /dev/null; then echo "self-test: a label past 30 characters passed" >&2; exit 1; fi
  badge_length_self_test > /dev/null || exit 1
  echo "ehds-badges: self-test passed"
  exit 0
fi

if [[ "$mode" == write ]]; then
  render "$OUT"
  echo "ehds-badges: wrote $OUT"
  exit 0
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
render "$tmp"
fail=0
for doc in "$tmp"/*.json; do
  name=$(basename "$doc")
  if ! cmp -s "$doc" "$OUT/$name"; then
    echo "::error::$OUT/$name is not what the book's table says: run scripts/render/ehds-badges.sh and commit it" >&2
    fail=1
  fi
done
words=$(forbidden_words "$OUT" README.md)
if [[ -n "$words" ]]; then
  echo "$words" >&2
  echo "::error::an EHDS badge says certified, compliant or conformant, which no conformity assessment supports (EHDS Art. 28)" >&2
  fail=1
fi
badge_logo_check "$OUT"/*.json || fail=1
badge_logo_readme README.md || fail=1
badge_caption_check README.md || fail=1
badge_color_check "$OUT"/*.json || fail=1
badge_length_check "$OUT"/*.json || fail=1
[[ "$fail" -eq 0 ]] && echo "ehds-badges: the badges match their tables, carry their logos and claim no certification"
exit "$fail"
