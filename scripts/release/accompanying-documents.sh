#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Write the information sheet and the instructions for use of a release as
# the two files the release lane attaches to its GitHub release.
#
# EHDS Art. 30(1)(d) (docs/law/eu/ehds/text.html) has each EHR system
# "accompanied, free of charge for the user, by the information sheet provided
# for in Article 38 and clear and complete instructions for use", and recital
# 37 asks for accessible formats. The EHR system is FerroEHR with FerroBRIDGE,
# and both documents live version-free in the FerroHEALTH book. This takes each
# page at the commit docs/ehds/system-documents.env pins, refuses a page whose
# revision is not the pinned one, and writes plain-text Markdown, which a
# screen reader reads as text: the family book's relative links made absolute,
# FerroEHR's links pointed at the frozen book of this release, the release's
# version and date stamped under the title, and the page's "This copy" section
# replaced by this release's facts (EHDS Art. 38(2)(b) and (e)), read from its
# pins by scripts/release/system-documents.sh.
#
# Usage: scripts/release/accompanying-documents.sh <tag> <out-dir>
#        scripts/release/accompanying-documents.sh --self-test
#   writes <out-dir>/ferroehr-<tag>-information-sheet.md and
#          <out-dir>/ferroehr-<tag>-instructions-for-use.md
# EHDS_PAGES_DIR names a directory to read the pages from in place of the
# network, for a test.
set -euo pipefail
# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
cd "$(dirname "$0")/../.."

readonly FAMILY="https://ferrohealth.eu/docs"
readonly DOCS="scripts/release/system-documents.sh"

die() { echo "accompanying-documents: $*" >&2; exit 1; }

# render PAGE TAG SOURCE TARGET: the attached copy of PAGE for TAG.
render() {
  local page="$1" tag="$2" source="$3" target="$4" version base date revised
  version="${tag#v}"
  # A pre-release is not frozen into the book, so its links go to the book of
  # main; a release's go to its own frozen copy.
  if [[ "$tag" == *-* ]]; then
    base="https://ferroehr.eu/docs/dev"
  else
    base="https://ferroehr.eu/docs/${tag}"
  fi
  date="$(bash "$DOCS" facts "$tag" "$page" | sed -n 's/.*, released \([0-9-]*\)\..*/\1/p' | head -n 1)"
  revised="$(sed -n '3s/^Revised \([0-9-]*\)\.$/\1/p' "$source")"
  # Everything above the page's "This copy" heading, without mdBook's toc
  # marker. Relative links resolve from website/book/src/ehds/ of the family
  # book: "../x.md" is a page of its root, "y.md" a page beside this one.
  awk '/^## This copy$/ { exit } /^<!-- toc -->$/ { next } { print }' "$source" \
    | sed -E \
      -e "s#\]\(\.\./([a-z0-9-]+)\.md#](${FAMILY}/\1.html#g" \
      -e "s#\]\(([a-z0-9][a-z0-9-]*)\.md#](${FAMILY}/ehds/\1.html#g" \
      -e "s#https://ferroehr\.eu/docs/latest/#${base}/#g" \
    | awk -v stamp="**This copy:** FerroEHR ${version}, released ${date}, manufactured by Cadasto B.V. It is the FerroHEALTH book's ${FAMILY}/ehds/${page}.html, revised ${revised}, with the facts of this release under \"This copy\" at the end." \
      'NR == 1 && /^# / { print; print ""; print stamp; next } { print }' >"$target"
  bash "$DOCS" facts "$tag" "$page" >>"$target"
  grep -qF "**This copy:**" "$target" || die "ehds/$page.md does not start with a '# ' title"
}

# check_sheet FILE: the information sheet carries every point of Art. 38(2).
check_sheet() {
  local file="$1" point
  for point in a b c d e; do
    grep -qE "^## \(${point}\) " "$file" || die "the information sheet has no section for Art. 38(2)(${point})"
  done
}

