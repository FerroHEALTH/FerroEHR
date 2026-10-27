#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The EHDS system documents a FerroEHR release relies on: the pin, the pages
# at the pinned commit of the FerroHEALTH book, and the per-release facts the
# release appends to the copy it attaches.
#
# The EHR system is FerroEHR with FerroBRIDGE, and its information sheet and
# instructions for use live version-free in the FerroHEALTH book. By the
# owner's ruling on #3705, each product's release lane appends that release's
# own facts under the page's last heading, "This copy": the version of the EHR
# system, which is "the release of the product this copy accompanies, with the
# oldest release of the other product it is tested against", the release date
# (EHDS Art. 38(2)(b)), and the versions of the standards (Art. 38(2)(e)),
# read from this release's pins. For FerroEHR the other product's release is
# the FerroBRIDGE release the Helm chart pins, the one it runs beside the CDR.
#
# Usage:
#   scripts/release/system-documents.sh pin <KEY>             print one pin value
#   scripts/release/system-documents.sh fetch <page> <file>   the page at the pinned commit
#   scripts/release/system-documents.sh facts <tag> <page>    the "This copy" section
#   scripts/release/system-documents.sh --self-test           prove the readers
# <page> is information-sheet or instructions-for-use. EHDS_PAGES_DIR names a
# directory to read the pages from in place of the network, for a test.
set -euo pipefail
# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
cd "$(dirname "$0")/../.."

readonly PIN="docs/ehds/system-documents.env"
readonly STANDARDS="docs/ehds/standards.tsv"
readonly VERSIONS="docs/VERSIONS.md"
readonly CHART="deploy/helm/ferroehr/Chart.yaml"
readonly HELPERS="deploy/helm/ferroehr/templates/_helpers.tpl"
readonly GRAMMAR="pin <KEY> | fetch <page> <file> | facts <tag> <page> | --self-test"

die() { echo "system-documents: $*" >&2; exit 1; }

# pin KEY: the value of KEY in the pin file.
pin() {
  local key="$1" value
  value="$(sed -n "s/^$key=//p" "$PIN" | head -n 1)"
  [[ -n "$value" ]] || die "$PIN has no $key"
  printf '%s\n' "$value"
}

# The pin key of a page's revision date.
revised_key() {
  local page="$1"
  case "$page" in
    information-sheet) echo INFORMATION_SHEET_REVISED ;;
    instructions-for-use) echo INSTRUCTIONS_FOR_USE_REVISED ;;
    *) die "unknown page '$page' (information-sheet or instructions-for-use)" ;;
  esac
}

# fetch PAGE FILE: the page at the pinned commit, refused when its Revised
# line is not the pinned date.
fetch() {
  local page="$1" file="$2" commit want got
  commit="$(pin FERROHEALTH_COMMIT)"
  want="$(pin "$(revised_key "$page")")"
  [[ "$commit" =~ ^[0-9a-f]{40}$ ]] || die "FERROHEALTH_COMMIT in $PIN is not a full commit id"
  if [[ -n "${EHDS_PAGES_DIR:-}" ]]; then
    cp "$EHDS_PAGES_DIR/$page.md" "$file"
  else
    curl --proto =https --fail --silent --show-error --location \
      "https://raw.githubusercontent.com/FerroHEALTH/FerroHEALTH/$commit/website/book/src/ehds/$page.md" \
      -o "$file" || die "ehds/$page.md could not be fetched at FerroHEALTH $commit"
  fi
  got="$(sed -n '3s/^Revised \([0-9-]*\)\.$/\1/p' "$file")"
  [[ "$got" == "$want" ]] \
    || die "ehds/$page.md at FerroHEALTH $commit is revised ${got:-<no Revised line>}, and $PIN pins $want"
  grep -qx '## This copy' "$file" || die "ehds/$page.md at FerroHEALTH $commit has no '## This copy' heading"
}

