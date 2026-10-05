#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Changelog structure guard (Keep a Changelog 1.1.0).
#
# Fails when any release section of CHANGELOG.md (including [Unreleased])
# violates the Keep a Changelog structure:
#   1. a duplicated `### <Type>` subsection inside one release section
#      (each type appears at most once per section — new entries merge into
#      the existing subsection, they never append a second header);
#   2. a subsection header outside the canonical type set
#      (Added / Changed / Deprecated / Removed / Fixed / Security).
#
# Wired into the CI changelog-guard job; runs unconditionally (the
# `no-changelog` escape label waives the entry REQUIREMENT, never the
# structural validity of the file).
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_known_flags "[<file>]" "" "$@"

file="${1:-CHANGELOG.md}"

# awk, not python: this repository ships no Python, and the check is a line scan
# over one file — exactly what awk is for. The quote character arrives as a
# variable because an awk string constant cannot portably escape it.
awk -v path="$file" -v q="'" '
  # A release heading opens a new section and resets what has been seen in it.
  /^## \[/ {
    section = $0
    sub(/^## \[/, "", section)
    sub(/\].*$/, "", section)
    delete seen
    next
  }
  # A subsection heading inside a section: check the type, then the duplicate.
  /^### / && section != "" {
    heading = $0
    sub(/^### /, "", heading)
    sub(/[[:space:]]+$/, "", heading)
    if (heading != "Added" && heading != "Changed" && heading != "Deprecated" \
        && heading != "Removed" && heading != "Fixed" && heading != "Security") {
      errors[++n] = path ":" FNR ": " q "### " heading q " in [" section \
        "] is not a Keep-a-Changelog type (allowed: Added, Changed, " \
        "Deprecated, Fixed, Removed, Security)"
    }
    if (heading in seen) {
      errors[++n] = path ":" FNR ": duplicate " q "### " heading q " in [" \
        section "] — merge the entry into the existing subsection instead of " \
        "adding a second header"
    }
    seen[heading] = 1
  }
  END {
    if (n > 0) {
      print "changelog structure check FAILED:" > "/dev/stderr"
      for (i = 1; i <= n; i++) print "  " errors[i] > "/dev/stderr"
      exit 1
    }
    print "changelog structure OK (" path ")"
  }
' "$file"
