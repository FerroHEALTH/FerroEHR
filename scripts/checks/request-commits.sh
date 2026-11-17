#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# A request-scoped write commits only through the commit choke point (#5762).
#
# `FerroEhrService::commit_recorded` (app/ferroehr/src/system_log/
# reservation.rs) writes a write's audit records, held under a reservation,
# before the write commits, and the reservation joins the commit; a write that
# commits any other way can commit with no record of its access. Every other
# `.commit()` of a transaction in app/ferroehr/src is a commit outside a
# request (a boot fill, a background sweep, a read-only snapshot, the audit
# domain's own store) and says so in a `// NOTE:` naming #5762 in the three
# lines above it. No openEHR spec governs the audit trail: our own design.
#
# Usage: scripts/checks/request-commits.sh [--self-test]
#   no args     → every tracked Rust source under app/ferroehr/src/
#   --self-test → prove the detector in both directions
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
cd "$(dirname "$0")/../.."

# The one file the choke point lives in.
readonly CHOKE_POINT='app/ferroehr/src/system_log/reservation.rs'

sources() {
  git ls-files -- 'app/ferroehr/src' | grep -E '\.rs$' | grep -vxF "$CHOKE_POINT" || true
}

# Report every `.commit()` in the files named on stdin with no `(#5762)` in
# the three lines above it, as `file:line: text`.
findings() {
  local f
  while IFS= read -r f; do
    [[ -n "$f" && -f "$f" ]] || continue
    awk -v file="$f" '
      { window[NR % 4] = $0 }
      /\.commit\(\)([[:space:]]*\.await|[[:space:]]*$)/ {
        held = 0
        for (back = 1; back <= 3; back++) {
          if (NR - back >= 1 && window[(NR - back) % 4] ~ /\(#5762\)/) held = 1
        }
        if (!held) printf "%s:%d: %s\n", file, NR, $0
      }
    ' "$f"
  done
}

self_test() {
  local tmp
  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand $tmp now, while it is still in scope
  trap "rm -rf '$tmp'" RETURN
  printf 'self.commit_recorded(tx, domain, &facts, records).await?;\n' >"$tmp/choke.rs"
  printf '// NOTE: our own design (#5762), a boot fill outside any request.\ntx.commit().await?;\n' >"$tmp/marked.rs"
  printf 'let x = 1;\ntx.commit().await?;\nlet sha = build.commit().unwrap_or("");\n' >"$tmp/bare.rs"
  printf '// NOTE: our own design (#5762), far away.\n\n\n\ntx\n    .commit().await?;\n' >"$tmp/distant.rs"

  local found failures=0 expected
  found="$(printf '%s\n' "$tmp"/choke.rs "$tmp"/marked.rs "$tmp"/bare.rs "$tmp"/distant.rs | findings)"
  for expected in bare.rs distant.rs; do
    grep -q "$expected" <<<"$found" || {
      echo "self-test: $expected was not caught" >&2
      failures=1
    }
  done
  for expected in choke.rs marked.rs; do
    if grep -q "$expected" <<<"$found"; then
      echo "self-test: $expected was wrongly caught" >&2
      failures=1
    fi
  done
  [[ "$failures" -eq 0 ]] || return 1
  echo "request-commits: self-test OK (a bare and a distant commit caught, the choke point and a marked commit allowed)."
}

report() {
  local found="$1"
  if [[ -z "$found" ]]; then
    echo "request-commits: OK (every commit outside the choke point says why it is no request's)."
    return 0
  fi
  echo "error: a transaction commits outside the commit choke point" >&2
  echo >&2
  printf '%s\n' "$found" >&2
  echo >&2
  echo "Commit a request-scoped write through FerroEhrService::commit_recorded, or," >&2
  echo "for a commit no request makes, say why in a // NOTE: naming #5762 above it." >&2
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
