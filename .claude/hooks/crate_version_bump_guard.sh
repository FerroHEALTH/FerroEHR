#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# .claude/hooks/crate_version_bump_guard.sh
#
# Claude Code PreToolUse hook (matcher: Bash). Blocks `git push` when the
# outgoing commits (vs the merge-base with origin/main) change PACKAGED
# content of the published crates/* members without bumping the lockstep
# 0.0.x crate version (.claude/rules/crates-publishing.md — published
# versions are immutable), and when a bump leaves fuzz/Cargo.lock behind.
# The authoritative twin is the crate-version-guard CI job; this hook fails
# the push before CI would.
#
# Escape hatch (mirrors the CI `no-crate-bump` label, for diffs that provably
# do not alter packaged bytes): FERROEHR_SKIP_CRATE_BUMP_GUARD=1 git push …
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

# Only git pushes are in scope; the explicit escape passes through.
printf '%s' "$cmd" | grep -qE '(^|[;&|[:space:]])git[[:space:]]+push([[:space:]]|$)' || exit 0
printf '%s' "$cmd" | grep -q 'FERROEHR_SKIP_CRATE_BUMP_GUARD=1' && exit 0

git rev-parse --is-inside-work-tree >/dev/null 2>&1 || exit 0
base="$(git merge-base HEAD origin/main 2>/dev/null || true)"
[ -n "$base" ] || exit 0

changed="$(git diff --name-only "$base" HEAD 2>/dev/null || true)"
packaged_change=0
# The packaged-path scope is shared with the CI twin, so the two cannot drift.
classify="$(dirname "${BASH_SOURCE[0]}")/../../scripts/checks/crate-packaged-paths.sh"
verdict=0
printf '%s\n' "$changed" | bash "$classify" || verdict=$?
case "$verdict" in
0) packaged_change=1 ;;
1) ;;
*)
  echo "BLOCKED: $classify failed (exit $verdict), so the packaged-content scope could not be judged." >&2
  exit 2
  ;;
esac

# A ROOT [workspace.dependencies] version change alters the PACKAGED manifest
# of every crates/* member that consumes the entry with `workspace = true` —
# `cargo package` renders the concrete requirement — without touching any
# crates/* file, which is how PR #3021 slipped past the path scope (#3036).
if [ "$packaged_change" -eq 0 ] && printf '%s\n' "$changed" | grep -qx 'Cargo.toml'; then
  table_names="$(awk '/^\[workspace\.dependencies\]/{f=1;next} /^\[/{f=0} f && /^[A-Za-z0-9_-]+[. =]/{print $1}' Cargo.toml | sed 's/\.workspace$//' | sort -u)"
  diff_names="$(git diff "$base" HEAD -- Cargo.toml | grep -E '^[+-][A-Za-z0-9_-]+(\.workspace)?[[:space:]]*=' | sed -E 's/^[+-]//; s/[[:space:]]*=.*//; s/\.workspace$//' | sort -u)"
  for name in $diff_names; do
    printf '%s\n' "$table_names" | grep -qx "$name" || continue
    if grep -lE "^${name}(\.workspace)?[[:space:]]*=" crates/openehr-*/Cargo.toml >/dev/null 2>&1; then
      packaged_change=1
      echo "crate-bump-guard: workspace dependency '$name' changed and crates/* members consume it — packaged requirements move." >&2
      break
    fi
  done
fi

[ "$packaged_change" -eq 1 ] || exit 0

old_ver="$(git show "$base:crates/openehr-base/Cargo.toml" 2>/dev/null | grep -m1 '^version = ' || true)"
new_ver="$(grep -m1 '^version = ' crates/openehr-base/Cargo.toml 2>/dev/null || true)"
if [ -n "$old_ver" ] && [ "$old_ver" = "$new_ver" ]; then
  echo "BLOCKED: the outgoing commits change packaged content of the published crates/* members without bumping the lockstep 0.0.x crate version (crates-publishing rule; published versions are immutable). Bump all nine 'version' fields + the internal version requirements in this branch — or, if the diff provably does not alter packaged bytes, re-run with FERROEHR_SKIP_CRATE_BUMP_GUARD=1 and apply the 'no-crate-bump' label to the PR." >&2
  exit 2
fi

# fuzz/ is its own workspace, so its lock records the nine by path dependency
# and goes stale silently when the bump lands — the fuzz lane then builds
# against manifests the lock contradicts.
ver="$(printf '%s' "$new_ver" | sed -E 's/version = "([^"]+)".*/\1/')"
for c in base rm am adl term lang query its sdt; do
  locked="$(awk -v n="\"openehr-$c\"" '$1 == "name" && $3 == n { hit = 1; next } hit && $1 == "version" { gsub(/"/, "", $3); print $3; exit }' fuzz/Cargo.lock 2>/dev/null || true)"
  if [ "$locked" != "$ver" ]; then
    echo "BLOCKED: fuzz/Cargo.lock records openehr-$c ${locked:-nothing} but the lockstep crate version is $ver. Refresh it with 'cargo update --manifest-path fuzz/Cargo.toml --workspace' and commit the lock in this branch." >&2
    exit 2
  fi
done

exit 0