# spec_versions ROW: the versions of the "Our pin" cell of the VERSIONS.md row
# that starts with ROW, read from the cell's bold text, else from its
# `Release-X.Y.Z` tags, else its first version, joined with "and".
spec_versions() {
  local name="$1" row cell found
  row="$(grep -m 1 -F "| $name |" "$VERSIONS" || true)"
  [[ -n "$row" ]] || die "$VERSIONS has no row '$name'"
  cell="$(awk -F'|' '{ print $3 }' <<<"$row")"
  found="$(grep -oE '\*\*[^*]+\*\*' <<<"$cell" | grep -oE '[0-9]+(\.[0-9]+)+' || true)"
  [[ -n "$found" ]] || found="$(grep -oE 'Release-[0-9]+\.[0-9]+\.[0-9]+' <<<"$cell" | sed 's/^Release-//' || true)"
  [[ -n "$found" ]] || found="$(grep -oE '[0-9]+(\.[0-9]+)+' <<<"$cell" | head -n 1 || true)"
  [[ -n "$found" ]] || die "the '$name' row of $VERSIONS names no version"
  awk '!seen[$0]++' <<<"$found" | paste -sd '|' - | sed 's/|/ and /g'
}

# The PostgreSQL pin of VERSIONS.md: its cell up to the first parenthesis.
database_version() {
  local cell
  cell="$(grep -m 1 -E '^\| PostgreSQL \|' "$VERSIONS" | awk -F'|' '{ print $3 }')"
  cell="${cell%%(*}"
  cell="$(sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//' <<<"$cell")"
  [[ -n "$cell" ]] || die "$VERSIONS has no PostgreSQL pin"
  printf '%s\n' "$cell"
}

# The oldest Kubernetes release the chart accepts, from its kubeVersion.
kubernetes_version() {
  local v
  v="$(sed -n 's/^kubeVersion: *"*>=\([0-9.]*\).*/\1/p' "$CHART" | head -n 1)"
  [[ -n "$v" ]] || die "$CHART names no kubeVersion floor"
  printf '%s or newer\n' "$v"
}

# The FerroBRIDGE release the chart runs beside the CDR.
bridge_version() {
  local v
  v="$(awk '/define "ferroehr.bridgePinnedVersion"/ { getline; print; exit }' "$HELPERS" | tr -d '[:space:]')"
  [[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "$HELPERS pins no FerroBRIDGE release"
  printf '%s\n' "$v"
}

# The release date of a version, from its changelog heading.
release_date() {
  local version="$1" date
  date="$(grep -E "^## \[${version//./\\.}\] - [0-9]{4}-[0-9]{2}-[0-9]{2}$" CHANGELOG.md \
    | sed -E 's/.* - ([0-9-]+)$/\1/' | head -n 1)"
  [[ -n "$date" ]] || die "CHANGELOG.md has no '## [$version] - YYYY-MM-DD' heading"
  printf '%s\n' "$date"
}

# facts TAG PAGE: the "This copy" section of PAGE for TAG.
facts() {
  local tag="$1" page="$2" version date bridge commit revised standard source value
  [[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || die "$tag is not a vX.Y.Z tag"
  version="${tag#v}"
  date="$(release_date "$version")"
  bridge="$(bridge_version)"
  commit="$(pin FERROHEALTH_COMMIT)"
  revised="$(pin "$(revised_key "$page")")"
  echo "## This copy"
  echo
  echo "This copy accompanies FerroEHR ${version}, released ${date}. It is the FerroHEALTH book's page revised ${revised}, taken at commit \`${commit}\` of <https://github.com/FerroHEALTH/FerroHEALTH>, with these facts of the release appended."
  echo
  echo "- **The EHR system and its version (EHDS Art. 38(2)(b)):** FerroEHR ${version} with FerroBRIDGE ${bridge}, the oldest FerroBRIDGE release this FerroEHR release is tested against: the release its Helm chart runs beside the CDR."
  echo "- **Release date:** ${date}, the date of the release's heading in FerroEHR's changelog."
  case "$page" in
    information-sheet)
      echo
      echo "The versions of the standards, formats and specifications FerroEHR ${version} supports (EHDS Art. 38(2)(e)), read from the release's pins. FerroBRIDGE's are on the copy each FerroBRIDGE release attaches."
      echo
      echo "| Standard or specification | Version |"
      echo "|---|---|"
      while IFS=$'\t' read -r standard source; do
        [[ -z "$standard" || "$standard" == \#* ]] && continue
        case "$source" in
          versions:*) value="$(spec_versions "${source#versions:}")" ;;
          database) value="$(database_version)" ;;
          text:*) value="${source#text:}" ;;
          *) die "$STANDARDS: unknown source '$source' for $standard" ;;
        esac
        echo "| $standard | $value |"
      done <"$STANDARDS"
      ;;
    instructions-for-use)
      echo "- **PostgreSQL:** $(database_version), the database FerroEHR needs."
      echo "- **Kubernetes:** $(kubernetes_version), the floor of the Helm chart's \`kubeVersion\`."
      ;;
    *) revised_key "$page" >/dev/null ;;
  esac
}

