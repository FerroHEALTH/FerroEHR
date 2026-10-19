#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# The release's milestone (no specification governs this: our own design). A
# release is cut only when the milestone titled exactly as its tag holds no
# open issue, and the release workflow closes that milestone once every
# publishing leg has succeeded, so a published release never leaves its
# milestone open.
#
#   milestone.sh check <tag>
#       Exit 0 when the milestone <tag> exists and holds no open issue or pull
#       request. Exit 1 naming the missing milestone or every open item.
#   milestone.sh close <tag>
#       Checks again, then closes the milestone. An already-closed milestone
#       counts as done. When NEEDS_JSON holds the workflow's `needs` context,
#       every leg in it must have succeeded first; otherwise the milestone
#       stays open and each leg that did not succeed is named.
#   milestone.sh --self-test
#       Proves both modes against a stub gh.
#
# A pre-release tag (vX.Y.Z-rc.1) finishes no milestone, so both modes accept
# it and change nothing. The repository is GITHUB_REPOSITORY, or the one gh
# resolves from the checkout; gh authenticates with GH_TOKEN. Exit 2 on a
# usage error. Requires gh and jq.
set -euo pipefail

usage() {
  echo "usage: milestone.sh check <tag> | close <tag> | --self-test" >&2
  exit 2
}

die() {
  echo "::error::milestone: $*" >&2
  exit 1
}

repo() {
  if [[ -n "${GITHUB_REPOSITORY:-}" ]]; then
    printf '%s' "$GITHUB_REPOSITORY"
  else
    gh repo view --json nameWithOwner --jq .nameWithOwner
  fi
}

