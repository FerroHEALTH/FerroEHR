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
# Usage: scripts/checks/typed-status.sh [--all | <file>...]
#   no args  → the files changed against origin/main
#   --all    → every tracked .rs file
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_known_flags "[--all | --all-really | <file>...]" "--all --all-really" "$@"
cd "$(dirname "$0")/../.."

# `--all` covers the WHOLE tree. Both parked sweeps have landed: the runner
# (#2054) holds `StatusCode` in memory and applies `.as_u16()` only where a
# number is rendered or serialized, and the viewer (#2055) compares through
# `CdrResponse::is(StatusCode)` and named constants, its `status: u16` DTO
# fields staying numeric because they are deserialized from and rendered into
# wire JSON. `--all-really` is kept as an alias so callers written against
# the parked-era flag keep working.
collect() {
  if [[ "${1:-}" = "--all-really" ]] || [[ "${1:-}" = "--all" ]]; then
    git ls-files '*.rs'
  elif [[ "$#" -gt 0 ]]; then
    printf '%s\n' "$@"
  else
    git diff --name-only origin/main...HEAD -- '*.rs' 2>/dev/null || git ls-files '*.rs'
  fi
}

failures=0
files=$(collect "$@")
[[ -n "$files" ]] || { echo "typed-status: no Rust files to check."; exit 0; }

for f in $files; do
  [[ -f "$f" ]] || continue
  # `.as_u16()` immediately compared, or a `.status()` compared to a literal.
  # shellcheck disable=SC2016 # the backticks quote a Rust path in the message text
  while IFS=: read -r line body; do
    [[ -n "${line:-}" ]] || continue
    printf '%s:%s: numeric status comparison (%s) — compare the typed \n' \
      "$f" "$line" "$(printf '%s' "$body" | sed 's/^[[:space:]]*//' | cut -c1-60)" >&2
    printf '    `http::StatusCode` constant instead (scripts/checks/typed-status.sh)\n' >&2
    failures=$((failures + 1))
  done < <(grep -nE \
    -e 'as_u16\(\)[[:space:]]*[=!]=' \
    -e 'status\(\)[[:space:]]*[=!]=[[:space:]]*[0-9]' \
    -e '\b[a-z_]*status[a-z_]*[[:space:]]*[=!]=[[:space:]]*[0-9]' \
    -e '\b[a-z_]*status[a-z_()]*[[:space:]]*[<>]=?[[:space:]]*[1-9][0-9]{2}([^0-9]|$)' \
    -e 'matches!\([[:space:]]*[a-z_]*status[a-z_]*[[:space:]]*,[[:space:]]*[0-9]' \
    -e 'assert(_eq|_ne)?!\([^,]*status\(\)[^,]*,[[:space:]]*[0-9]' \
    "$f" || true)
  # A `match` on a status-named scrutinee with bare numeric-literal arms, and a
  # struct PATTERN pinning a status field to a literal (`{ status: 412, .. }` —
  # the `..` rest token is what separates a pattern from legal construction).
  # Both are comparisons the `[=!]=` shapes above cannot see; the arm window is
  # scrutinee-anchored so openEHR's own 3-digit codes (249/523/…) matched under
  # non-status scrutinees stay out of scope.
  # shellcheck disable=SC2016 # the backticks quote a Rust path in the message text
  while IFS=: read -r line body; do
    [[ -n "${line:-}" ]] || continue
    printf '%s:%s: numeric status comparison (%s) — compare the typed \n' \
      "$f" "$line" "$(printf '%s' "$body" | sed 's/^[[:space:]]*//' | cut -c1-60)" >&2
    printf '    `http::StatusCode` constant instead (scripts/checks/typed-status.sh)\n' >&2
    failures=$((failures + 1))
  done < <(awk '
    /match[[:space:]].*status/ { window = 8 }
    window > 0 && /^[[:space:]]*[0-9]{3}[[:space:]]*(\|[[:space:]]*[0-9]{3}[[:space:]]*)*(=>|\|)/ {
      printf "%d:%s\n", NR, $0
    }
    window > 0 { window-- }
    /status:[[:space:]]*[0-9]{3}[[:space:]]*,[[:space:]]*\.\./ { printf "%d:%s\n", NR, $0 }
  ' "$f" || true)
done

if [[ "$failures" -gt 0 ]]; then
  echo "typed-status: $failures violation(s) — see above." >&2
  exit 1
fi
echo "typed-status: OK."
