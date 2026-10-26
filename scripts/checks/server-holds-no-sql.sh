#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The wiring binary runs no SQL of its own (#3631).
#
# `app/ferroehr-server` is wiring only, and `ferroehr` is the one crate that
# talks to PostgreSQL. A statement written in the server crate is a storage
# read nobody's storage tests cover, beside the domain rules (the pool, the
# subject rule) the `ferroehr` function would have applied. The server may
# still name a pool's type to hand it on; it may not run a statement on it.
#
# Usage: scripts/checks/server-holds-no-sql.sh [--self-test]
#   no args     → every tracked Rust source under app/ferroehr-server/src/
#   --self-test → prove the detector in both directions
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
cd "$(dirname "$0")/../.."

# Every sqlx entry point that runs a statement, and a SQL statement literal.
readonly BANNED='sqlx::(query|query_as|query_scalar|query_with|query_as_with|query_scalar_with|raw_sql)\b|AssertSqlSafe|"[[:space:]]*(SELECT|INSERT|UPDATE|DELETE|WITH)[[:space:]]'

sources() {
  git ls-files -- 'app/ferroehr-server/src' | grep -E '\.rs$' || true
}

# Report every banned mention in the files named on stdin as `file:line: text`.
findings() {
  local -a files=()
  local f
  while IFS= read -r f; do
    [[ -n "$f" && -f "$f" ]] && files+=("$f")
  done
  [[ "${#files[@]}" -gt 0 ]] || return 0
  grep -HnE "$BANNED" "${files[@]}" 2>/dev/null || true
}

self_test() {
  local tmp
  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand $tmp now, while it is still in scope
  trap "rm -rf '$tmp'" RETURN
  printf 'fn wire(pool: sqlx::PgPool) -> Resolver { resolver(pool) }\n' >"$tmp/clean.rs"
  printf 'let n = sqlx::query_scalar("SELECT 1").fetch_one(&pool).await;\n' >"$tmp/entry.rs"
  printf 'const Q: &str = "SELECT subject_id FROM ehr WHERE id = 7";\n' >"$tmp/literal.rs"

  local found failures=0 expected
  found="$(printf '%s\n' "$tmp"/clean.rs "$tmp"/entry.rs "$tmp"/literal.rs | findings)"
  for expected in entry.rs literal.rs; do
    grep -q "$expected" <<<"$found" || {
      echo "self-test: $expected was not caught" >&2
      failures=1
    }
  done
  if grep -q 'clean.rs' <<<"$found"; then
    echo "self-test: the clean file was wrongly caught" >&2
    failures=1
  fi
  [[ "$failures" -eq 0 ]] || return 1
  echo "server-holds-no-sql: self-test OK (an entry point and a statement caught, a pool type allowed)."
}

report() {
  local found="$1"
  if [[ -z "$found" ]]; then
    echo "server-holds-no-sql: OK (app/ferroehr-server runs no SQL)."
    return 0
  fi
  echo "error: the wiring binary runs SQL of its own" >&2
  echo >&2
  printf '%s\n' "$found" >&2
  echo >&2
  echo "Move the statement behind a ferroehr storage or service function and" >&2
  echo "call that from the wiring." >&2
  return 1
}

case "${1:-}" in
--self-test)
  self_test
  ;;
"")
  report "$(sources | findings)"
  ;;
*)
  guard_usage "[--self-test]"
  ;;
esac