valid_tag() {
  if [[ ! "$1" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
    echo "::error::milestone: '$1' is not a vX.Y.Z tag" >&2
    exit 2
  fi
}

# find_milestone REPO TAG: prints "<number> <state>" of the milestone titled
# exactly TAG, or nothing when there is none.
find_milestone() {
  gh api --paginate "repos/$1/milestones?state=all&per_page=100" |
    jq -rs --arg t "$2" 'add // [] | map(select(.title == $t)) | first // empty | "\(.number) \(.state)"'
}

# open_items REPO NUMBER: one line per open issue or pull request in the
# milestone, both of which the milestone's open count includes.
open_items() {
  gh api --paginate "repos/$1/issues?milestone=$2&state=open&per_page=100" |
    jq -rs 'add // [] | .[] | "#\(.number) \(if .pull_request then "(pull request) " else "" end)\(.title)"'
}

# refuse_open TAG ITEMS: fails naming every open item, when there is one.
refuse_open() {
  local item
  [[ -n "$2" ]] || return 0
  while IFS= read -r item; do
    echo "::error::milestone: $1 still holds $item" >&2
  done <<<"$2"
  die "milestone $1 has $(wc -l <<<"$2" | tr -d ' ') open item(s). Close or move them to the next milestone before the release is cut"
}

check() {
  local tag="$1" repository found number state
  valid_tag "$tag"
  if [[ "$tag" == *-* ]]; then
    echo "milestone: $tag is a pre-release, which finishes no milestone."
    return 0
  fi
  repository="$(repo)"
  found="$(find_milestone "$repository" "$tag")"
  [[ -n "$found" ]] || die "no milestone of $repository is titled '$tag'. Create it before the release is cut"
  read -r number state <<<"$found"
  refuse_open "$tag" "$(open_items "$repository" "$number")"
  echo "milestone: $tag (#$number, $state) holds no open item."
}

close() {
  local tag="$1" repository failed leg found number state
  valid_tag "$tag"
  if [[ "$tag" == *-* ]]; then
    echo "milestone: $tag is a pre-release, which finishes no milestone."
    return 0
  fi
  if [[ -n "${NEEDS_JSON:-}" ]]; then
    failed="$(jq -r 'to_entries[] | select(.value.result != "success") | "\(.key) (\(.value.result))"' <<<"$NEEDS_JSON")"
    if [[ -n "$failed" ]]; then
      while IFS= read -r leg; do
        echo "::error::milestone: the release leg $leg did not succeed" >&2
      done <<<"$failed"
      die "the release of $tag is incomplete, so milestone $tag stays open. Re-run the failed jobs and this job closes it"
    fi
  fi
  repository="$(repo)"
  found="$(find_milestone "$repository" "$tag")"
  [[ -n "$found" ]] || die "no milestone of $repository is titled '$tag', so there is nothing to close"
  read -r number state <<<"$found"
  if [[ "$state" == closed ]]; then
    echo "milestone: $tag (#$number) is already closed."
    return 0
  fi
  refuse_open "$tag" "$(open_items "$repository" "$number")"
  gh api -X PATCH "repos/$repository/milestones/$number" -f state=closed >/dev/null
  echo "milestone: closed $tag (#$number)."
}

self_test() {
  local scratch fails=0 out
  scratch="$(mktemp -d)"
  trap 'rm -rf "$scratch"' RETURN
  export GITHUB_REPOSITORY=o/r
  STUB_MILESTONES='[{"number":4,"title":"v1.1.0","state":"open"}][{"number":3,"title":"v1.0.0","state":"open"}]'
  STUB_ISSUES='[]'
  # The stub answers the three calls the script makes and records each close.
  # shellcheck disable=SC2329 # the functions under test call it in place of gh
  gh() {
    case "$*" in
      "api --paginate repos/o/r/milestones?state=all&per_page=100") printf '%s' "$STUB_MILESTONES" ;;
      "api --paginate repos/o/r/issues?milestone=3&state=open&per_page=100") printf '%s' "$STUB_ISSUES" ;;
      "api -X PATCH repos/o/r/milestones/3 -f state=closed") echo closed >>"$scratch/patched" ;;
      *)
        echo "stub gh: unexpected call: $*" >&2
        return 1
        ;;
    esac
  }
  # expect NAME WANT_RC WANT_TEXT WANT_PATCHED -- COMMAND...
  expect() {
    local name="$1" want_rc="$2" want_text="$3" want_patched="$4" rc patched
    shift 5
    rm -f "$scratch/patched"
    if out="$("$@" 2>&1)"; then rc=0; else rc=$?; fi
    patched=no
    [[ -f "$scratch/patched" ]] && patched=yes
    if [[ "$rc" != "$want_rc" || "$out" != *"$want_text"* || "$patched" != "$want_patched" ]]; then
      echo "FAIL: $name (exit $rc, closed $patched): $out" >&2
      fails=$((fails + 1))
    else
      echo "ok: $name"
    fi
  }
  expect "check passes an empty milestone" 0 "holds no open item" no -- check v1.0.0
  expect "check passes a pre-release" 0 "pre-release" no -- check v1.0.0-rc.1
  expect "check refuses a malformed tag" 2 "is not a vX.Y.Z tag" no -- check 1.0.0
  expect "check refuses a missing milestone" 1 "no milestone of o/r is titled 'v9.9.9'" no -- check v9.9.9
  expect "close closes an empty milestone" 0 "closed v1.0.0 (#3)" yes -- close v1.0.0
  NEEDS_JSON='{"plan":{"result":"success"},"crates":{"result":"failure"}}' \
    expect "close keeps the milestone open after a failed leg" 1 "the release leg crates (failure) did not succeed" no -- close v1.0.0
  NEEDS_JSON='{"plan":{"result":"success"},"crates":{"result":"success"}}' \
    expect "close closes after every leg succeeded" 0 "closed v1.0.0" yes -- close v1.0.0
  expect "close refuses a missing milestone" 1 "nothing to close" no -- close v9.9.9
  NEEDS_JSON='{"crates":{"result":"skipped"}}' \
    expect "close leaves a pre-release alone" 0 "pre-release" no -- close v1.0.0-rc.1
  STUB_ISSUES='[{"number":7,"title":"A straggler"}][{"number":8,"title":"A fix","pull_request":{}}]'
  expect "check names each open issue" 1 "v1.0.0 still holds #7 A straggler" no -- check v1.0.0
  expect "check names each open pull request" 1 "#8 (pull request) A fix" no -- check v1.0.0
  expect "close refuses a milestone with open items" 1 "has 2 open item(s)" no -- close v1.0.0
  STUB_MILESTONES='[{"number":3,"title":"v1.0.0","state":"closed"}]'
  STUB_ISSUES='[]'
  expect "close counts a closed milestone as done" 0 "already closed" no -- close v1.0.0
  expect "check passes a closed, empty milestone" 0 "holds no open item" no -- check v1.0.0
  unset -f gh
  if [[ "$fails" -ne 0 ]]; then
    echo "milestone: self-test FAILED ($fails)" >&2
    return 1
  fi
  echo "milestone: self-test OK."
}

case "${1:-}" in
  check | close)
    [[ $# -eq 2 ]] || usage
    "$1" "$2"
    ;;
  --self-test)
    [[ $# -eq 1 ]] || usage
    self_test
    ;;
  *) usage ;;
esac
