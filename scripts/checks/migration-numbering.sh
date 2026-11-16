#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Migration-numbering guard: the migrations of one domain are numbered 0001 to
# N, each number once (#5741).
#
# sqlx orders a domain's migrations by the number that opens each file name
# and records each applied one by that number in `<schema>._sqlx_migrations`.
# Two pull requests that each add the next number pass on their own and land
# two files with one number (#5711 and #5734 both added audit 0039), and a
# migration-count test does not notice, since each branch counted its own.
# A gap is refused too, so `_sqlx_migrations` reads as the set's table of
# contents and the next number is never ambiguous. No openEHR spec governs
# migration numbering: our own design.
#
# Scope: every `app/ferroehr/migrations/<domain>/` directory that holds
# `NNNN_<name>.sql` files; `templates/` holds no migration and is skipped.
#
# Usage: scripts/checks/migration-numbering.sh [--self-test | <migrations dir>]
#   no args     → app/ferroehr/migrations of this tree
#   --self-test → runs the guard over fixtures with a shared number, a gap and
#                 a clean set, and asserts the first two are refused and the
#                 third accepted.
set -euo pipefail
export LC_ALL=C

# The domains under `root` that break the numbering, one line per finding.
findings() {
  local root="$1" dir domain expected number previous path file shared
  for dir in "$root"/*/; do
    domain="$(basename "$dir")"
    [[ "$domain" == templates ]] && continue
    expected=1
    previous=""
    # NOTE: a glob expands in the C collation's order here, which is the order sqlx applies.
    for path in "$dir"[0-9][0-9][0-9][0-9]_*.sql; do
      [[ -e "$path" ]] || continue
      file="$(basename "$path")"
      number="${file%%_*}"
      if [[ "$number" == "$previous" ]]; then
        shared=("$dir$number"_*.sql)
        echo "${domain}: two migrations share ${number}: ${shared[*]##*/}"
        continue
      fi
      if ((10#$number != expected)); then
        printf '%s: %s follows %04d; the numbers run from 0001 with no gap\n' \
          "$domain" "$file" "$((expected - 1))"
      fi
      previous="$number"
      expected=$((10#$number + 1))
    done
  done
}

if [[ "${1:-}" == "--self-test" ]]; then
  self_root="$(mktemp -d)"
  trap 'rm -rf "$self_root"' EXIT
  self_fail=0
  # <label>|<files of one domain, space-separated>|<expected exit: 0 accept, 1 reject>
  self_cases=(
    "two migrations sharing a number|0001_a.sql 0002_b.sql 0002_c.sql|1"
    "a gap in the numbering|0001_a.sql 0003_c.sql|1"
    "a set that does not start at 0001|0002_b.sql|1"
    "a clean set|0001_a.sql 0002_b.sql 0003_c.sql|0"
  )
  for self_case in "${self_cases[@]}"; do
    label="${self_case%%|*}"
    rest="${self_case#*|}"
    files="${rest%%|*}"
    want="${rest#*|}"
    rm -rf "$self_root/migrations"
    mkdir -p "$self_root/migrations/audit" "$self_root/migrations/templates"
    touch "$self_root/migrations/templates/node.sql.in"
    for file in $files; do
      touch "$self_root/migrations/audit/$file"
    done
    got=0
    bash "$0" "$self_root/migrations" >/dev/null 2>&1 || got=1
    if [[ "$got" != "$want" ]]; then
      echo "::error::--self-test: ${label}: expected exit ${want}, got ${got}." >&2
      self_fail=1
    else
      echo "  ok: ${label}"
    fi
  done
  exit "$self_fail"
fi

root="${1:-$(cd "$(dirname "$0")/../.." && pwd)/app/ferroehr/migrations}"
found="$(findings "$root")"
if [[ -n "$found" ]]; then
  while IFS= read -r line; do
    echo "::error::migration-numbering: ${line}" >&2
  done <<<"$found"
  echo "Renumber the newer migration to the next free number of its domain and bump the count in app/ferroehr/tests/it/persistence.rs." >&2
  exit 1
fi
echo "migration-numbering: every domain is numbered from 0001 with no gap and no number twice"
