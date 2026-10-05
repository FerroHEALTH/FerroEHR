#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Proves block_dangerous.sh in both directions (#3359): a scratch repository on
# a named branch stands in for the project, each shape is fed as the tool-call
# JSON the hook reads, and the exit code is asserted. Run by the shellcheck CI
# job beside the lane, and by hand: bash .claude/hooks/block_dangerous_test.sh
set -euo pipefail
cd "$(dirname "$0")/../.." || exit 1
readonly HOOK='.claude/hooks/block_dangerous.sh'

tmp="$(mktemp -d)"
# shellcheck disable=SC2064 # expand $tmp now, while it is still in scope
trap "rm -rf '$tmp'" EXIT
git -C "$tmp" init -q --initial-branch=main .

failures=0
# expect <0|2> <branch> <command>
expect() {
  local want="$1" branch="$2" command="$3" got payload
  git -C "$tmp" switch -qc "$branch" 2>/dev/null || git -C "$tmp" switch -q "$branch"
  payload="$(jq -cn --arg c "$command" '{tool_input: {command: $c}}')"
  if CLAUDE_PROJECT_DIR="$tmp" bash "$HOOK" <<<"$payload" 2>/dev/null; then got=0; else got=$?; fi
  if [[ "$got" != "$want" ]]; then
    echo "block_dangerous_test: on '$branch', expected exit $want, got $got for: $command" >&2
    failures=1
  fi
}

# Protected refs by name, wherever the segment spells them.
expect 2 feat/x 'git push --force origin main'
expect 2 feat/x 'git push --force-with-lease origin HEAD:master'
expect 2 feat/x 'git push -f origin refs/heads/main'
# An explicit conventional-type branch passes; a protected-looking substring does not trip it.
expect 0 main 'git push --force-with-lease origin feat/x:feat/x'
expect 0 main 'git push --force-with-lease origin fix/flat-master05-audit'
# A bare force push is judged by the CURRENT branch.
expect 2 main 'git push --force-with-lease'
expect 0 feat/x 'git push --force-with-lease'
expect 2 scratch 'git push --force-with-lease'
# Prose that mentions a force push in another command is not a push.
expect 0 main 'gh issue create --body "the hook refused git push --force-with-lease on main"'
expect 0 main 'git commit -m "docs: describe when a force-push to main is refused"'
# A plain push is never a force push.
expect 0 main 'git push origin main'
# The other refusals still hold.
expect 2 feat/x 'rm -rf target'
expect 0 feat/x 'rm -rf /tmp/scratch'

if [[ "$failures" -ne 0 ]]; then
  exit 1
fi
echo "block_dangerous_test: OK (protected refs refused, explicit branches allowed, bare pushes judged by the current branch, prose ignored)."
