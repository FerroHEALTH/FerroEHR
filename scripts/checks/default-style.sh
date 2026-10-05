#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Default-value style guard (owner directive 2026-08-06; the shape of RFC 3681,
# https://rust-lang.github.io/rfcs/3681-default-field-values.html).
#
# A field's default value belongs in ONE hand-written `impl Default` for its
# struct, written inline:
#
#     impl Default for OidcConfig {
#         fn default() -> Self {
#             Self { clock_skew_leeway_seconds: 60, allow_insecure_issuer: false }
#         }
#     }
#
# with container-level `#[serde(default)]` so serde fills omitted fields from it.
# That is exactly what RFC 3681's `clock_skew_leeway_seconds: u64 = 60` expands
# to; the syntax itself is nightly-only (feature `default_field_values`,
# https://github.com/rust-lang/rust/issues/132162), so we hand-write the
# expansion and this guard keeps the tree in that shape.
#
# Three forms fail:
#   1. `#[serde(default = "path")]`  — the per-field path form. The default then
#      lives in a function instead of the Default impl, so `Default::default()`
#      and a deserialized value can disagree.
#   2. `fn default_<field>()`        — a helper whose only purpose is one field's
#      default value.
#   3. `const DEFAULT_<X>` with exactly ONE reference — a constant that is not
#      shared is just a default value spelled far from its struct.
#
# What stays legal: a `const` with MORE THAN ONE consumer (a spec-fixed value
# like `service::DEFAULT_SYSTEM_ID`), referenced from inside a Default impl; and
# `#[serde(default)]` with no path, which is the required form.
#
# Usage: scripts/checks/default-style.sh [--all | <file>...]
#   no args  → the files changed against origin/main
#   --all    → every .rs file, tracked or untracked (unignored)
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_known_flags "[--all | <file>...]" "--all" "$@"
cd "$(dirname "$0")/../.."

# `Option::default` and `Vec::default` are std paths, not project helpers: they
# say "absent" on a field whose Default is already that, which is redundant but
# not a second home for a value. They are reported as a hint, never a failure.
STD_PATHS='Option::default|Vec::default|String::default|bool::default'

collect() {
  if [[ "${1:-}" = "--all" ]]; then
    # -co --exclude-standard: tracked AND untracked (unignored) files — a new
    # file is checkable before it is ever staged.
    git ls-files -co --exclude-standard '*.rs'
  elif [[ "$#" -gt 0 ]]; then
    printf '%s\n' "$@"
  else
    git diff --name-only origin/main...HEAD -- '*.rs' 2>/dev/null || git ls-files '*.rs'
  fi
}

failures=0
report() {
  printf '%s\n' "$1" >&2
  failures=$((failures + 1))
}

files=$(collect "$@")
[[ -n "$files" ]] || { echo "default-style: no Rust files to check."; exit 0; }

for f in $files; do
  [[ -f "$f" ]] || continue

  # (1) the per-field serde path form
  while IFS=: read -r line body; do
    [[ -n "${line:-}" ]] || continue
    printf '%s' "$body" | grep -qE "$STD_PATHS" && continue
    report "$f:$line: \`#[serde(default = \"…\")]\` — put the value in the struct's \
\`impl Default\` and use container-level \`#[serde(default)]\` instead \
(scripts/checks/default-style.sh)"
  done < <(grep -nE '#\[serde\((.*, )?default = "' "$f" | sed 's/^\([0-9]*\):\(.*\)$/\1:\2/' || true)

  # (2) a helper function that exists to be one field's default. The signature
  # is what distinguishes it from a legitimate domain function: a default-value
  # helper takes NO arguments AND RETURNS the value
  # (`fn default_bind() -> String`). That excludes `default_provider(&self)`,
  # `default_committer(&self)` and `default_unrenderable(owner, field)`, which
  # are ordinary functions that happen to start with the word, and
  # `#[test] fn default_is_development()`, which returns nothing.
  while IFS=: read -r line _; do
    [[ -n "${line:-}" ]] || continue
    report "$f:$line: a zero-argument \`default_*\` constructor — inline the value \
in the struct's \`impl Default\` (scripts/checks/default-style.sh)"
  done < <(grep -nE '^[[:space:]]*(pub(\([^)]*\))?[[:space:]]+)?(const[[:space:]]+)?fn[[:space:]]+default_[a-z0-9_]*[[:space:]]*\(\)[[:space:]]*->' "$f" || true)
done

# (3) single-reader DEFAULT_ constants. A constant earns its name by being
# SHARED; one read is a default value spelled far from its struct.
#
# The reader count is scoped by Rust visibility, which makes it exact: a private
# `const` is unreachable outside its own file, so its readers are countable
# there, while a `pub`/`pub(crate)` one is counted tree-wide. Counting every
# name tree-wide would conflate same-named constants in different modules
# (`events::config::DEFAULT_URL` and `db::DEFAULT_URL` are unrelated).
#
# Declarations come from the SAME collected file set as rules 1-2 (a plain grep
# over each file), so explicit-path invocations — the per-edit hook's shape —
# and untracked files are both inspected; tree-wide reader counts carry
# `--untracked` for the same reason (`git grep` alone sees only tracked
# content).
for f in $files; do
  [[ -f "$f" ]] || continue
  while IFS=: read -r line decl; do
    [[ -n "${line:-}" ]] || continue
    name=$(printf '%s' "$decl" | sed -nE 's/.*const[[:space:]]+(DEFAULT_[A-Z0-9_]+).*/\1/p')
    [[ -n "$name" ]] || continue
    if printf '%s' "$decl" | grep -q 'pub[[:space:]]\|pub(' ; then
      # Visible beyond the file: count tree-wide, discounting every declaration.
      hits=$(git grep --untracked -how "$name" -- '*.rs' | wc -l | tr -d ' ')
      decls=$(git grep --untracked -hcE "const[[:space:]]+$name\b" -- '*.rs' | paste -sd+ - | bc 2>/dev/null || echo 1)
      readers=$((hits - decls))
    else
      readers=$(($(grep -cow "$name" "$f" | tr -d ' ') - 1))
    fi
    if [[ "$readers" -le 1 ]]; then
      report "$f:$line: \`const $name\` has $readers reader(s) — a constant \
earns its name by being shared; inline the value in the \`impl Default\` that \
reads it (scripts/checks/default-style.sh)"
    fi
  done < <(grep -nE 'const[[:space:]]+DEFAULT_[A-Z0-9_]+' "$f" || true)
done

if [[ "$failures" -gt 0 ]]; then
  echo "default-style: $failures violation(s) — see above." >&2
  exit 1
fi
echo "default-style: OK."
