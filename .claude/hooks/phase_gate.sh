#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# .claude/hooks/phase_gate.sh
#
# Claude Code Stop hook: blocks ending a session in which no commit was made
# AND no tracker activity happened — the tracker is GitHub Issues (CLAUDE.md
# issue workflow): an issue created, commented, edited, or closed since the
# session started counts as recording the work.
#
# Uses .claude/.session-start-head + .claude/.session-start-time written by
# inject_phase_context.sh at SessionStart. Exit 2 blocks the stop once; a
# second stop attempt (with stop_hook_active=true) is allowed through so
# purely informational sessions can still end.

set -uo pipefail

payload="$(cat)" || true

# Do not loop: if we already blocked once this stop, let the session end.
if printf '%s' "$payload" | grep -q '"stop_hook_active"[[:space:]]*:[[:space:]]*true'; then
  exit 0
fi

root="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
cd "$root" || exit 0

marker=".claude/.session-start-head"
[ -f "$marker" ] || exit 0 # no baseline recorded; cannot judge — do not nag

head_now="$(git rev-parse HEAD 2>/dev/null || true)"
[ -n "$head_now" ] || exit 0

if [ "$(cat "$marker")" != "$head_now" ]; then
  exit 0 # at least one commit was made this session
fi

# No commit yet: allow the stop if any issue activity (create/comment/edit/
# close) happened since session start. The issues API's `since` filters on
# last-updated; a non-empty result means the tracker was touched.
ts_marker=".claude/.session-start-time"
if [ -f "$ts_marker" ] && command -v gh >/dev/null 2>&1; then
  since="$(cat "$ts_marker")"
  touched="$(gh api "repos/{owner}/{repo}/issues?state=all&since=${since}&per_page=1" --jq 'length' 2>/dev/null || echo "")"
  if [ "${touched:-0}" = "1" ]; then
    exit 0
  fi
  # gh failed (offline/auth): cannot judge — do not nag.
  [ -n "$touched" ] || exit 0
fi

echo "tracker gate: no commit was made and no GitHub issue was created/commented/updated this session. Follow the issue workflow (CLAUDE.md): record what you did on the tracker — tick the issue's exit-criteria checkboxes ('gh issue edit <n>'), post a status comment ('gh issue comment <n>'), or open an issue for newly-registered work — and commit on a conventional-type branch (feat/, fix/, chore/, ...). If this session was purely informational, stop again to end anyway." >&2
exit 2