self_test() {
  local tmp out
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN
  # The readers, over this tree's own pins.
  [[ "$(spec_versions 'RM (Reference Model)')" =~ ^[0-9.]+( and [0-9.]+)*$ ]] || die "self-test: the RM row reads no version"
  [[ "$(spec_versions 'ITS-XML (XSDs)')" == *" and "* ]] || die "self-test: the two ITS-XML lineages were not both read"
  [[ "$(spec_versions 'QUERY (AQL)')" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "self-test: the AQL row reads no version"
  [[ "$(database_version)" == [0-9]* ]] || die "self-test: no PostgreSQL pin"
  [[ "$(kubernetes_version)" == [0-9]*" or newer" ]] || die "self-test: no Kubernetes floor"
  bridge_version >/dev/null
  while IFS=$'\t' read -r standard source; do
    [[ -z "$standard" || "$standard" == \#* ]] && continue
    case "$source" in
      versions:*) spec_versions "${source#versions:}" >/dev/null ;;
      database | text:*) ;;
      *) die "self-test: $STANDARDS names an unknown source '$source'" ;;
    esac
  done <"$STANDARDS"
  # The fetch refuses a page whose revision is not the pinned one, and one
  # without the heading the facts replace.
  printf '# A\n\nRevised %s.\n\n## This copy\n' "$(pin INFORMATION_SHEET_REVISED)" >"$tmp/information-sheet.md"
  printf '# B\n\nRevised 1999-01-01.\n\n## This copy\n' >"$tmp/instructions-for-use.md"
  if [[ "$(pin FERROHEALTH_COMMIT)" =~ ^[0-9a-f]{40}$ ]]; then
    EHDS_PAGES_DIR="$tmp" fetch information-sheet "$tmp/out.md" \
      || die "self-test: a page at the pinned revision was refused"
    if (EHDS_PAGES_DIR="$tmp" fetch instructions-for-use "$tmp/out2.md") 2>/dev/null; then
      die "self-test: a page at another revision was accepted"
    fi
    printf '# A\n\nRevised %s.\n' "$(pin INFORMATION_SHEET_REVISED)" >"$tmp/information-sheet.md"
    if (EHDS_PAGES_DIR="$tmp" fetch information-sheet "$tmp/out3.md") 2>/dev/null; then
      die "self-test: a page without '## This copy' was accepted"
    fi
  else
    die "self-test: FERROHEALTH_COMMIT in $PIN is not a full commit id"
  fi
  out="$(spec_versions 'AM (Archetype Model)')"
  [[ "$out" == *" and "* ]] || die "self-test: the two AM generations were not both read ($out)"
  echo "system-documents: self-test OK (the pins read, a pinned page fetched, another revision and a page without 'This copy' refused)."
}

case "${1:-}" in
  pin)
    [[ "$#" -eq 2 ]] || guard_usage "$GRAMMAR"
    pin "$2"
    ;;
  fetch)
    [[ "$#" -eq 3 ]] || guard_usage "$GRAMMAR"
    fetch "$2" "$3"
    ;;
  facts)
    [[ "$#" -eq 3 ]] || guard_usage "$GRAMMAR"
    facts "$2" "$3"
    ;;
  --self-test)
    shift
    guard_no_args "$@"
    self_test
    ;;
  *) guard_usage "$GRAMMAR" ;;
esac
