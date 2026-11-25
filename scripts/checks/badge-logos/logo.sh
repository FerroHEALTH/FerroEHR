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
#   badge_caption_check FILE fails unless the conformance caption sits on its
#                            own line directly under a bold badge heading
#
# Needs jq.

badge_logo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# NOTE: owner decision 2026-11-25; the badges stay short and this caption says once what they count.
badge_caption='<sub>(self-tested and self-assessed counts; none is a certification)</sub>'

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
    previous ~ /^\*\*[^*]+\*\*(<br>)?$/ && ($0 == caption || $0 == caption "<br>") { found = 1 }
    { previous = $0 }
    END { exit !found }
  ' "$1"; then
    echo "badge logo: $1 has no line $badge_caption directly under a bold badge heading" >&2
    return 1
  fi
}
