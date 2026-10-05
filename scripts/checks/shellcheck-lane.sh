#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# ShellCheck over every shell program this repository WROTE.
#
# The tooling languages here are bash and Rust (.claude/rules/rust-style.md
# §No Python), and the Rust half has been lint-gated since the beginning while
# the bash half — the CI guards, the vendor fetchers, the deploy probes, the
# release machinery, the git hooks — had no static check at all. actionlint
# already shellchecks the `run:` blocks EMBEDDED in workflows, which is the
# smaller half: the scripts those blocks call were unchecked.
#
# Severity: `-S style`, the lowest floor ShellCheck offers, so everything
# gates. A finding is either fixed or carries a per-line
# `# shellcheck disable=SCnnnn` directive with its reason on the same line —
# never a blanket exclusion, and never a global disable (this repository has no
# .shellcheckrc for exactly that reason: a file that can turn a code off
# everywhere is a file that eventually does).
#
# Discovery is derived, not listed: every tracked `*.sh` plus every tracked
# extensionless file whose first line is a shell shebang. The second half is
# what covers `.githooks/commit-msg` — the one such program today — and covers
# the next hook the day it lands rather than the day someone remembers.
#
# Usage: scripts/checks/shellcheck-lane.sh [--all | <file>...]
#   no args  → every tracked shell program outside the vendored trees
#   --all    → the same scope, said explicitly
#   <file>…  → just those files, at the same severity
#
# An unrecognised flag is a usage error (exit 2), refused by name before any
# checking runs: forwarding it to shellcheck reported a wrong command line as a
# lint finding, with this lane's own findings epilogue under it.
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_known_flags "[--all | <file>...]" "--all" "$@"

cd "$(dirname "$0")/../.."

# Vendored trees are upstream material, vendored verbatim by a
# scripts/vendor/*.sh fetcher and never hand-edited
# (.claude/rules/vendored-corpora.md) — a finding in one of them is not ours to
# fix, and editing it to silence this lane is forbidden by that rule. No
# vendored tree carries a shell program today; the exclusion is here so the
# first one that arrives cannot turn this lane red for something unfixable.
readonly VENDORED='^(docs/specs/|corpus/|website/book/theme/|(crates|tools)/[^/]+/vendor/)'

# A shell shebang, direct or through env: bash, sh, dash, ksh, zsh.
shell_shebang() {
  head -n 1 "$1" 2>/dev/null | grep -Eq '^#!.*[/ ](bash|sh|dash|ksh|zsh)([[:space:]]|$)'
}

collect() {
  git ls-files '*.sh' | grep -Ev "$VENDORED"
  # Extensionless tracked files (no dot in the last path segment), filtered
  # down to the ones that are actually shell.
  while read -r f; do
    [[ -f "$f" ]] || continue
    if shell_shebang "$f"; then printf '%s\n' "$f"; fi
  done < <(git ls-files | grep -Ev "$VENDORED" | grep -Ev '\.[^/]+$')
}

if [[ "$#" -gt 0 ]]; then
  files=$(printf '%s\n' "$@")
else
  files=$(collect | sort -u)
fi

[[ -n "$files" ]] || { echo "shellcheck-lane: no shell programs to check."; exit 0; }

command -v shellcheck >/dev/null 2>&1 || {
  echo "error: shellcheck is not installed (https://www.shellcheck.net/)" >&2
  exit 1
}

count=$(printf '%s\n' "$files" | wc -l | tr -d ' ')
# `--external-sources` lets a `# shellcheck source=…` directive be FOLLOWED
# rather than reported as SC1091: the guards source scripts/lib/*.sh, and a
# sourced file that is merely announced teaches the checker nothing about the
# functions it defines. Paths in those directives are repo-root-relative, which
# is where this lane runs shellcheck from.
if ! printf '%s\n' "$files" \
  | xargs shellcheck --external-sources --severity=style --format=gcc; then
  echo >&2
  echo "shellcheck-lane: findings above. Fix each one, or — where the flagged" >&2
  echo "form is deliberate — add a '# shellcheck disable=SCnnnn' directive on" >&2
  echo "the line before the command, with the reason as a trailing comment." >&2
  exit 1
fi

echo "shellcheck-lane: OK ($count shell programs, severity=style)."
