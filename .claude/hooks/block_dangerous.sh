#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# .claude/hooks/block_dangerous.sh
#
# Claude Code PreToolUse hook (matcher: Bash). Blocks destructive commands:
#   - rm -rf / rm -fr (delete specific files, use git rm, or work under /tmp)
#   - force-pushes touching main/master/main, and bare force-pushes
#   - deletion of docs/plans/WORKLIST.md or README.md (the single tracker + guide)
#   - deletion of the read-only reference/v1 ref (Stage 2 archaeology source)
#
# Reads the tool-call JSON on stdin. Exit 2 blocks; exit 0 allows.

set -euo pipefail

payload="$(cat)"

if command -v jq >/dev/null 2>&1; then
  cmd="$(printf '%s' "$payload" | jq -r '.tool_input.command // empty' 2>/dev/null || true)"
else
  cmd="$payload"
fi
[ -n "${cmd:-}" ] || exit 0

# rm with both -r and -f (combined or separate flags), unless scoped to /tmp.
if printf '%s' "$cmd" | grep -qE '(^|[;&|[:space:]])rm[[:space:]]+-[a-zA-Z]*([rR][a-zA-Z]*f|f[a-zA-Z]*[rR])' ||
  printf '%s' "$cmd" | grep -qE '(^|[;&|[:space:]])rm[[:space:]]+(-[a-zA-Z]+[[:space:]]+)*-[a-zA-Z]*[rR][a-zA-Z]*([[:space:]]+-[a-zA-Z]+)*[[:space:]]+-[a-zA-Z]*f'; then
  if ! printf '%s' "$cmd" | grep -qE 'rm[[:space:]]+-[a-zA-Z]+[[:space:]]+"?(/private)?/tmp/'; then
    echo "BLOCKED: 'rm -rf' is not allowed (block_dangerous hook). Delete specific files with 'git rm' or 'rm <file>', or operate under /tmp." >&2
    exit 2
  fi
fi

# Force pushes: never to main/master. The judgement reads the `git push`
# SEGMENT of the command only (up to `;`, `|` or `&`), never the whole line,
# so an issue body or a commit message that mentions a force push is not one
# (#3359). The protected names match as WHOLE REF WORDS inside that segment
# (`refs/heads/main`, `origin main`, `HEAD:main` all hit), never as raw
# substrings, which falsely blocked branches whose names merely CONTAIN a
# protected name (fix/flat-master05-…, #542). A push that names no refspec
# is judged by the branch it would push: the CURRENT one.
# The segment must START a command (the line, or `;`, `|`, `&`, `(`, a
# subshell), optionally after variable assignments, so a `git push` quoted
# inside another command's argument is prose, not a push.
force_re='(--force([^-]|$)|--force-with-lease|[[:space:]]-f([[:space:]]|$)|[[:space:]]\+[[:alnum:]])'
while IFS= read -r seg; do
  [ -n "$seg" ] || continue
  printf '%s' "$seg" | grep -qE "$force_re" || continue
  if printf '%s' "$seg" | grep -qE '(^|[[:space:]:/+])(main|master)([[:space:]]|$|["'"'"';&|])'; then
    echo "BLOCKED: force-push touching main/master is forbidden (CLAUDE.md hard rule)." >&2
    exit 2
  fi
  if printf '%s' "$seg" | grep -qE '(feat|fix|chore|docs|refactor|perf|test|ci|build|release|claude)/'; then
    continue
  fi
  current="$(git -C "${CLAUDE_PROJECT_DIR:-.}" branch --show-current 2>/dev/null || true)"
  case "$current" in
  main | master | "")
    echo "BLOCKED: a force-push that names no branch would push '${current:-a detached HEAD}'; force-push (prefer --force-with-lease) only an explicit conventional-type branch (feat/, fix/, chore/, docs/, refactor/, perf/, test/, ci/, build/, release/)." >&2
    exit 2
    ;;
  feat/* | fix/* | chore/* | docs/* | refactor/* | perf/* | test/* | ci/* | build/* | release/* | claude/*) ;;
  *)
    echo "BLOCKED: a force-push that names no branch would push '$current', which is not a conventional-type branch (feat/, fix/, chore/, docs/, refactor/, perf/, test/, ci/, build/, release/)." >&2
    exit 2
    ;;
  esac
done < <(printf '%s' "$cmd" | grep -oE '(^|[;|&(`]|\$\()[[:space:]]*([A-Za-z_][A-Za-z0-9_]*=[^[:space:]]*[[:space:]]+)*git[[:space:]]+push[^;|&]*' || true)

# Never delete the tracker pointer or the plans guide (the tracker itself is
# GitHub Issues; these files are the pointer + lifecycle guide). Completed
# plan files may be pruned once their close is recorded in the PR description.
if printf '%s' "$cmd" | grep -qE '(^|[;&|[:space:]])(git[[:space:]]+rm|rm)[^;|&]*docs/plans/(WORKLIST\.md|README\.md)'; then
  echo "BLOCKED: docs/plans/WORKLIST.md (the tracker pointer) and docs/plans/README.md (the lifecycle guide) must not be deleted. The tracker is GitHub Issues; implemented plan files are deleted in the PR that lands them." >&2
  exit 2
fi

# Never delete the read-only v1 reference ref.
if printf '%s' "$cmd" | grep -qE 'git[[:space:]]+(branch[[:space:]]+(-D|-d|--delete)[^;|&]*reference/v1|update-ref[[:space:]]+-d[[:space:]]+refs/heads/reference/v1|push[^;|&]*:[[:space:]]*reference/v1)'; then
  echo "BLOCKED: reference/v1 is the read-only pre-v2 reference (consulted only in Stage 2). It must not be deleted or overwritten." >&2
  exit 2
fi

exit 0
