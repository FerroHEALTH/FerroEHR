#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The configuration keys FerroEHR reads, as dotted TOML paths, from the
# reference file app/ferroehr/assets/ferroehr.default.toml, which
# scripts/checks/config-reference.sh holds to the JSON Schema of the
# configuration. A key the reference does not carry is one the server refuses
# at start (deny_unknown_fields throughout). No specification governs this:
# our own design.
#
# Source it as:
#
#   # shellcheck source=scripts/lib/config-keys.sh
#   . "$(dirname "$0")/../lib/config-keys.sh"

# config_keys FILE: every TOML path of FILE, plus every ancestor table (a
# page may name `[audit]` alone), sorted and once each. A `#?` line documents
# a real optional, secret or derived key and counts; a plain `#` comment does
# not, except under a `# [table.<name>]` header, a table under an
# operator-chosen name, whose keys are written as `# key = …` and count with
# `*` in the name's place.
config_keys() {
  local file="$1"
  awk '
    { line = $0
      sub(/^[[:space:]]*/, "", line)
      if (line ~ /^# \[[A-Za-z0-9_.]+\.<name>(\.[A-Za-z0-9_.]+)?\]$/) {
        sec = line; gsub(/^# \[|\]$/, "", sec); gsub(/<name>/, "*", sec); named = 1; next
      }
      if (named && line ~ /^# [A-Za-z0-9_]+ = /) sub(/^# /, "", line)
      else if (line ~ /^#\?[[:space:]]*/) { sub(/^#\?[[:space:]]*/, "", line); named = 0 }
      else if (line ~ /^#/) next
      else named = 0
      if (line ~ /^\[\[?[A-Za-z0-9_.]+\]\]?/) { sec = line; gsub(/^\[\[?|\]\]?.*$/, "", sec); next }
      if (line ~ /^[A-Za-z0-9_]+[[:space:]]*=/) {
        k = line; sub(/[[:space:]]*=.*$/, "", k)
        path = (sec == "" ? k : sec "." k)
        print path
        n = split(path, seg, ".")
        acc = seg[1]
        for (i = 2; i < n; i++) { print acc; acc = acc "." seg[i] }
        if (n > 1) print acc
      }
    }' "$file" | sort -u
}
