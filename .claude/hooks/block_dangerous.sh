#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# .claude/hooks/block_dangerous.sh
#
# Claude Code PreToolUse hook (matcher: Bash). Blocks destructive commands:
#   - recursive forced deletes: rm -rf / rm -fr / rm --recursive --force, and
#     find -delete / find -exec rm, unless every target is under /tmp
#   - force-pushes touching main/master, bare force-pushes from a branch that
#     is not a conventional-type one, and pushes that delete main/master
#   - deletion of docs/plans/WORKLIST.md or README.md (the single tracker + guide)
#   - deletion of the read-only reference/v1 ref (Stage 2 archaeology source)
#
# Before any judgement the command is normalised (#4046), so a spelling of
# the same command is judged as the command: a backslash or a path before the
# binary (`\rm`, `/bin/rm`, `/usr/bin/git`), a launcher in front of it (`env`,
# `sudo`, `command`, `exec`, `nohup`, `time`, `nice`, `xargs`), a shell
# wrapper around it (`sh -c '…'`, `bash -c "…"`, `eval "…"`), and git's
# global options before the verb (`git -C . push`, `git --no-pager push`).
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

# A command starts at the line, after `;`, `|`, `&`, `(`, a backquote or `$(`.
readonly START='(^|[;|&(`]|\$\()'

# One normalisation pass; the caller repeats it until nothing changes, so
# nested spellings (`sudo sh -c "env /usr/bin/git -C . push …"`) unfold.
normalise() {
  sed -E \
    -e 's/(^|[;|&(`[:space:]'"'"'"])\\([A-Za-z])/\1\2/g' \
    -e 's#'"$START"'([[:space:]]*)[^[:space:];|&()`'"'"'"]*/([A-Za-z][-A-Za-z0-9_.]*)#\1\2\3#g' \
    -e 's#(-exec(dir)?[[:space:]]+)[^[:space:];|&]*/rm([[:space:]]|$)#\1rm\3#g' \
    -e 's/'"$START"'[[:space:]]*(sudo|env|command|exec|nohup|time|nice|builtin|xargs)(([[:space:]]+-[^[:space:]]+)*)[[:space:]]+/\1 /g' \
    -e 's/'"$START"'[[:space:]]*(sh|bash|zsh|dash|ksh)(([[:space:]]+-[A-Za-z]+)*)[[:space:]]+-[A-Za-z]*c[[:space:]]+\$?['"'"'"]?/\1; /g' \
    -e 's/'"$START"'[[:space:]]*eval[[:space:]]+['"'"'"]?/\1; /g' \
    -e 's/(^|[^[:alnum:]_.-])git[[:space:]]+(-C|-c|--git-dir|--work-tree|--namespace|--config-env)[[:space:]]+[^[:space:]]+/\1git/g' \
    -e 's/(^|[^[:alnum:]_.-])git[[:space:]]+-[-A-Za-z]+(=[^[:space:]]*)?([[:space:]]|$)/\1git\3/g'
}

norm="$cmd"
for _ in 1 2 3 4 5 6 7 8; do
  next="$(printf '%s\n' "$norm" | normalise)"
  [ "$next" != "$norm" ] || break
  norm="$next"
done

# under_tmp <word>: the word, unquoted, names a path below /tmp.
under_tmp() {
  local word="${1//[\"\']/}"
  case "$word" in
  */../* | */..) return 1 ;;
  /tmp/?* | /private/tmp/?*) return 0 ;;
  *) return 1 ;;
  esac
}

# Recursive forced rm. Each `rm` segment (up to `;`, `|` or `&`) is read word
# by word; it is refused when it is both recursive and forced and any target
# is outside /tmp. No target at all (`xargs rm -rf`) is refused as well: the
# targets arrive from elsewhere and cannot be judged.
while IFS= read -r seg; do
  [ -n "$seg" ] || continue
  read -ra words <<<"$seg"
  recursive=0 force=0 targets=0 outside=0 operands=0
  for w in "${words[@]:1}"; do
    w="${w//[\"\']/}"
    if [ "$operands" -eq 0 ]; then
      case "$w" in
      --) operands=1; continue ;;
      --recursive) recursive=1; continue ;;
      --force) force=1; continue ;;
      --*) continue ;;
      -*)
        case "$w" in *[rR]*) recursive=1 ;; esac
        case "$w" in *f*) force=1 ;; esac
        continue
        ;;
      esac
    fi
    [ -n "$w" ] || continue
    targets=$((targets + 1))
    under_tmp "$w" || outside=1
  done
  if [ "$recursive" -eq 1 ] && [ "$force" -eq 1 ] && { [ "$targets" -eq 0 ] || [ "$outside" -eq 1 ]; }; then
    echo "BLOCKED: 'rm -rf' is not allowed (block_dangerous hook). Delete specific files with 'git rm' or 'rm <file>', or operate under /tmp." >&2
    exit 2
  fi
