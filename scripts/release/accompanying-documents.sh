#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Write the information sheet and the instructions for use of a release as
# the two files the release lane attaches to its GitHub release.
#
# EHDS Art. 30(1)(d) (docs/law/eu/ehds/text.html) has each EHR system
# "accompanied, free of charge for the user, by the information sheet provided
# for in Article 38 and clear and complete instructions for use", and recital
# 37 asks for accessible formats. The book is deployed from main, so the copy
# a release carries is cut here from the tag's own pages: plain-text Markdown,
# which a screen reader reads as text, with the book's relative links rewritten
# to the frozen book of that release and the release's version and date
# stamped under the title.
#
# Usage: scripts/release/accompanying-documents.sh <tag> <out-dir>
#   writes <out-dir>/ferroehr-<tag>-information-sheet.md and
#          <out-dir>/ferroehr-<tag>-instructions-for-use.md
# It refuses a tag whose changelog section has no dated heading, and a page
# the structure check in scripts/checks/technical-documentation.sh refuses.
set -euo pipefail
# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
cd "$(dirname "$0")/../.."

[[ "$#" -eq 2 ]] || guard_usage "<tag> <out-dir>"
tag="$1"
out="$2"
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] \
  || { echo "accompanying-documents: $tag is not a vX.Y.Z tag" >&2; exit 1; }
version="${tag#v}"

bash scripts/checks/technical-documentation.sh >/dev/null

# The release date is the date on the changelog heading, the same date the
# support period is computed from.
date="$(grep -E "^## \[${version//./\\.}\] - [0-9]{4}-[0-9]{2}-[0-9]{2}$" CHANGELOG.md \
  | sed -E 's/.* - ([0-9-]+)$/\1/' | head -n 1)"
[[ -n "$date" ]] || { echo "accompanying-documents: CHANGELOG.md has no '## [$version] - YYYY-MM-DD' heading" >&2; exit 1; }

# A pre-release is not frozen into the book, so its links go to the book of
# main; a release's go to its own frozen copy.
if [[ "$tag" == *-* ]]; then
  base="https://ferroehr.eu/docs/dev"
else
  base="https://ferroehr.eu/docs/${tag}"
fi

mkdir -p "$out"
render() {
  local page="$1" name="$2"
  local target="$out/ferroehr-${tag}-${name}.md"
  # Relative links resolve from website/book/src/compliance/: "../x.md" is a
  # page of the book root, "y.md" a page beside this one. Only links that end
  # up under the frozen book get .md rewritten to .html.
  sed -E \
    -e "s#\]\(\.\./#](${base}/#g" \
    -e "s#\]\(([a-z0-9][a-z0-9-]*\.md)#](${base}/compliance/\1#g" \
    -e "s#\]\((${base//./\\.}/[^)#]*)\.md#](\1.html#g" \
    "$page" > "$target.tmp"
  # The stamp goes under the H1, so the file says which release it belongs to.
  awk -v stamp="**This copy:** FerroEHR ${version}, released ${date}, manufactured by Cadasto B.V. The online version is ${base}/compliance/${name}.html." \
    'NR == 1 && /^# / { print; print ""; print stamp; next } { print }' \
    "$target.tmp" > "$target"
  rm -f "$target.tmp"
  grep -qF "**This copy:**" "$target" \
    || { echo "accompanying-documents: $page does not start with a '# ' title" >&2; exit 1; }
  echo "$target"
}

render website/book/src/compliance/information-sheet.md information-sheet
render website/book/src/compliance/instructions-for-use.md instructions-for-use
