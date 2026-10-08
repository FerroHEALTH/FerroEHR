#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The notes of a release link that release's CRA user information and state
# its support period, and the page they link is in the book the tag freezes.
#
# CRA Art. 13(18) has each product "accompanied by the information and
# instructions to the user set out in Annex II", and Art. 13(19) and Annex II
# point 7 the end of the support period (docs/law/eu/cra/text.html). The
# information is website/book/src/compliance/cra-user-information.md, which the
# docs-freeze leg publishes for a release at /docs/<tag>/; a pre-release is not
# frozen, so its notes link the book of main. The release lane composes the
# notes with the URL this script prints and refuses the release on a finding.
#
# Usage:
#   scripts/checks/release-notes.sh --url <tag>          print the page URL for <tag>
#   scripts/checks/release-notes.sh <notes-file> <tag>   check composed notes
#   scripts/checks/release-notes.sh --self-test          prove the detectors
set -euo pipefail
# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
# A relative notes path is the caller's, so it is resolved before the cd.
caller_dir="$PWD"
cd "$(dirname "$0")/../.."

PAGE="website/book/src/compliance/cra-user-information.md"
SUMMARY="website/book/src/SUMMARY.md"
GRAMMAR="--url <tag> | <notes-file> <tag> | --self-test"

# The frozen-book URL of the page for <tag>, the scheme of
# scripts/site/cut-version.sh (/docs/vX.Y.Z/) and of the book's own paths.
page_url() {
  local tag="$1"
  [[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] \
    || { echo "release-notes: '$tag' is not a vX.Y.Z tag" >&2; return 2; }
  if [[ "$tag" == *-* ]]; then
    echo "https://ferroehr.eu/docs/dev/compliance/cra-user-information.html"
  else
    echo "https://ferroehr.eu/docs/${tag}/compliance/cra-user-information.html"
  fi
}

# Prints one line per finding in the notes text on stdin, for <tag>.
notes_findings() {
  local tag="$1" url notes
  url="$(page_url "$tag")"
  notes="$(cat)"
  grep -qF "](${url})" <<<"$notes" \
    || echo "the notes do not link the CRA user information of $tag ($url; CRA Art. 13(18), Annex II)"
  grep -qxF "### Support period" <<<"$notes" \
    || echo "the notes have no '### Support period' section (CRA Art. 13(19), Annex II point 7)"
}

self_test() {
  local good out
  good="$(printf '%s\n' "### Fixed" "- a fix" "" "### Support period" "" "until the end of October 2031" "" \
    "### Information and instructions to the user (CRA Annex II)" "" \
    "[For v4.3.5](https://ferroehr.eu/docs/v4.3.5/compliance/cra-user-information.html)")"
  out="$(notes_findings v4.3.5 <<<"$good")"
  [[ -z "$out" ]] || { echo "self-test: complete notes were refused: $out" >&2; exit 1; }
  out="$(notes_findings v4.3.6 <<<"$good")"
  [[ "$out" == *"do not link"* ]] || { echo "self-test: another tag's link was accepted" >&2; exit 1; }
  out="$(notes_findings v4.3.5 <<<"$(grep -v 'cra-user-information' <<<"$good")")"
  [[ "$out" == *"do not link"* ]] || { echo "self-test: notes without the link were accepted" >&2; exit 1; }
  out="$(notes_findings v4.3.5 <<<"$(grep -vxF '### Support period' <<<"$good")")"
  [[ "$out" == *"Support period"* ]] || { echo "self-test: notes without the support period were accepted" >&2; exit 1; }
  out="$(notes_findings v4.3.5-rc1 <<<"$good")"
  [[ "$out" == *"docs/dev/"* ]] || { echo "self-test: a pre-release was not sent to the book of main" >&2; exit 1; }
  if page_url "4.3.5" >/dev/null 2>&1; then
    echo "self-test: a tag without its v was accepted" >&2
    exit 1
  fi
  echo "release-notes: self-test OK (complete notes, another tag, no link, no support period, pre-release, bad tag)."
}

case "${1:-}" in
  --self-test)
    shift
    guard_no_args "$@"
    self_test
    exit 0
    ;;
  --url)
    [[ "$#" -eq 2 ]] || guard_usage "$GRAMMAR"
    page_url "$2"
    exit 0
    ;;
esac
[[ "$#" -eq 2 ]] || guard_usage "$GRAMMAR"
guard_known_flags "$GRAMMAR" "" "$@"
notes="$1"
[[ "$notes" == /* ]] || notes="$caller_dir/$notes"
tag="$2"
[[ -f "$notes" ]] || { echo "release-notes: $notes does not exist" >&2; exit 2; }
page_url "$tag" >/dev/null

fail=0
report() { echo "release-notes: $1" >&2; fail=1; }

[[ -f "$PAGE" ]] || report "$PAGE does not exist, so the link would not resolve"
grep -qF "(compliance/cra-user-information.md)" "$SUMMARY" \
  || report "$SUMMARY does not list compliance/cra-user-information.md, so the book would not render it"
while IFS= read -r finding; do
  if [[ -n "$finding" ]]; then report "$finding"; fi
done < <(notes_findings "$tag" <"$notes")

[[ "$fail" -eq 0 ]] || exit 1
echo "release-notes: $notes links $(page_url "$tag") and states the support period"