done < <(printf '%s\n' "$norm" | grep -oE '(^|[;|&(`[:space:]])rm[[:space:]][^;|&]*' | sed -E 's/^[;|&(`[:space:]]//' || true)

# find that deletes (`-delete`, or `-exec`/`-execdir` running rm) is refused
# unless every starting point is under /tmp. A find with no starting point
# searches `.`, the project.
while IFS= read -r seg; do
  [ -n "$seg" ] || continue
  printf '%s' "$seg" | grep -qE '[[:space:]](-delete|-exec(dir)?[[:space:]]+rm)([[:space:]]|$)' || continue
  read -ra words <<<"$seg"
  starts=0 outside=0
  for w in "${words[@]:1}"; do
    case "$w" in
    -H | -L | -P) continue ;;
    -* | '(' | '\(' | '!') break ;;
    esac
    starts=$((starts + 1))
    under_tmp "$w" || outside=1
  done
  if [ "$starts" -eq 0 ] || [ "$outside" -eq 1 ]; then
    echo "BLOCKED: a deleting 'find' outside /tmp is not allowed (block_dangerous hook). Delete specific files with 'git rm' or 'rm <file>', or operate under /tmp." >&2
    exit 2
  fi
done < <(printf '%s\n' "$norm" | grep -oE '(^|[;|&(`[:space:]])find[[:space:]][^;|&]*' | sed -E 's/^[;|&(`[:space:]]//' || true)

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
# inside another command's argument is prose, not a push. A push that
# deletes a protected branch (`--delete`, `-d`, or a `:main` refspec) is
# refused like a force push.
force_re='(--force([^-]|$)|--force-with-lease|--force-if-includes|[[:space:]]-[A-Za-z]*f[A-Za-z]*([[:space:]]|$)|[[:space:]]\+[[:alnum:]])'
delete_re='(--delete([[:space:]]|$)|[[:space:]]-d([[:space:]]|$)|[[:space:]]:[[:alnum:]])'
protected_re='(^|[[:space:]:/+])(main|master)([[:space:]]|$|["'"'"';&|])'
while IFS= read -r seg; do
  [ -n "$seg" ] || continue
  if printf '%s' "$seg" | grep -qE "$delete_re" && printf '%s' "$seg" | grep -qE "$protected_re"; then
    echo "BLOCKED: deleting main/master on a remote is forbidden (CLAUDE.md hard rule)." >&2
    exit 2
  fi
  printf '%s' "$seg" | grep -qE "$force_re" || continue
  if printf '%s' "$seg" | grep -qE "$protected_re"; then
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
done < <(printf '%s\n' "$norm" | grep -oE "$START"'[[:space:]]*([A-Za-z_][A-Za-z0-9_]*=[^[:space:]]*[[:space:]]+)*git[[:space:]]+push[^;|&]*' || true)

# Never delete the tracker pointer or the plans guide (the tracker itself is
# GitHub Issues; these files are the pointer + lifecycle guide). Completed
# plan files may be pruned once their close is recorded in the PR description.
if printf '%s' "$norm" | grep -qE '(^|[;&|[:space:]])(git[[:space:]]+rm|rm)[^;|&]*docs/plans/(WORKLIST\.md|README\.md)'; then
  echo "BLOCKED: docs/plans/WORKLIST.md (the tracker pointer) and docs/plans/README.md (the lifecycle guide) must not be deleted. The tracker is GitHub Issues; implemented plan files are deleted in the PR that lands them." >&2
  exit 2
fi

# Never delete the read-only v1 reference ref.
if printf '%s' "$norm" | grep -qE 'git[[:space:]]+(branch[[:space:]]+(-D|-d|--delete)[^;|&]*reference/v1|update-ref[[:space:]]+-d[[:space:]]+refs/heads/reference/v1|push[^;|&]*:[[:space:]]*reference/v1)'; then
  echo "BLOCKED: reference/v1 is the read-only pre-v2 reference (consulted only in Stage 2). It must not be deleted or overwritten." >&2
  exit 2
fi

exit 0
