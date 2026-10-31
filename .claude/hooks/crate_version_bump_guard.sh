#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# .claude/hooks/crate_version_bump_guard.sh
#
# Claude Code PreToolUse hook (matcher: Bash). Blocks `git push` when the
# outgoing commits edit the lockstep 0.0.x version of the published crates/*
# members or an internal requirement: the publish step sets it, never a pull
# request (.claude/rules/crates-publishing.md, #4555). The judgement is
# scripts/checks/crate-version-guard.sh, the guard the crate-version-guard CI
# job runs, called on origin/main and HEAD, so the hook and CI cannot drift
# (#4217); this hook fails the push before CI would.
#
# Escape hatch (mirrors the CI `no-crate-bump` label, for a change that must
# set the version by hand): FERROEHR_SKIP_CRATE_BUMP_GUARD=1 git push …
#
# Reads the tool-call JSON on stdin. Exit 2 blocks; exit 0 allows.

set -euo pipefail

payload="$(cat)"

if command -v jq >/dev/null 2>&1; then
  cmd="$(jq -r '.tool_input.command // empty' <<<"$payload" 2>/dev/null || true)"
else
  cmd="$payload"
fi
[ -n "${cmd:-}" ] || exit 0

# Only git pushes are in scope; the explicit escape passes through.
grep -qE '(^|[;&|[:space:]])git[[:space:]]+push([[:space:]]|$)' <<<"$cmd" || exit 0
grep -q 'FERROEHR_SKIP_CRATE_BUMP_GUARD=1' <<<"$cmd" && exit 0

git rev-parse --is-inside-work-tree >/dev/null 2>&1 || exit 0
git rev-parse --verify --quiet origin/main >/dev/null || exit 0
git merge-base HEAD origin/main >/dev/null 2>&1 || exit 0

guard="$(dirname "${BASH_SOURCE[0]}")/../../scripts/checks/crate-version-guard.sh"
verdict=0
out="$(bash "$guard" origin/main HEAD 2>&1)" || verdict=$?
reason="${out//::error::/}"
case "$verdict" in
0) exit 0 ;;
1)
  echo "BLOCKED: $reason" >&2
  echo "If this change must set the version by hand, re-run with FERROEHR_SKIP_CRATE_BUMP_GUARD=1 and apply the 'no-crate-bump' label to the PR." >&2
  exit 2
  ;;
*)
  echo "BLOCKED: $guard failed (exit $verdict), so the crate version could not be judged:" >&2
  echo "$reason" >&2
  exit 2
  ;;
esac
