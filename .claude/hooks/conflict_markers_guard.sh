#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# .claude/hooks/conflict_markers_guard.sh
#
# PreToolUse hook (matcher: Bash).
# Blocks a `git commit` BEFORE it runs while a file staged in the checkout
# the command runs in holds a merge-conflict marker
# (scripts/checks/conflict-markers.sh --staged, #4900).
#
# Reads the tool-call JSON on stdin. Exit 2 blocks the tool call and returns
# the stderr text. Exit 0 allows it.

set -euo pipefail

payload="$(cat)"

if ! grep -qE 'git[[:space:]]+commit' <<<"$payload"; then
  exit 0
fi

# The checkout the command runs in: a `cd <dir>` it opens with, else the
# session's working directory.
dir="$(sed -nE 's/.*"command"[[:space:]]*:[[:space:]]*"cd[[:space:]]+([^"&;[:space:]]+).*/\1/p' <<<"$payload" | sed -n 1p)"
if [[ -z "$dir" ]]; then
  dir="$(sed -nE 's/.*"cwd"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/p' <<<"$payload" | sed -n 1p)"
fi
dir="${dir:-$CLAUDE_PROJECT_DIR}"
git -C "$dir" rev-parse --show-toplevel >/dev/null 2>&1 || exit 0

report="$(mktemp)"
trap 'rm -f "$report"' EXIT
if ! bash "$CLAUDE_PROJECT_DIR/scripts/checks/conflict-markers.sh" --staged "$dir" >/dev/null 2>"$report"; then
  cat "$report" >&2
  echo "BLOCKED: a staged file holds an unresolved merge conflict; resolve it before you commit." >&2
  exit 2
fi
exit 0
