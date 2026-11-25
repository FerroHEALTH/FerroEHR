#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The changed paths that can change what a build runs (#6519): reads the
# changed paths of BASE..HEAD on stdin and prints each one except a Rust
# source whose every changed line, in the diff from the fork point, is a line
# comment or blank. A comment-only change still reaches clippy and rustdoc,
# which read comments; the lanes that only run code (the tests, the
# instruction counts, the release nesting build, the browser lanes, the
# SemVer judge) read this list. No specification governs this: our own
# design.
#
# Usage:
#   scripts/lib/comment-only.sh <base> <head> < paths
#   scripts/lib/comment-only.sh --self-test
set -euo pipefail

# Whether PATH's diff between FORK and HEAD changes only comment or blank
# lines; a deleted, added or binary file is never comment-only.
comment_only() {
  local fork="$1" head="$2" path="$3" diff lines
  [[ "$path" == *.rs ]] || return 1
  git cat-file -e "$fork:$path" 2>/dev/null || return 1
  git cat-file -e "$head:$path" 2>/dev/null || return 1
  diff="$(git diff -U0 --no-renames "$fork" "$head" -- "$path")"
  lines="$(sed -n -E '/^(\+\+\+|---) /d; /^[+-]/p' <<<"$diff")"
  [[ -n "$lines" ]] || return 1
  ! grep -qvE '^[+-][[:space:]]*(//.*)?$' <<<"$lines"
}

filter() {
  local base="$1" head="$2" fork path
  fork="$(git merge-base "$base" "$head")" || return 1
  while IFS= read -r path; do
    [[ -n "$path" ]] || continue
    comment_only "$fork" "$head" "$path" || printf '%s\n' "$path"
  done
}

self_test() {
  local work fail=0 base got
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' RETURN
  g() { git -C "$work" -c user.name=self-test -c user.email=self-test@invalid -c commit.gpgsign=false -c core.hooksPath=/dev/null "$@"; }
  g init -q -b main
  printf 'fn a() {}\n// old\n' >"$work/a.rs"
  printf 'fn b() {}\n' >"$work/b.rs"
  printf 'text\n' >"$work/c.md"
  g add . && g commit -qm base
  base="$(g rev-parse HEAD)"
  printf 'fn a() {}\n// new\n/// doc\n\n' >"$work/a.rs"
  printf 'fn b() { let _x = 1; } // and a comment\n' >"$work/b.rs"
  printf 'more\n' >"$work/c.md"
  printf 'fn d() {}\n' >"$work/d.rs"
  g add . && g commit -qm head
  got="$(cd "$work" && printf 'a.rs\nb.rs\nc.md\nd.rs\n' | filter "$base" HEAD | tr '\n' ' ')"
  if [[ "$got" != "b.rs c.md d.rs " ]]; then
    echo "comment-only: self-test: expected 'b.rs c.md d.rs ', got '$got'" >&2
    fail=1
  fi
  [[ "$fail" -eq 0 ]] || return 1
  echo "comment-only: self-test OK (a comment-only source dropped; code, a new file and a non-Rust path kept)."
}

case "${1:-}" in
--self-test) self_test ;;
"" | -*)
  echo "usage: $0 <base> <head> < paths | --self-test" >&2
  exit 2
  ;;
*) filter "$1" "${2:?usage: $0 <base> <head> < paths}" ;;
esac
