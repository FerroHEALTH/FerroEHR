# shellcheck shell=bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The FerroHEALTH badge logos: every README badge carries one. The master
# copy of this directory lives in FerroHEALTH/FerroSMART under
# scripts/checks/badge-logos/; the other repositories vendor it verbatim.
# No specification governs the badges: our own design.
#
#   badge_logo_fields NAME   the endpoint JSON fields of logo NAME: eu (the
#                            EU emblem on EU blue), openid, or a pictogram
#                            of this directory drawn as logoSvg
#   badge_logo_uri NAME      the pictogram NAME as a percent-encoded data
#                            URI, for the logo= parameter of a static badge
#   badge_logo_check FILE... fails for each endpoint badge without a logo
#   badge_logo_readme FILE   fails for each static badge without a logo
#   badge_caption_check FILE fails unless the conformance caption sits once on its
#                            own line directly beside a bold badge heading
#   badge_count_color N M [HELD]
#                            the message colour of N passing out of M, of
#                            which HELD failing ones an open upstream report
#                            holds
#   badge_color_check FILE...
#                            fails for each endpoint badge whose colour does
#                            not follow its count, or, with no count, is not
#                            the informational blue
#   badge_length_check FILE...
#                            fails for each endpoint badge whose label runs
#                            past badge_label_max characters, whose message
#                            runs past badge_message_max, or whose message
#                            carries a date or "by hand"
#   badge_length_readme FILE fails for each static shields.io badge of FILE
#                            whose label or message runs past those limits
#   badge_block_check FILE   fails when the README badge block of FILE, from
#                            its first badge to its first markdown heading,
#                            holds more than one line of prose
#   badge_render_check FILE  fails for each badge line of FILE that a
#                            CommonMark renderer shows as text: one inside
#                            an HTML block or comment
#   badge_length_self_test   the cases of the four above
#   badge_count_color_self_test
#                            the cases of badge_count_color
#   badge_post_quantum TSV   the endpoint JSON of the post-quantum badge of
#                            the inventory TSV: label post-quantum, message
#                            <safe>/<total>, coloured by the count
#   badge_post_quantum_check TSV FILE
#                            fails when the badge FILE disagrees with TSV,
#                            or TSV holds a malformed row
#   badge_post_quantum_self_test
#                            the cases of the two above
#
# A post-quantum inventory (conformance/post-quantum.tsv) has the header
# use, algorithm, safe, pq_standard, location, tab-separated, and one row
# per cryptographic use; safe is yes or no. A row whose pq_standard is
# "no standard yet" counts as not safe whatever its safe column says. The
# rows count what NIST IR 8547 (final) §4.1 and the EU coordinated
# implementation roadmap (NIS Cooperation Group, v1.1) §4.1 and §4.2 treat
# as quantum-vulnerable.
#
# Needs jq.

badge_logo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# NOTE: owner decision 2026-11-25; the badges stay short and this caption says once what they count.
badge_caption='<sub>Self-tested and self-assessed; none is a certification.</sub>'

# The pictogram NAME as one line of SVG, its comments removed.
badge_logo_svg() {
  local file="$badge_logo_dir/$1.svg"
  if [[ ! -f "$file" ]]; then
    echo "badge logo: no pictogram named $1" >&2
    return 2
  fi
  grep -v '^<!--' "$file" | tr -d '\n'
}

badge_logo_fields() {
  local svg
  case "$1" in
    eu) jq -cn '{labelColor: "#003399", namedLogo: "europeanunion", logoColor: "#FFCC00"}' ;;
    openid) jq -cn '{namedLogo: "openid", logoColor: "white"}' ;;
    *)
      svg="$(badge_logo_svg "$1")" || return 2
      jq -cn --arg svg "$svg" '{logoSvg: $svg}'
      ;;
  esac
}

badge_logo_uri() {
  local svg
  svg="$(badge_logo_svg "$1")" || return 2
  printf 'data:image/svg+xml;base64,%s' "$(printf '%s' "$svg" | base64 | tr -d '\n')" |
    sed -e 's/+/%2B/g' -e 's#/#%2F#g' -e 's/=/%3D/g' -e 's/;/%3B/g' -e 's/,/%2C/g' -e 's/:/%3A/g'
}