main() {
  local tag="$1" out="$2" page
  [[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || die "$tag is not a vX.Y.Z tag"
  bash scripts/checks/technical-documentation.sh >/dev/null
  mkdir -p "$out"
  # Global, so the exit trap still sees it once main has returned.
  WORK="$(mktemp -d)"
  trap 'rm -rf "$WORK"' EXIT
  for page in information-sheet instructions-for-use; do
    bash "$DOCS" fetch "$page" "$WORK/$page.md"
  done
  check_sheet "$WORK/information-sheet.md"
  for page in information-sheet instructions-for-use; do
    render "$page" "$tag" "$WORK/$page.md" "$out/ferroehr-${tag}-${page}.md"
    echo "$out/ferroehr-${tag}-${page}.md"
  done
}

self_test() {
  local tmp revised_sheet revised_ifu tag out
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN
  revised_sheet="$(bash "$DOCS" pin INFORMATION_SHEET_REVISED)"
  revised_ifu="$(bash "$DOCS" pin INSTRUCTIONS_FOR_USE_REVISED)"
  mkdir -p "$tmp/pages"
  {
    printf '# The EHDS EHR system: information sheet\n\nRevised %s.\n\n<!-- toc -->\n\n' "$revised_sheet"
    printf 'See the [instructions for use](instructions-for-use.md) and [security](../security.md#x).\n\n'
    printf 'Read [the audit](https://ferroehr.eu/docs/latest/audit.html#y).\n\n'
    for point in a b c d e; do printf '## (%s) Point\n\nText.\n\n' "$point"; done
    printf '## This copy\n\nThe book carries no facts here.\n'
  } >"$tmp/pages/information-sheet.md"
  printf '# The EHDS EHR system: instructions for use\n\nRevised %s.\n\nText.\n\n## This copy\n\nNone.\n' \
    "$revised_ifu" >"$tmp/pages/instructions-for-use.md"
  # The newest dated changelog heading stands in for the tag.
  tag="v$(grep -m 1 -oE '^## \[[0-9]+\.[0-9]+\.[0-9]+\] - [0-9-]+$' CHANGELOG.md | sed -E 's/^## \[([^]]+)\].*/\1/')"
  EHDS_PAGES_DIR="$tmp/pages" main "$tag" "$tmp/out" >/dev/null
  out="$tmp/out/ferroehr-${tag}-information-sheet.md"
  grep -qF "**This copy:** FerroEHR ${tag#v}, released" "$out" || die "self-test: no stamp under the title"
  grep -qF "](${FAMILY}/ehds/instructions-for-use.html)" "$out" || die "self-test: a sibling page link was not made absolute"
  grep -qF "](${FAMILY}/security.html#x)" "$out" || die "self-test: a family root link was not made absolute"
  grep -qF "](https://ferroehr.eu/docs/${tag}/audit.html#y)" "$out" || die "self-test: a FerroEHR link does not point at the frozen book"
  grep -qF "The book carries no facts here." "$out" && die "self-test: the page's own 'This copy' text survived"
  grep -qF "<!-- toc -->" "$out" && die "self-test: the toc marker survived"
  [[ "$(grep -c '^## This copy$' "$out")" -eq 1 ]] || die "self-test: the copy does not carry exactly one 'This copy' section"
  grep -qF "| openEHR ITS-REST |" "$out" || die "self-test: the standards table is missing"
  grep -qF "**Kubernetes:**" "$tmp/out/ferroehr-${tag}-instructions-for-use.md" || die "self-test: the platform facts are missing"
  sed -i.bak '/^## (c) /d' "$tmp/pages/information-sheet.md"
  if (EHDS_PAGES_DIR="$tmp/pages" main "$tag" "$tmp/out2") >/dev/null 2>&1; then
    die "self-test: a sheet without Art. 38(2)(c) was attached"
  fi
  echo "accompanying-documents: self-test OK (stamp, absolute family links, frozen FerroEHR links, facts replace 'This copy', a sheet missing a point refused)."
}

case "${1:-}" in
  --self-test)
    shift
    guard_no_args "$@"
    self_test
    ;;
  *)
    [[ "$#" -eq 2 ]] || guard_usage "<tag> <out-dir> | --self-test"
    main "$1" "$2"
    ;;
esac
