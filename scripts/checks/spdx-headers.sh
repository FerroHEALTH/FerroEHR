#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Every first-party Rust source file states its licensing INSIDE itself.
#
# `REUSE.toml` already covers the whole tree by glob and `reuse lint` proves it
# complete, but a glob declaration is exactly the thing that does NOT travel
# with a file. REUSE 3.3 exists for that property — licensing "is preserved when
# the file is copied and reused by third parties" — and only a header in the
# file delivers it. This gate is what keeps the headers from rotting away one
# new file at a time; `reuse lint` cannot see the difference, because the glob
# covers a headerless file just as well.
#
# SCOPE: tracked `.rs` files. Vendored trees are excluded, not exempted — a
# header there would assert something about somebody else's file, and
# hand-editing vendored material is forbidden. Other first-party file types
# (shell, SQL, YAML) are outside this gate.
#
# THE EXPECTED HEADER is derived from `REUSE.toml`, so the two can never
# disagree: every published spec crate is Apache-2.0 (the licence of the openEHR
# inputs it is generated from; the six that embed specification text also name
# the openEHR Foundation as a holder), and everything else is BUSL-1.1. A
# generation-twin template under `tools/openehr-codegen/templates/<crate>/`
# follows the crate it is stamped INTO, because its content is that crate's
# content.
#
# GENERATED FILES get their header from the emitter (`openehr-codegen`), never
# from a hand edit or from `--fix` here: a hand edit is silently overwritten by
# the next `emit` and fails `codegen-drift`.
#
# Usage:
#   scripts/checks/spdx-headers.sh          # verify (exit 1 on any violation)
#   scripts/checks/spdx-headers.sh --fix    # insert missing headers (hand-written only)
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_known_flags "[--fix]" "--fix" "$@"

cd "$(dirname "$0")/../.."

# The expected tags below are DATA this gate compares against, not this file's
# own licensing, and `reuse lint` reads a tag wherever it appears — the
# specification's own remedy for a file that quotes the syntax it checks. This
# file's licensing comes from REUSE.toml, like every other script here.
# REUSE-IgnoreStart
readonly PROJECT_COPYRIGHT='// SPDX-FileCopyrightText: Cadasto B.V.'
readonly OPENEHR_COPYRIGHT='// SPDX-FileCopyrightText: openEHR Foundation'
readonly BUSL_HEADER="$PROJECT_COPYRIGHT
// SPDX-License-Identifier: BUSL-1.1"
# The five generated model crates and the openehr-its wire layer: the
# project's code and the openEHR-derived material it carries, both Apache-2.0
# (owner decisions 2026-09-03 and 2026-09-24). The three hand-written engines,
# openehr-adl, openehr-query and openehr-sdt, embed nothing and carry the plain
# BUSL header like the application.
readonly DUAL_HEADER="$PROJECT_COPYRIGHT
$OPENEHR_COPYRIGHT
// SPDX-License-Identifier: Apache-2.0"
readonly -a DUAL_CRATES=(
  openehr-am
  openehr-its
  openehr-base
  openehr-lang
  openehr-rm
  openehr-term
)

# How far into a file an SPDX tag still counts as the header: a generated
# banner, up to two copyright lines and the identifier.
readonly HEAD_LINES=8

mode="${1:-}"
case "$mode" in
'' | --fix) ;;
*)
  echo "usage: $0 [--fix]" >&2
  exit 2
  ;;
esac

# The crate whose licensing a path follows — its own for `crates/<name>/`, the
# destination crate for a generation-twin template. Empty for anything else.
licensing_crate() {
  case "$1" in
  crates/*)
    local rest=${1#crates/}
    printf '%s' "${rest%%/*}"
    ;;
  tools/openehr-codegen/templates/*)
    local rest=${1#tools/openehr-codegen/templates/}
    printf '%s' "${rest%%/*}"
    ;;
  *) printf '' ;;
  esac
}

expected_header() {
  local krate
  krate=$(licensing_crate "$1")
  for dual in "${DUAL_CRATES[@]}"; do
    if [[ "$krate" = "$dual" ]]; then
      printf '%s' "$DUAL_HEADER"
      return
    fi
  done
  printf '%s' "$BUSL_HEADER"
}

is_generated() {
  head -n 1 "$1" | grep -q '@generated'
}

fail=0
fixed=0
checked=0
while IFS= read -r f; do
  [[ -f "$f" ]] || continue
  checked=$((checked + 1))
  expected=$(expected_header "$f")
  found=$(head -n "$HEAD_LINES" "$f" | grep -E '^// SPDX-(FileCopyrightText|License-Identifier): ' || true)

  if [[ "$found" = "$expected" ]]; then
    continue
  fi

  if [[ -z "$found" ]] && [[ "$mode" = --fix ]] && ! is_generated "$f"; then
    printf '%s\n\n' "$expected" | cat - "$f" >"$f.spdx.tmp"
    mv "$f.spdx.tmp" "$f"
    fixed=$((fixed + 1))
    continue
  fi

  fail=1
  if [[ -z "$found" ]]; then
    echo "$f: no SPDX header in the first $HEAD_LINES lines" >&2
  else
    echo "$f: SPDX header does not match the licensing REUSE.toml declares" >&2
    echo "  expected: ${expected//$'\n'/ | }" >&2
    echo "  found:    ${found//$'\n'/ | }" >&2
  fi
  if is_generated "$f"; then
    echo "  (generated — fix the openehr-codegen emitter and regenerate)" >&2
  fi
done < <(git ls-files '*.rs')

if [[ "$fixed" -ne 0 ]]; then
  echo "spdx-headers: inserted $fixed header(s)."
fi
if [[ "$fail" -ne 0 ]]; then
  echo >&2
  echo "Licensing must travel with a file that is copied out of this tree" >&2
  echo "(REUSE 3.3). Hand-written files take the header directly; generated" >&2
  echo "ones take it from openehr-codegen." >&2
  exit 1
fi
echo "spdx-headers: OK ($checked files)."

# REUSE-IgnoreEnd