badge_logo_check() {
  local file failures=0
  for file in "$@"; do
    if ! jq -e '(.namedLogo // .logoSvg // "") | length > 0' "$file" > /dev/null; then
      echo "badge logo: $file carries no namedLogo or logoSvg" >&2
      failures=$((failures + 1))
    elif ! jq -e '.namedLogo != "europeanunion" or (.labelColor | ascii_upcase) == "#003399"' "$file" > /dev/null; then
      echo "badge logo: $file shows the EU emblem without the EU blue label" >&2
      failures=$((failures + 1))
    fi
  done
  [[ "$failures" -eq 0 ]]
}

badge_logo_readme() {
  local url failures=0
  while IFS= read -r url; do
    if [[ "$url" != *logo=* ]]; then
      echo "badge logo: $1 has a badge without a logo: $url" >&2
      failures=$((failures + 1))
    elif [[ "$url" == *logo=europeanunion* && "$url" != *labelColor=003399* ]]; then
      echo "badge logo: $1 shows the EU emblem without the EU blue label: $url" >&2
      failures=$((failures + 1))
    fi
  done < <(grep -oE 'https://img\.shields\.io/[^)" ]+' "$1" | grep -v '^https://img\.shields\.io/endpoint?' || true)
  [[ "$failures" -eq 0 ]]
}

badge_caption_check() {
  if ! awk -v caption="$badge_caption" '
    { line[NR] = $0 }
    $0 == caption || $0 == caption "<br>" { n++; at = NR }
    END {
      if (n != 1) exit 1
      heading = "^\\*\\*[^*]+\\*\\*(<br>)?$"
      exit !(line[at - 1] ~ heading || line[at + 1] ~ heading)
    }
  ' "$1"; then
    echo "badge logo: $1 does not hold the line $badge_caption once, beside a bold badge heading" >&2
    return 1
  fi
}

# NOTE: owner decisions 2026-11-25 and 2026-11-27; green only when all pass, yellowgreen when every missing one is held upstream.
badge_count_color() {
  local held="${3:-0}"
  if [[ "$1" -eq "$2" && "$2" -gt 0 ]]; then
    echo brightgreen
  elif [[ "$held" -gt 0 && $(($1 + held)) -eq "$2" ]]; then
    echo yellowgreen
  elif [[ "$1" -eq 0 ]]; then
    echo red
  else
    echo yellow
  fi
}

badge_count_color_self_test() {
  local pair n m held want got
  for pair in "3 3 0 brightgreen" "2 4 0 yellow" "0 4 0 red" "0 0 0 red" "2 4 2 yellowgreen" \
    "2 4 1 yellow" "0 2 2 yellowgreen" "0 2 1 red" "4 4 0 brightgreen"; do
    read -r n m held want <<< "$pair"
    got="$(badge_count_color "$n" "$m" "$held")"
    if [[ "$got" != "$want" ]]; then
      echo "badge colour: self-test FAILED: $n of $m with $held held reads $got, not $want" >&2
      return 1
    fi
  done
  echo "badge colour: self-test OK"
}

badge_color_check() {
  local file message color want failures=0
  for file in "$@"; do
    message="$(jq -r .message "$file")"
    color="$(jq -r .color "$file")"
    if [[ "$message" =~ ([0-9]+)\ ?/\ ?([0-9]+) ]]; then
      want="$(badge_count_color "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}")"
    else
      want=blue
    fi
    if [[ "$color" != "$want" ]]; then
      echo "badge logo: $file is $color, and its message '$message' asks for $want" >&2
      failures=$((failures + 1))
    fi
  done
  [[ "$failures" -eq 0 ]]
}

# NOTE: owner decision 2026-11-27; the label names the specification, the message is the count, the breakdown sits on the linked page.
badge_label_max=30
badge_message_max=20

