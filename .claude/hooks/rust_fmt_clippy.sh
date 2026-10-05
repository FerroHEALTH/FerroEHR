#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# .claude/hooks/rust_fmt_clippy.sh
#
# Claude Code PostToolUse hook (matcher: Write|Edit).
# Formats an edited .rs file with rustfmt. Never blocks; swallows all failures
# (rustfmt failing to parse a draft is expected and fine).
#
# NOTE: this hook never runs clippy. A per-edit `cargo clippy` on the owning
# crate check-builds that crate plus its dependency cone on every file edit,
# and running it per-package (from the crate dir) gives resolver-v3 feature
# unification that differs from the workspace build — invalidating shared
# artifacts and thrashing the cargo cache (target/ past 190 GB, 10+ minute dev
# builds). Clippy is a per-phase gate the agent runs explicitly
# (`cargo clippy --workspace --all-targets`), never a per-edit hook.

set -uo pipefail

payload="$(cat)" || true

if command -v jq >/dev/null 2>&1; then
  file_path="$(printf '%s' "$payload" | jq -r '.tool_input.file_path // empty' 2>/dev/null)" || true
else
  file_path="$(printf '%s' "$payload" | sed -n 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"
fi

repo_root="${CLAUDE_PROJECT_DIR:-$(cd "$(dirname "$0")/../.." && pwd)}"

case "${file_path:-}" in
*.rs) ;;
*.sh)
  # Per-edit shell lint (#2825), the same one check the per-PR lane runs
  # (scripts/checks/shellcheck-lane.sh in single-file mode), so a finding
  # surfaces at the edit instead of at PR time. Skipped silently when the
  # linter is absent; the PR lane still gates.
  [ -f "$file_path" ] || exit 0
  if command -v shellcheck >/dev/null 2>&1 \
    && [ -x "$repo_root/scripts/checks/shellcheck-lane.sh" ]; then
    findings="$("$repo_root/scripts/checks/shellcheck-lane.sh" "$file_path" 2>&1)" || {
      printf '%s\n' "$findings" >&2
      exit 2
    }
  fi
  exit 0
  ;;
*) exit 0 ;;
esac
[ -f "$file_path" ] || exit 0

rustfmt --edition 2024 "$file_path" >/dev/null 2>&1 || true

# Comment-style guard (.claude/rules/comments.md): block comments, TODO(#N)
# form, NOTE/essay budgets. Exit 2 feeds the findings back as a correction.
repo_root="${CLAUDE_PROJECT_DIR:-$(cd "$(dirname "$0")/../.." && pwd)}"
if [ -x "$repo_root/scripts/checks/comment-style.sh" ]; then
  findings="$("$repo_root/scripts/checks/comment-style.sh" --files "$file_path" 2>&1)" || {
    printf '%s\n' "$findings" >&2
    exit 2
  }
fi

# Spec-citation guard (.claude/rules/spec-adherence.md): every openEHR spec
# file a comment cites must exist under docs/specs/openehr/ — a citation that
# names no real file reads as authority while providing none.
if [ -x "$repo_root/scripts/checks/spec-citations.sh" ]; then
  findings="$("$repo_root/scripts/checks/spec-citations.sh" "$file_path" 2>&1)" || {
    printf '%s\n' "$findings" >&2
    exit 2
  }
fi

# Default-value style guard (.claude/rules/rust-style.md §Default values): the
# default belongs inline in the struct's own `Default` impl. Per-file mode only
# — the single-reader `const` check needs the whole tree and runs in CI.
if [ -x "$repo_root/scripts/checks/default-style.sh" ]; then
  findings="$("$repo_root/scripts/checks/default-style.sh" "$file_path" 2>&1)" || {
    printf '%s\n' "$findings" >&2
    exit 2
  }
fi

# Typed-status guard (.claude/rules/rust-style.md §HTTP statuses): a status is
# compared as a `StatusCode`, never against a numeric literal.
if [ -x "$repo_root/scripts/checks/typed-status.sh" ]; then
  findings="$("$repo_root/scripts/checks/typed-status.sh" "$file_path" 2>&1)" || {
    printf '%s\n' "$findings" >&2
    exit 2
  }
fi

# E2E wait guard (.claude/rules/leptos-ui.md §10): a browser journey holds the
# bounded `common::Element`, never a raw `WebElement`/`SelectElement`, and waits
# on a condition rather than a timer. Whole-scope only (the guard reads every
# journey file), so it runs when the edited file IS one.
case "$file_path" in
*/app/ferroehr-viewer/tests/it/e2e_*.rs)
  if [ -x "$repo_root/scripts/checks/e2e-waits.sh" ]; then
    findings="$("$repo_root/scripts/checks/e2e-waits.sh" 2>&1)" || {
      printf '%s\n' "$findings" >&2
      exit 2
    }
  fi
  ;;
esac

exit 0
