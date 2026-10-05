#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The one refusal path every guard under scripts/checks/ shares.
#
# The guards are run by hand constantly and their flag vocabularies genuinely
# differ (`--all`, `--diff <base> [head]`, `--files`, `--fix`, `--self-test`,
# `--schema PATH`, named files, nothing at all). What must NOT differ is what
# happens to a flag a guard does not know: it is refused by name, before any
# checking runs, so a wrong command line can never be read as a finding.
#
# Deliberately two functions and no parser. Modelling six grammars in one
# place would either force a false uniform vocabulary onto the guards or grow
# into a configuration language; the duplication this removes is the refusal
# and its message shape, which is the part that was inconsistent.
#
# Source it from a guard as:
#
#   # shellcheck source=scripts/lib/guard-args.sh
#   . "$(dirname "$0")/../lib/guard-args.sh"
#
# Exit code 2 is the usage code every guard here already used, distinct from
# 1 (findings) and 0 (clean), so a caller can tell the three apart.

# Refuse the command line: print `usage: <script> <grammar>` and exit 2.
# $1 = the accepted grammar, e.g. "[--all | --diff <base> [head]]".
guard_usage() {
  echo "usage: $0 $1" >&2
  exit 2
}

# Refuse any argument at all, for a guard whose scope is fixed. Call it as
# `guard_no_args "$@"` before the guard does any work.
guard_no_args() {
  [[ "$#" -eq 0 ]] || {
    echo "error: $0 takes no arguments (got: $*)" >&2
    guard_usage "(no arguments — this guard's scope is fixed)"
  }
}

# Refuse any dash-led argument outside the guard's own vocabulary, for a guard
# that also takes bare file paths. A wrong flag is then a usage error rather
# than a filename the guard goes looking for.
# $1 = the accepted grammar, $2 = the accepted flags (space-separated),
# then "$@" of the caller.
guard_known_flags() {
  local grammar="$1" accepted=" $2 " arg
  shift 2
  for arg in "$@"; do
    case "$arg" in
    -*)
      [[ "$accepted" == *" $arg "* ]] || {
        echo "error: $0: unknown flag $arg" >&2
        guard_usage "$grammar"
      }
      ;;
    *) ;;
    esac
  done
}