badge_length_check() {
  local file label message failures=0
  for file in "$@"; do
    label="$(jq -r .label "$file")"
    message="$(jq -r .message "$file")"
    if [[ "$(jq '.label | length' "$file")" -gt "$badge_label_max" ]]; then
      echo "badge length: $file has a label longer than $badge_label_max characters: $label" >&2
      failures=$((failures + 1))
    fi
    if [[ "$(jq '.message | length' "$file")" -gt "$badge_message_max" ]]; then
      echo "badge length: $file has a message longer than $badge_message_max characters: $message" >&2
      failures=$((failures + 1))
    fi
    if grep -qiE '[0-9]{4}-[0-9]{2}-[0-9]{2}|by hand' <<< "$message"; then
      echo "badge length: $file names a date or 'by hand' in its message, which belongs on the linked page: $message" >&2
      failures=$((failures + 1))
    fi
  done
  [[ "$failures" -eq 0 ]]
}

# NOTE: owner decision 2026-11-27; a badge block is headings and badges, with at most one short line, the rest on the linked pages.
badge_block_max_prose=1

# Prints each prose line of the README badge block of FILE: every line from
# the first badge or the caption to the first markdown heading that is not a
# badge, a bold group heading, a break, a blank line, an HTML comment or an
# HTML tag alone.
badge_block_prose() {
  awk -v caption="$badge_caption" '
    !started && (/\[!\[/ || $0 == caption || $0 == caption "<br>") { started = 1 }
    !started { next }
    /^#/ { exit }
    /\[!\[/ { next }
    /^[[:space:]]*$/ { next }
    /^\*\*[^*]+\*\*(<br>)?$/ { next }
    /^(<br>|<br\/>|<\/?div[^>]*>|<\/?p[^>]*>|<\/?details>|<summary>.*<\/summary>|<!--.*-->)$/ { next }
    { print }
  ' "$1"
}

badge_block_check() {
  local prose count
  prose="$(badge_block_prose "$1")"
  count="$(grep -c . <<< "$prose" || true)"
  if [[ "$count" -gt "$badge_block_max_prose" ]]; then
    echo "badge block: $1 holds $count lines of prose among its badges, more than $badge_block_max_prose; move them to the page the badges link to:" >&2
    printf '  %s\n' "${prose//$'\n'/$'\n'  }" >&2
    return 1
  fi
}

# Prints each badge line of FILE that CommonMark 0.31.2 §4.6 reads as raw
# HTML, so GitHub shows it as text: a line inside an HTML block, which a
# block-level tag opens anywhere and a line holding one other tag alone opens
# outside a paragraph, running to the next blank line; or inside a comment.
badge_render_raw() {
  awk '
    BEGIN {
      block = "address|article|aside|blockquote|body|center|details|dialog|dd|div|dl|dt|fieldset|figcaption|figure|footer|form|h[1-6]|header|hr|html|li|main|nav|ol|p|section|summary|table|tbody|td|tfoot|th|thead|tr|ul"
    }
    comment { if (/\[!\[/) print NR ": " $0; if (/-->/) { comment = 0; para = 0 } next }
    html { if (/^[[:space:]]*$/) { html = 0; para = 0 } else if (/\[!\[/) print NR ": " $0; next }
    /^[[:space:]]*$/ { para = 0; next }
    /^ {0,3}<!--/ { if (!/-->/) comment = 1; para = 0; next }
    $0 ~ "^ {0,3}</?(" block ")([[:space:]/>]|$)" { html = 1; next }
    !para && /^ {0,3}(<[A-Za-z][A-Za-z0-9-]*([[:space:]]+[^<>]*)?\/?>|<\/[A-Za-z][A-Za-z0-9-]*[[:space:]]*>)[[:space:]]*$/ { html = 1; next }
    { para = 1 }
  ' "$1"
}

badge_render_check() {
  local raw
  raw="$(badge_render_raw "$1")"
  if [[ -n "$raw" ]]; then
    echo "badge render: $1 has badges that CommonMark reads as raw HTML, so GitHub shows them as text; end the HTML block with a blank line:" >&2
    printf '  %s\n' "${raw//$'\n'/$'\n'  }" >&2
    return 1
  fi
}

# Prints "<label><TAB><message>" of each static shields.io badge in FILE: the
# /badge/LABEL-MESSAGE-COLOR path form, with -- for a dash, __ for an
# underscore, _ for a space and percent-encoding (https://shields.io/badges/static-badge).
badge_static_texts() {
  local url path part label message
  while IFS= read -r url; do
    path="${url#https://img.shields.io/badge/}"
    path="${path%%\?*}"
    path="${path//--/$'\x01'}"
    path="${path//__/$'\x02'}"
    IFS='-' read -r -a part <<< "$path"
    if [[ "${#part[@]}" -ge 3 ]]; then
      label="${part[0]}"
      message="${part[1]}"
    else
      label=""
      message="${part[0]:-}"
    fi
    for part in label message; do
      local value="${!part}"
      value="${value//_/ }"
      value="${value//$'\x01'/-}"
      value="${value//$'\x02'/_}"
      printf -v "$part" '%b' "${value//%/\\x}"
    done
    printf '%s\t%s\n' "$label" "$message"
  done < <(grep -oE 'https://img\.shields\.io/badge/[^)" ]+' "$1" || true)
}

badge_length_readme() {
  local label message failures=0
  while IFS=$'\t' read -r label message; do
    if [[ "$(jq -rn --arg s "$label" '$s | length')" -gt "$badge_label_max" ]]; then
      echo "badge length: $1 has a static badge whose label runs past $badge_label_max characters: $label" >&2
      failures=$((failures + 1))
    fi
    if [[ "$(jq -rn --arg s "$message" '$s | length')" -gt "$badge_message_max" ]]; then
      echo "badge length: $1 has a static badge whose message runs past $badge_message_max characters: $message" >&2
      failures=$((failures + 1))
    fi
  done < <(badge_static_texts "$1")
  [[ "$failures" -eq 0 ]]
}

badge_length_self_test() {
  local work
  work="$(mktemp -d)"
  badge_length_case() {
    jq -cn --arg label "$1" --arg message "$2" '{schemaVersion: 1, label: $label, message: $message, color: "blue"}' > "$work/b.json"
    if badge_length_check "$work/b.json" 2> /dev/null; then
      [[ "$3" == pass ]] && return 0
    else
      [[ "$3" == fail ]] && return 0
    fi
    echo "badge length: self-test FAILED: label '$1' and message '$2' should $3" >&2
    rm -f "$work/b.json"
    rmdir "$work"
    return 1
  }
  badge_length_case "Federation Tier 1.0.1" "364/383" pass || return 1
  badge_length_case "$(printf 'x%.0s' {1..30})" "$(printf '9%.0s' {1..20})" pass || return 1
  badge_length_case "OpenID Connect Core 1.0 · OIDF" "1/2" pass || return 1
  badge_length_case "$(printf 'x%.0s' {1..31})" "1/2" fail || return 1
  badge_length_case "OIDC Core 1.0 OP" "$(printf '9%.0s' {1..21})" fail || return 1
  badge_length_case "OIDC Core 1.0 OP" "3/36 passed, 30 failed; open: #933" fail || return 1
  printf '[![a](https://img.shields.io/badge/CRA-user%%20information-blue?logo=x)](a)\n' > "$work/r.md"
  printf '[![b](https://img.shields.io/badge/License-BUSL--1.1-blue?logo=x)](b)\n' >> "$work/r.md"
  if ! badge_length_readme "$work/r.md" 2> /dev/null; then
    echo "badge length: self-test FAILED: a short static badge is refused" >&2
    rm -f "$work/b.json" "$work/r.md"
    rmdir "$work"
    return 1
  fi
  if [[ "$(badge_static_texts "$work/r.md" | tail -1)" != $'License\tBUSL-1.1' ]]; then
    echo "badge length: self-test FAILED: a static badge reads $(badge_static_texts "$work/r.md" | tail -1)" >&2
    rm -f "$work/b.json" "$work/r.md"
    rmdir "$work"
    return 1
  fi
  printf '[![c](https://img.shields.io/badge/CRA-information_for__all%%20of%%20its%%20users-blue)](c)\n' > "$work/r.md"
  if badge_length_readme "$work/r.md" 2> /dev/null; then
    echo "badge length: self-test FAILED: a long static badge message passes" >&2
    rm -f "$work/b.json" "$work/r.md"
    rmdir "$work"
    return 1
  fi
  badge_length_case "IHE ATNA interoperability" "3/3 2026-10-29" fail || return 1
  badge_length_case "tx 1.11.0 snomed R4B" "2/378, by hand" fail || return 1
  badge_length_case "Xt-EHR D8.2 2026-05-19" "11/11 evidenced" pass || return 1
  printf '# T\n\n[![a](x)](y)\n\n**Group**<br>\n<sub>Self-tested; none is a certification.</sub><br>\n[![b](x)](y)\n<br>\n<!-- c -->\n</div>\n\n## Next\n\nprose\nmore prose\n' > "$work/r.md"
  if ! badge_block_check "$work/r.md" 2> /dev/null; then
    echo "badge block: self-test FAILED: a block with one line of prose is refused" >&2
    rm -f "$work/b.json" "$work/r.md"
    rmdir "$work"
    return 1
  fi
  printf '[![a](x)](y)\n<sub>one</sub><br>\n**Group**<br>\n<sub>the red rows are a deviation</sub><br>\n[![b](x)](y)\n' > "$work/r.md"
  if badge_block_check "$work/r.md" 2> /dev/null; then
    echo "badge block: self-test FAILED: a block with two lines of prose passes" >&2
    rm -f "$work/b.json" "$work/r.md"
    rmdir "$work"
    return 1
  fi
  printf '<div align="center">\n\n**G**<br>\n[![a](x)](y)\n<br>\n**H**<br>\n[![b](x)](y)\n<!-- m -->\n[![c](x)](y)\n<details>\n<summary>s</summary>\n\n[![d](x)](y)\n\n</details>\n</div>\n' > "$work/r.md"
  if ! badge_render_check "$work/r.md" 2> /dev/null; then
    echo "badge render: self-test FAILED: a README whose badges render is refused" >&2
    rm -f "$work/b.json" "$work/r.md"
    rmdir "$work"
    return 1
  fi
  local raw_case
  for raw_case in '<!-- m -->\n<br>\n**H**<br>\n[![a](x)](y)\n' '<div align="center">\n[![a](x)](y)\n' '<!-- open\n[![a](x)](y)\n-->\n'; do
    printf '%b' "$raw_case" > "$work/r.md"
    if badge_render_check "$work/r.md" 2> /dev/null; then
      echo "badge render: self-test FAILED: a badge inside an HTML block passes: $raw_case" >&2
      rm -f "$work/b.json" "$work/r.md"
      rmdir "$work"
      return 1
    fi
  done
  rm -f "$work/b.json" "$work/r.md"
  rmdir "$work"
  echo "badge length: self-test OK"
}

# The header of a post-quantum inventory.
badge_post_quantum_header=$'use\talgorithm\tsafe\tpq_standard\tlocation'

# Prints "<safe> <total>" of the post-quantum inventory $1, or fails naming
# the first malformed row.
badge_post_quantum_count() {
  awk -F'\t' -v header="$badge_post_quantum_header" '
    /^(#|$)/ { next }
    !seen { seen = 1; if ($0 != header) { print "post-quantum: the header is not " header > "/dev/stderr"; bad = 1; exit } next }
    NF != 5 { printf "post-quantum: line %d has %d columns, not 5\n", NR, NF > "/dev/stderr"; bad = 1; exit }
    $3 != "yes" && $3 != "no" { printf "post-quantum: line %d says safe is %s, not yes or no\n", NR, $3 > "/dev/stderr"; bad = 1; exit }
    { total++; if ($3 == "yes" && $4 != "no standard yet") safe++ }
    END {
      if (bad) exit 1
      if (!seen) { print "post-quantum: the inventory is empty" > "/dev/stderr"; exit 1 }
      print safe + 0, total + 0
    }
  ' "$1"
}

badge_post_quantum() {
  local counts safe total color logo
  counts="$(badge_post_quantum_count "$1")" || return 1
  read -r safe total <<< "$counts"
  color="$(badge_count_color "$safe" "$total")"
  logo="$(badge_logo_fields key)"
  jq -cn --arg message "$safe/$total" --arg color "$color" --argjson logo "$logo" \
    '{schemaVersion: 1, label: "post-quantum", message: $message, color: $color} + $logo'
}

badge_post_quantum_check() {
  local fresh
  fresh="$(badge_post_quantum "$1")" || return 1
  if [[ "$(jq -c . "$2")" != "$fresh" ]]; then
    echo "post-quantum: $2 is $(jq -c '{message, color}' "$2"), and $1 counts $(jq -c '{message, color}' <<< "$fresh")" >&2
    return 1
  fi
}

badge_post_quantum_self_test() {
  local work header
  work="$(mktemp -d)"
  header="$badge_post_quantum_header"
  badge_pq_row() { printf '%s\t%s\t%s\t%s\t%s\n' "$@"; }
  badge_pq_expect() {
    local message color
    message="$(badge_post_quantum "$work/pq.tsv" | jq -r .message)"
    color="$(badge_post_quantum "$work/pq.tsv" | jq -r .color)"
    if [[ "$message $color" != "$1 $2" ]]; then
      echo "post-quantum: self-test FAILED: $3 reads $message $color, not $1 $2" >&2
      rm -f "$work/pq.tsv" "$work/pq.json"
      rmdir "$work"
      return 1
    fi
  }
  badge_pq_refused() {
    if badge_post_quantum "$work/pq.tsv" > /dev/null 2>&1; then
      echo "post-quantum: self-test FAILED: $1 passes" >&2
      rm -f "$work/pq.tsv" "$work/pq.json"
      rmdir "$work"
      return 1
    fi
  }
  { echo "$header"; badge_pq_row tls X25519MLKEM768 yes 'RFC 10024' a; badge_pq_row tokens ML-DSA-65 yes 'RFC 9964' b; } > "$work/pq.tsv"
  badge_pq_expect 2/2 brightgreen "every row safe" || return 1
  { echo "$header"; badge_pq_row tls X25519MLKEM768 yes 'RFC 10024' a; badge_pq_row tokens RS256 no 'RFC 9964' b; } > "$work/pq.tsv"
  badge_pq_expect 1/2 yellow "some rows safe" || return 1
  { echo "$header"; badge_pq_row tls X25519 no 'RFC 10024' a; badge_pq_row tokens RS256 no 'RFC 9964' b; } > "$work/pq.tsv"
  badge_pq_expect 0/2 red "no row safe" || return 1
  { echo "$header"; badge_pq_row tls X25519MLKEM768 yes 'RFC 10024' a; badge_pq_row saml RSA yes 'no standard yet' b; } > "$work/pq.tsv"
  badge_pq_expect 1/2 yellow "a no-standard-yet row marked yes" || return 1
  { echo "$header"; badge_pq_row tls X25519 no 'RFC 10024' a; printf 'tokens\tRS256\tno\tRFC 9964\n'; } > "$work/pq.tsv"
  badge_pq_refused "a row of four columns" || return 1
  { echo "$header"; badge_pq_row tls X25519 maybe 'RFC 10024' a; } > "$work/pq.tsv"
  badge_pq_refused "a safe column of maybe" || return 1
  { echo "use"; badge_pq_row tls X25519 no 'RFC 10024' a; } > "$work/pq.tsv"
  badge_pq_refused "another header" || return 1
  { echo "$header"; badge_pq_row tls X25519MLKEM768 yes 'RFC 10024' a; badge_pq_row tokens RS256 no 'RFC 9964' b; } > "$work/pq.tsv"
  badge_post_quantum "$work/pq.tsv" > "$work/pq.json"
  badge_post_quantum_check "$work/pq.tsv" "$work/pq.json" || {
    echo "post-quantum: self-test FAILED: a fresh badge is refused" >&2
    return 1
  }
  jq -c '.message = "2/2" | .color = "brightgreen"' "$work/pq.json" > "$work/pq.tsv.json"
  mv "$work/pq.tsv.json" "$work/pq.json"
  if badge_post_quantum_check "$work/pq.tsv" "$work/pq.json" 2> /dev/null; then
    echo "post-quantum: self-test FAILED: a badge whose count disagrees passes" >&2
    return 1
  fi
  jq -c '.message = "1/2" | .color = "brightgreen"' "$work/pq.json" > "$work/pq.tsv.json"
  mv "$work/pq.tsv.json" "$work/pq.json"
  if badge_post_quantum_check "$work/pq.tsv" "$work/pq.json" 2> /dev/null; then
    echo "post-quantum: self-test FAILED: a badge whose colour disagrees passes" >&2
    return 1
  fi
  rm -f "$work/pq.tsv" "$work/pq.json"
  rmdir "$work"
  echo "post-quantum: self-test OK"
}
