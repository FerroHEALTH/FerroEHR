#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# scripts/gh/rel.sh — the deterministic GitHub issue-relationship helper.
#
# WHY THIS EXISTS: `gh` has NO native subcommand for sub-issues or issue
# dependencies (verified against gh 2.88.1), so relationships can only be set
# through `gh api`. Worse, every WRITE endpoint takes the target issue's
# DATABASE id, not its #number — a foot-gun that makes hand-typed `gh api`
# calls easy to get wrong. This wrapper resolves #number -> database id for
# you and calls the one correct endpoint, so every relationship command is
# consistent, typed correctly, and fails loud on a bad number.
#
# Reads are #number-keyed and need no id resolution; writes go through here.
#
# Official docs (durable references — the ONLY citations allowed for this):
#   Sub-issues API ...... https://docs.github.com/en/rest/issues/sub-issues
#   Dependencies API .... https://docs.github.com/en/rest/issues/issue-dependencies
#   Sub-issues (concept)  https://docs.github.com/en/issues/tracking-your-work-with-issues/using-issues/adding-sub-issues
#   Dependencies (concept) https://docs.github.com/en/issues/tracking-your-work-with-issues/using-issues/creating-issue-dependencies
#
# Policy — WHEN to use each relationship: .claude/rules/issue-relationships.md
#
# Limits (from the docs above): <=100 sub-issues per parent, <=8 nesting
# levels, one parent per issue (use --replace to move it); <=50 issues per
# dependency direction.
#
# Usage:
#   scripts/gh/rel.sh parent     <child> <parent> [--replace]  # child -> sub-issue of parent
#   scripts/gh/rel.sh unparent   <child>                       # detach child from its parent
#   scripts/gh/rel.sh blocked-by <n> <blocker>                # n is blocked by blocker
#   scripts/gh/rel.sh unblock    <n> <blocker>                # remove "n blocked-by blocker"
#   scripts/gh/rel.sh blocking   <n> <blocked>                # n blocks blocked
#   scripts/gh/rel.sh unblocking <n> <blocked>                # remove "n blocking blocked"
#   scripts/gh/rel.sh tree       <n>                          # print every relationship of n
#   scripts/gh/rel.sh id         <n>                          # print the database id of n
#
# All commands act on the current repository (`gh repo view`).

set -euo pipefail

die() {
  echo "gh-rel: $*" >&2
  exit 1
}

need_int() {
  case "${1:-}" in
    '' | *[!0-9]*) die "expected an issue number, got '${1:-}'" ;;
    *) ;;
  esac
}

command -v gh >/dev/null 2>&1 || die "the GitHub CLI (gh) is not installed"

REPO="$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null)" ||
  die "could not resolve the current repository (run inside a gh-authenticated clone)"

# Resolve an issue #number to its database id (the value every write endpoint
# wants). Fails loud if the issue does not exist.
dbid() {
  need_int "$1"
  local id
  id="$(gh api "repos/$REPO/issues/$1" --jq '.id' 2>/dev/null)" ||
    die "issue #$1 not found in $REPO"
  case "$id" in
    '' | *[!0-9]*) die "could not resolve the database id for #$1" ;;
    *) ;;
  esac
  printf '%s' "$id"
}

# Print a jq-formatted list from an endpoint, or "    —" when empty/absent.
list_or_dash() {
  local out
  out="$(gh api "$1" --jq "$2" 2>/dev/null || true)"
  if [[ -n "$out" ]]; then echo "$out"; else echo "    —"; fi
}

cmd_parent() {
  local child="${1:?child issue number}" parent="${2:?parent issue number}" flag="${3:-}"
  need_int "$child"
  need_int "$parent"
  [[ "$child" != "$parent" ]] || die "an issue cannot be its own parent"
  local cid body
  cid="$(dbid "$child")"
  if [[ "$flag" = "--replace" ]]; then
    body="$(printf '{"sub_issue_id":%d,"replace_parent":true}' "$cid")"
  elif [[ -n "$flag" ]]; then
    die "unknown flag '$flag' (only --replace is supported)"
  else
    body="$(printf '{"sub_issue_id":%d}' "$cid")"
  fi
  printf '%s' "$body" | gh api --method POST "repos/$REPO/issues/$parent/sub_issues" --input - >/dev/null
  echo "ok: #$child is now a sub-issue of #$parent"
}

