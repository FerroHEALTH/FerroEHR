#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# .claude/hooks/no_attribution_guard.sh
#
# Claude Code PreToolUse hook (matcher: Bash).
# Belt-and-suspenders companion to the .githooks/commit-msg hook: it blocks a
# `git commit` (or PR-creating command) BEFORE it runs if the command text
# carries any AI/Claude attribution, so Claude is forced to remove it rather
# than relying on the message being rewritten after the fact.
#
# Reads the tool-call JSON on stdin. Exit 2 blocks the tool call and returns
# the stderr text to Claude. Exit 0 allows it.

set -euo pipefail

payload="$(cat)"

# Only concern ourselves with git commit / PR creation commands.
if ! printf '%s' "$payload" | grep -qiE 'git[[:space:]]+commit|gh[[:space:]]+pr[[:space:]]+create|git[[:space:]]+push'; then
  exit 0
fi

# Attribution tokens we refuse to let through.
if printf '%s' "$payload" | grep -qiE 'co-authored-by:.*(claude|anthropic|\[bot\])|generated[[:space:]]+with[[:space:]]+.*claude|generated[[:space:]]+by[[:space:]]+.*claude|claude[[:space:]]+code|🤖|assisted-by:.*claude|anthropic[[:space:]]+claude'; then
  echo "BLOCKED: this commit/PR command contains AI/Claude attribution (Co-authored-by, 'Generated with Claude', 🤖, etc.). Remove all such lines from the message/description and retry. Commit and PR text must describe only the change. This is a hard project rule." >&2
  exit 2
fi

exit 0
