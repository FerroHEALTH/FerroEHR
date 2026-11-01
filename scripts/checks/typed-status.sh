#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Typed-status guard (owner directive 2026-08-06).
#
# An HTTP status is compared as a TYPE, never as a number:
#
#     if status == StatusCode::OK { … }          // yes
#     if status.as_u16() == 200 { … }            // no
#     if status == 200 { … }                     // no
#
# `http::StatusCode` exists so the compiler catches a typo'd or misremembered
# code; `as_u16() == 401` throws that away, and a bare literal tells a reader
# nothing about which of the 4xx family was meant. The `http` crate names every
# registered code (docs.rs/http/latest/http/status/struct.StatusCode.html), so
# there is always a constant to use.
#
# Rendering the number is fine and stays legal — a log field, a metric label, a
# recorded wire outcome (`status.as_u16()` inside `format!`, a struct field, a
# `/ 100` class bucket). Only COMPARISON against a numeric literal is refused.
#
# The three shapes below are all the same violation, and the guard has to see all
# three or it reports OK on a tree containing the pattern it exists to ban (which
# it did, on two live sites, until this was widened):
#
#     if status.as_u16() == 401 { … }                    // direct
#     let status = resp.status().as_u16(); if status == 401 { … }   // via a local
#     assert_eq!(resp.status(), 200);                    // via an assertion
#
# The second is caught by matching the COMPARISON of a status-named binding
# against a literal, wherever it sits — not by flagging the `as_u16()` binding
# itself, and not by flagging a `status: u16` parameter. Those two broader rules
# were tried and rejected on merit: they fire on code that legitimately RECEIVES a
# recorded wire number, which is the conformance runner's whole job (its catalogue
# stores expected statuses as JSON numbers and compares recorded against expected
# — a data-driven comparison with no literal in it, and correct as it stands).
# What the rule forbids is a LITERAL in the comparison, so that is what the guard
# looks for, in every form it can take: direct, through a local, in a `matches!`,
# and in an assertion.
#
# Usage: scripts/checks/typed-status.sh [--all | --files <file>... | <file>...]
#   no args  → every tracked .rs file (the CI job)
#   --all    → the same
#   --files  → the named files, as bare paths do (the per-edit hook)
#
# A named file is read from the repository root, else from the caller's
# directory, so a path relative to either works.
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_known_flags "[--all | --all-really | --files <file>... | <file>...]" "--all --all-really --files" "$@"
caller="$PWD"
cd "$(dirname "$0")/../.."

# `--all-really` is kept as an alias of `--all`, the whole tree, so callers
# written against the parked-era flag keep working. The no-argument run
# reads the whole tree too: a run against origin/main read only committed
# changes and passed a worktree with edits as "no Rust files" (#4591).
files=()
if [[ "$#" -eq 0 || "${1:-}" = "--all" || "${1:-}" = "--all-really" ]]; then
  while IFS= read -r f; do
    [[ -f "$f" ]] && files+=("$f")
  done < <(git ls-files '*.rs')
  echo "typed-status: checking ${#files[@]} tracked .rs files."
else
  [[ "${1:-}" = "--files" ]] && shift
  for f in "$@"; do
    if [[ -f "$f" ]]; then
      files+=("$f")
    elif [[ -f "$caller/$f" ]]; then
      files+=("$caller/$f")
    fi
  done
fi
[[ "${#files[@]}" -gt 0 ]] || { echo "typed-status: no Rust files to check."; exit 0; }

# Prints `<file>:<line>:<text>` for every numeric status comparison, one grep
# and one awk over all the files (#4591).
hits() {
  # `.as_u16()` immediately compared, or a `.status()` compared to a literal.
  printf '%s\0' "${files[@]}" | xargs -0 grep -nHE \
    -e 'as_u16\(\)[[:space:]]*[=!]=' \
    -e 'status\(\)[[:space:]]*[=!]=[[:space:]]*[0-9]' \
    -e '\b[a-z_]*status[a-z_]*[[:space:]]*[=!]=[[:space:]]*[0-9]' \
    -e '\b[a-z_]*status[a-z_()]*[[:space:]]*[<>]=?[[:space:]]*[1-9][0-9]{2}([^0-9]|$)' \
    -e 'matches!\([[:space:]]*[a-z_]*status[a-z_]*[[:space:]]*,[[:space:]]*[0-9]' \
    -e 'assert(_eq|_ne)?!\([^,]*status\(\)[^,]*,[[:space:]]*[0-9]' || true
  # A `match` on a status-named scrutinee with bare numeric-literal arms, and a
  # struct PATTERN pinning a status field to a literal (`{ status: 412, .. }` —
  # the `..` rest token is what separates a pattern from legal construction).
  # Both are comparisons the `[=!]=` shapes above cannot see; the arm window is
  # scrutinee-anchored so openEHR's own 3-digit codes (249/523/…) matched under
  # non-status scrutinees stay out of scope.
  # shellcheck disable=SC2016 # an awk program, which xargs runs
  printf '%s\0' "${files[@]}" | xargs -0 awk '
    FNR == 1 { window = 0 }
    /match[[:space:]].*status/ { window = 8 }
    window > 0 && /^[[:space:]]*[0-9]{3}[[:space:]]*(\|[[:space:]]*[0-9]{3}[[:space:]]*)*(=>|\|)/ {
      printf "%s:%d:%s\n", FILENAME, FNR, $0
    }
    window > 0 { window-- }
    /status:[[:space:]]*[0-9]{3}[[:space:]]*,[[:space:]]*\.\./ { printf "%s:%d:%s\n", FILENAME, FNR, $0 }
  '
}

failures=0
# shellcheck disable=SC2016 # the backticks quote a Rust path in the message text
while IFS=: read -r f line body; do
  [[ -n "${line:-}" ]] || continue
  printf '%s:%s: numeric status comparison (%s) — compare the typed \n' \
    "$f" "$line" "$(printf '%s' "$body" | sed 's/^[[:space:]]*//' | cut -c1-60)" >&2
  printf '    `http::StatusCode` constant instead (scripts/checks/typed-status.sh)\n' >&2
  failures=$((failures + 1))
done < <(hits | LC_ALL=C sort -t: -k1,1 -k2,2n)

if [[ "$failures" -gt 0 ]]; then
  echo "typed-status: $failures violation(s) — see above." >&2
  exit 1
fi
echo "typed-status: OK."
