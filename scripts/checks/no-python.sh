#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# No Python in this repository (owner hard rule, 2026-08-10).
#
# The tooling languages are bash and Rust. Python is banned everywhere —
# standalone scripts, and especially embedded in shell, where it is worse than
# either language alone: a heredoc nested in a command substitution makes bash
# scan the Python for quote pairs, so ONE apostrophe in a comment breaks the
# whole script. That happened while writing scripts/render/zenodo-json.sh, and
# it is the concrete reason this rule is machine-enforced rather than advisory.
#
# There are no exemptions. The last two — the Helm selector-immutability and
# restricted-profile gates — parsed multi-document YAML and were converted to
# yq + jq under #2220, every assertion re-proven against the same mutations it
# caught before.
#
# Usage: scripts/checks/no-python.sh   (no arguments)
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"
cd "$(dirname "$0")/../.." || exit 1

# No file is exempt. If one ever has to be, it goes here WITH its tracking
# issue — never as a quiet omission from ROOTS below.
EXEMPT='^$'

# Where shell and CI live. Deliberately not the whole tree: a Python file
# inside a vendored corpus is upstream material, not ours to rewrite.
ROOTS=(scripts .github/workflows .claude/hooks deploy docker)

hits=0
while IFS= read -r file; do
  [[ -f "$file" ]] || continue
  case "$file" in
    */node_modules/*|*/vendor/*|*/target/*) continue ;;
    *) ;;
  esac
  printf '%s' "$file" | grep -qE "$EXEMPT" && continue
  # An invocation, not the word: prose about not using Python is fine and this
  # repository contains several such comments. pipx/pip are Python by another
  # door — `pipx run cffconvert` ran a Python tool for months with no `python`
  # token for this guard to see (#2791).
  if matches="$(grep -nE '(^|[^[:alnum:]_./-])(python3?|pipx|pip3?)([[:space:]]|$)' "$file" \
                | grep -vE '#.*(python|pipx|pip)' || true)"; then
    if [[ -n "$matches" ]]; then
      printf '%s:\n%s\n' "$file" "$matches"
      hits=$((hits + 1))
    fi
  fi
done < <(find "${ROOTS[@]}" -type f \( -name '*.sh' -o -name '*.yml' -o -name '*.yaml' -o -name '*.py' \) 2>/dev/null | sort)

# A standalone Python file is a violation regardless of content.
while IFS= read -r py; do
  printf '%s: a Python source file\n' "$py"
  hits=$((hits + 1))
done < <(find "${ROOTS[@]}" -type f -name '*.py' 2>/dev/null | sort)

if [[ "$hits" -gt 0 ]]; then
  echo
  echo "::error::Python is banned in this repository (see .claude/rules/rust-style.md)."
  echo "  Use bash + jq/awk/sed, or Rust. The two Helm gates are the only"
  echo "  exemptions and are tracked by #2220."
  exit 1
fi
echo "no-python: clean"
