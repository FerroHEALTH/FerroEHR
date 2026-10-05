#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Every openEHR spec file a comment cites must exist under docs/specs/openehr/.
#
# The vendored spec text is the oracle (.claude/rules/spec-adherence.md), and a
# citation is how a reader checks that a behaviour is grounded. A citation that
# names no real file cannot be checked by anyone, so it is worth exactly as much
# as no citation at all — and it reads as authority while providing none. The
# 2026-08-11 template audit found eighteen of them, including a `master04-` that
# should have been `master06-` and two class paths carrying a sub-package
# segment the filenames do not have.
#
# What this does NOT check: whether a citation's SECTION exists, and whether
# quoted text is real. Both are review-enforced. The audit that motivated this
# also found a quotation ("include, but are not limited to") that appears
# nowhere in the vendored specs while naming a file that does exist — this guard
# would not have caught it, and pretending otherwise would be its own defect.
#
# Usage: spec-citations.sh [--all | <file>...]
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_known_flags "[--all | <file>...]" "--all" "$@"

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$repo_root"

specs_dir="docs/specs/openehr"
if [[ ! -d $specs_dir ]]; then
  echo "spec-citations: $specs_dir not found — nothing to check against" >&2
  exit 0
fi

# Every real spec filename, once.
real=$(mktemp)
trap 'rm -f "$real"' EXIT
find "$specs_dir" -name '*.adoc' -exec basename {} \; | sort -u >"$real"

files=()
if [[ ${1:-} == "--all" ]]; then
  while IFS= read -r f; do files+=("$f"); done < <(
    git ls-files '*.rs' '*.sql' | grep -v '^docs/specs/'
  )
else
  files=("$@")
fi
[[ ${#files[@]} -eq 0 ]] && exit 0

violations=0
for file in "${files[@]}"; do
  [[ -f $file ]] || continue
  # Only comments cite specs; a citation inside a string literal is data.
  while IFS=: read -r line_no cited; do
    [[ -n $cited ]] || continue
    # An `include::` line is quoting an asciidoc DIRECTIVE, not citing a file.
    grep -qF "include::" <<<"$(sed -n "${line_no}p" "$file")" && continue
    # A wrapped doc line may start the name with `...`; the convention also
    # allows citing a class file by a suffix of its dotted path.
    bare=${cited#...}
    # `x.adoc` / `y.yaml` in prose describing the SHAPE of a citation are
    # placeholders, not citations — no vendored spec file has a one-letter stem.
    [[ ${#bare} -le 6 ]] && continue
    if ! grep -qE "(^|\.)$(printf '%s' "$bare" | sed 's/[.[\*^$]/\\&/g')$" "$real"; then
      printf '%s:%s: cites %s, which is not a file under %s/\n' \
        "$file" "$line_no" "$cited" "$specs_dir"
      violations=$((violations + 1))
    fi
  done < <(grep -nE '^\s*(//|--)' "$file" 2>/dev/null |
    grep -oE '^[0-9]+:|[A-Za-z0-9._-]+\.adoc' |
    awk '/^[0-9]+:$/ { n = $0; next } { print n $0 }' |
    sort -u)
done

if [[ $violations -gt 0 ]]; then
  echo
  echo "spec-citations: $violations citation(s) name no vendored spec file" >&2
  echo "(rules: .claude/rules/spec-adherence.md — cite the vendored specs or official external docs)" >&2
  exit 1
fi

echo "spec-citations: OK (${#files[@]} files)."