cmd_unparent() {
  local child="${1:?child issue number}"
  need_int "$child"
  local parent cid
  parent="$(gh api "repos/$REPO/issues/$child/parent" --jq '.number' 2>/dev/null || true)"
  [[ -n "$parent" ]] || die "#$child has no parent"
  cid="$(dbid "$child")"
  printf '{"sub_issue_id":%d}' "$cid" | gh api --method DELETE "repos/$REPO/issues/$parent/sub_issue" --input - >/dev/null
  echo "ok: detached #$child from parent #$parent"
}

cmd_blocked_by() {
  local n="${1:?issue number}" blocker="${2:?blocker issue number}"
  need_int "$n"
  need_int "$blocker"
  [[ "$n" != "$blocker" ]] || die "an issue cannot block itself"
  local bid
  bid="$(dbid "$blocker")"
  printf '{"issue_id":%d}' "$bid" | gh api --method POST "repos/$REPO/issues/$n/dependencies/blocked_by" --input - >/dev/null
  echo "ok: #$n is now blocked by #$blocker"
}

cmd_unblock() {
  local n="${1:?issue number}" blocker="${2:?blocker issue number}"
  need_int "$n"
  need_int "$blocker"
  local bid
  bid="$(dbid "$blocker")"
  gh api --method DELETE "repos/$REPO/issues/$n/dependencies/blocked_by/$bid" >/dev/null
  echo "ok: #$n is no longer blocked by #$blocker"
}

# "n blocks blocked" is stored as "blocked is blocked-by n" (the only writable
# direction the API exposes), so we POST to the OTHER issue's blocked_by list.
cmd_blocking() {
  local n="${1:?issue number}" blocked="${2:?blocked issue number}"
  need_int "$n"
  need_int "$blocked"
  [[ "$n" != "$blocked" ]] || die "an issue cannot block itself"
  local nid
  nid="$(dbid "$n")"
  printf '{"issue_id":%d}' "$nid" | gh api --method POST "repos/$REPO/issues/$blocked/dependencies/blocked_by" --input - >/dev/null
  echo "ok: #$n now blocks #$blocked"
}

cmd_unblocking() {
  local n="${1:?issue number}" blocked="${2:?blocked issue number}"
  need_int "$n"
  need_int "$blocked"
  local nid
  nid="$(dbid "$n")"
  gh api --method DELETE "repos/$REPO/issues/$blocked/dependencies/blocked_by/$nid" >/dev/null
  echo "ok: #$n no longer blocks #$blocked"
}

cmd_tree() {
  local n="${1:?issue number}"
  need_int "$n"
  local title parent
  title="$(gh api "repos/$REPO/issues/$n" --jq '"[\(.state)] \(.title)"' 2>/dev/null || true)"
  echo "#$n ${title:-} — $REPO"
  # The parent endpoint 404s when there is no parent, and gh emits the error
  # body on stdout; capture only on a clean (2xx) call so it does not leak.
  parent="$(gh api "repos/$REPO/issues/$n/parent" --jq '"#\(.number) [\(.state)] \(.title)"' 2>/dev/null)" || parent=""
  echo "  parent:"
  echo "    ${parent:-—}"
  echo "  sub-issues:"
  list_or_dash "repos/$REPO/issues/$n/sub_issues" '.[] | "    #\(.number) [\(.state)] \(.title)"'
  echo "  blocked by:"
  list_or_dash "repos/$REPO/issues/$n/dependencies/blocked_by" '.[] | "    #\(.number) [\(.state)] \(.title)"'
  echo "  blocking:"
  list_or_dash "repos/$REPO/issues/$n/dependencies/blocking" '.[] | "    #\(.number) [\(.state)] \(.title)"'
}

cmd_id() {
  dbid "${1:?issue number}"
  echo
}

usage() {
  # The banner is the header comment block itself, printed structurally
  # (skip the shebang + SPDX lines, stop at the first non-comment line) so
  # editing the header can never misalign a hardcoded line window.
  awk 'NR <= 3 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0"
}

main() {
  local sub="${1:-}"
  [[ -n "$sub" ]] || {
    usage
    exit 1
  }
  shift
  case "$sub" in
    parent) cmd_parent "$@" ;;
    unparent) cmd_unparent "$@" ;;
    blocked-by) cmd_blocked_by "$@" ;;
    unblock) cmd_unblock "$@" ;;
    blocking) cmd_blocking "$@" ;;
    unblocking) cmd_unblocking "$@" ;;
    tree) cmd_tree "$@" ;;
    id) cmd_id "$@" ;;
    -h | --help | help) usage ;;
    *) die "unknown command '$sub' (run with no args for usage)" ;;
  esac
}

main "$@"
