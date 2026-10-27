#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The notes of a release link that release's CRA user information, state its
# support period and name the revision of the EHR system's documents it
# relies on, and the page they link is in the book the tag freezes.
#
# CRA Art. 13(18) has each product "accompanied by the information and
# instructions to the user set out in Annex II", and Art. 13(19) and Annex II
# point 7 the end of the support period (docs/law/eu/cra/text.html). The
# information is website/book/src/compliance/cra-user-information.md, which the
# docs-freeze leg publishes for a release at /docs/<tag>/; a pre-release is not
# frozen, so its notes link the book of main. The release lane composes the
# notes with the URL this script prints and refuses the release on a finding.
# The information sheet, the instructions for use, the readiness record and
# the technical documentation are the EHR system's (FerroEHR with FerroBRIDGE)
# and live in the FerroHEALTH book; docs/ehds/system-documents.env pins the
# commit and the revision of each page a release relies on, and the notes name
# them in the section `--ehds` prints (EHDS Art. 30(1)(d), Art. 37).
#
# Usage:
#   scripts/checks/release-notes.sh --url <tag>          print the page URL for <tag>
#   scripts/checks/release-notes.sh --ehds               print the EHDS documents section
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
PIN="docs/ehds/system-documents.env"
GRAMMAR="--url <tag> | --ehds | <notes-file> <tag> | --self-test"

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

# The notes section naming the revision of the EHR system's documents, read
# from the pin. A pin that is incomplete prints nothing, which the notes check
# then refuses.
ehds_section() {
  local commit sheet ifu readiness techdoc
  commit="$(sed -n 's/^FERROHEALTH_COMMIT=//p' "$PIN" 2>/dev/null)"
  sheet="$(sed -n 's/^INFORMATION_SHEET_REVISED=//p' "$PIN" 2>/dev/null)"
  ifu="$(sed -n 's/^INSTRUCTIONS_FOR_USE_REVISED=//p' "$PIN" 2>/dev/null)"
  readiness="$(sed -n 's/^READINESS_REVISED=//p' "$PIN" 2>/dev/null)"
  techdoc="$(sed -n 's/^TECHNICAL_DOCUMENTATION_REVISED=//p' "$PIN" 2>/dev/null)"
  [[ -n "$commit" && -n "$sheet" && -n "$ifu" && -n "$readiness" && -n "$techdoc" ]] || return 0
  echo "### The EHDS system documents"
  echo
  echo "The EHR system is FerroEHR with FerroBRIDGE, and its documents are pages of the FerroHEALTH book. This release relies on them at commit \`${commit}\` of <https://github.com/FerroHEALTH/FerroHEALTH>: the [information sheet](https://ferrohealth.eu/docs/ehds/information-sheet.html) revised ${sheet} and the [instructions for use](https://ferrohealth.eu/docs/ehds/instructions-for-use.html) revised ${ifu}, attached to this release with its versions appended (EHDS Art. 30(1)(d), Art. 38), the [readiness record](https://ferrohealth.eu/docs/ehds/readiness.html) revised ${readiness} and the [technical documentation](https://ferrohealth.eu/docs/ehds/technical-documentation.html) revised ${techdoc} (EHDS Art. 37)."
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
  local section
  section="$(ehds_section)"
  if [[ -z "$section" ]]; then
    echo "$PIN does not name the commit and the four revision dates"
  elif [[ "$notes" != *"$section"* ]]; then
    echo "the notes do not name the pinned revision of the EHDS system documents ('scripts/checks/release-notes.sh --ehds' prints the section)"
  fi
}

self_test() {
  local good out
  good="$(printf '%s\n' "### Fixed" "- a fix" "" "### Support period" "" "until the end of October 2031" "" \
    "### Information and instructions to the user (CRA Annex II)" "" \
    "[For v4.3.5](https://ferroehr.eu/docs/v4.3.5/compliance/cra-user-information.html)" "")
$(ehds_section)"
  out="$(notes_findings v4.3.5 <<<"$good")"
  [[ -z "$out" ]] || { echo "self-test: complete notes were refused: $out" >&2; exit 1; }
  out="$(notes_findings v4.3.6 <<<"$good")"
  [[ "$out" == *"do not link"* ]] || { echo "self-test: another tag's link was accepted" >&2; exit 1; }
  out="$(notes_findings v4.3.5 <<<"$(grep -v 'cra-user-information' <<<"$good")")"
  [[ "$out" == *"do not link"* ]] || { echo "self-test: notes without the link were accepted" >&2; exit 1; }
  out="$(notes_findings v4.3.5 <<<"$(grep -vxF '### Support period' <<<"$good")")"
  [[ "$out" == *"Support period"* ]] || { echo "self-test: notes without the support period were accepted" >&2; exit 1; }
  out="$(notes_findings v4.3.5 <<<"${good//revised /revised 1999-01-01 or }")"
  [[ "$out" == *"EHDS system documents"* ]] || { echo "self-test: notes naming another revision were accepted" >&2; exit 1; }
  out="$(notes_findings v4.3.5 <<<"$(grep -v 'ferrohealth.eu' <<<"$good")")"
  [[ "$out" == *"EHDS system documents"* ]] || { echo "self-test: notes without the EHDS documents were accepted" >&2; exit 1; }
  out="$(notes_findings v4.3.5-rc1 <<<"$good")"
  [[ "$out" == *"docs/dev/"* ]] || { echo "self-test: a pre-release was not sent to the book of main" >&2; exit 1; }
  if page_url "4.3.5" >/dev/null 2>&1; then
    echo "self-test: a tag without its v was accepted" >&2
    exit 1
  fi
  echo "release-notes: self-test OK (complete notes, another tag, no link, no support period, another or no EHDS revision, pre-release, bad tag)."
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
  --ehds)
    shift
    guard_no_args "$@"
    ehds_section
    exit 0
    ;;
  *) ;;
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
echo "release-notes: $notes links $(page_url "$tag"), states the support period and names the pinned EHDS system documents"
