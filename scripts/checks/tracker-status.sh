#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# scripts/checks/tracker-status.sh — a book page never says "planned" about a
# closed issue, and never "shipped" about an open one (#3289).
#
# The control matrix is generated and cannot go stale; the prose around it can,
# and after the v4.2.0 cut the compliance overview still said "guidance planned,
# #3161" about pages that release had published. This guard reads every
# `#NNNN` issue reference on the book's pages (and every `issues: [...]` entry
# of the EHDS status source), fetches the issues' states in one call, and fails
# when a status word beside the reference contradicts the tracker.
#
# What counts as a contradiction, per LINE (a table row or a sentence):
#   - a PLANNED word (planned, not yet, upcoming, in progress, partly shipped,
#     partial, remains open, is tracked, not built) beside a reference to a CLOSED issue,
#     unless the line also says it shipped/landed/closed (a history sentence);
#   - a SHIPPED word (shipped, landed, closed in) beside a reference to an OPEN
#     issue, unless the line also carries a planned word (a split row).
#   - in website/book/ehds.yaml: `status: planned` whose `issues:` are all closed.
# Generated pages (control-matrix, ehds-readiness, technical-documentation) are
# render output of their own generators and are skipped.
#
# Usage: scripts/checks/tracker-status.sh [--closing <pr-number>]
#
# An issue the pull request under review CLOSES counts as closed: a control that
# lands in that PR is written "shipped, #N" on the page in the same change, and
# the merge closes the issue. CI hands the PR body in through PR_BODY (from the
# event payload, never interpolated into the shell); a local run names the PR
# with --closing, which reads the body through gh. Only the closing keywords
# GitHub itself honours count (Closes/Fixes/Resolves, docs.github.com
# "Linking a pull request to an issue").
set -euo pipefail
cd "$(dirname "$0")/../.."
# shellcheck source=scripts/lib/guard-args.sh
source scripts/lib/guard-args.sh
if [[ "${1:-}" == "--closing" ]]; then
  [[ -n "${2:-}" ]] || { echo "usage: $0 [--closing <pr-number>]" >&2; exit 2; }
  PR_BODY="$(gh pr view "$2" --json body --jq .body)"
  shift 2
fi
guard_no_args "$@"
command -v jq >/dev/null || { echo "error: jq is required" >&2; exit 1; }
command -v gh >/dev/null || { echo "error: gh is required" >&2; exit 1; }

BOOK=website/book/src
EHDS=website/book/ehds.yaml
PLANNED='planned|not yet|upcoming|in progress|partly shipped|partial|remains open|is tracked|tracked (on|in)|to be done|pending|not planned|not (yet )?built|still to come|(until|once|when) .* lands|yet to land'
SHIPPED='shipped|landed|closed in|closed by|delivered'

# Every issue number referenced on a page (excluding generated pages) or in
# the EHDS source, resolved once.
pages=()
while IFS= read -r page; do pages+=("$page"); done < <(git ls-files "$BOOK" | grep -E '\.md$' \
  | grep -v -E '/(control-matrix|ehds-readiness|technical-documentation)\.md$')
numbers="$( { grep -ohE '#[0-9]{3,5}\b' "${pages[@]}" 2>/dev/null | tr -d '#'; \
              grep -oE 'issues: \[[0-9, ]+\]' "$EHDS" | grep -oE '[0-9]+'; } | sort -un)"
[[ -n "$numbers" ]] || { echo "tracker-status: no issue references found"; exit 0; }

repo="$(gh repo view --json nameWithOwner --jq .nameWithOwner)"
# One GraphQL call for the whole set (aliases), never one call per issue.
query="query(\$owner:String!,\$name:String!){repository(owner:\$owner,name:\$name){"
for n in $numbers; do query+="i$n: issue(number:$n){number state} "; done
query+="}}"
states="$(gh api graphql -f owner="${repo%%/*}" -f name="${repo##*/}" -f query="$query" \
  --jq '.data.repository | to_entries | map(select(.value != null)) | map({(.value.number|tostring): .value.state}) | add // {}')"
# The issues the reviewed PR closes read as CLOSED, so a same-PR "shipped, #N"
# is not refused while the merge that closes #N is the very thing under review.
closing="$(grep -oiE '\b(close[sd]?|fix(e[sd])?|resolve[sd]?)[[:space:]]+#[0-9]+' <<<"${PR_BODY:-}" | grep -oE '[0-9]+$' | sort -u || true)"
for n in $closing; do
  states="$(jq --arg n "$n" '.[$n] = "CLOSED"' <<<"$states")"
done
state_of() { jq -r --arg n "$1" '.[$n] // "UNKNOWN"' <<<"$states"; }

fail=0
report() { echo "tracker-status: $1" >&2; fail=1; }

for page in "${pages[@]}"; do
  while IFS= read -r line; do
    lineno="${line%%:*}"; text="${line#*:}"
    lower="$(tr '[:upper:]' '[:lower:]' <<<"$text")"
    has_planned=0; has_shipped=0
    grep -qiE "$PLANNED" <<<"$lower" && has_planned=1
    # "partly shipped" is a planned status, not a shipped one.
    grep -qiE "$SHIPPED" <<<"${lower//partly shipped/}" && has_shipped=1
    for n in $(grep -oE '#[0-9]{3,5}\b' <<<"$text" | tr -d '#' | sort -u); do
      st="$(state_of "$n")"
      if [[ "$st" == CLOSED && $has_planned -eq 1 && $has_shipped -eq 0 ]]; then
        report "$page:$lineno says a planned/open status beside #$n, which is CLOSED — rewrite it to what shipped"
      elif [[ "$st" == OPEN && $has_shipped -eq 1 && $has_planned -eq 0 ]]; then
        report "$page:$lineno says shipped beside #$n, which is still OPEN"
      fi
    done
  done < <(grep -nE '#[0-9]{3,5}\b' "$page" || true)
done

# The EHDS status source: a `status: planned` entry whose `issues:` line
# names only closed issues. awk pairs each planned line with the issues line
# right after it; the shell then asks the state map for every number.
while IFS=: read -r lineno issues; do
  all_closed=1; any=0
  while read -r n; do
    any=1
    [[ "$(state_of "$n")" == CLOSED ]] || all_closed=0
  done < <(grep -oE '[0-9]+' <<<"$issues")
  if [[ $any -eq 1 && $all_closed -eq 1 ]]; then
    report "$EHDS:$lineno is \`planned\` but every issue it names ($(grep -oE '[0-9]+' <<<"$issues" | sed 's/^/#/' | paste -sd, - | sed 's/,/, /g')) is CLOSED — give it its real status"
  fi
done < <(awk '/status: planned/ { planned = NR; next }
              planned && NR == planned + 1 && /issues: \[/ { sub(/^[^[]*\[/, ""); sub(/\].*$/, ""); print planned ":" $0 }
              { planned = 0 }' "$EHDS")

if [[ $fail -ne 0 ]]; then
  echo "tracker-status: the book contradicts the tracker (above). A closed issue's control reads as shipped; an open one as planned." >&2
  exit 1
fi
echo "tracker-status: OK (${#pages[@]} pages, $(wc -w <<<"$numbers" | tr -d ' ') issue references agree with the tracker)."
