#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# RFC 0201: an error carries its cause (#2034).
#
# `Error::source` is part of the `Error` contract, and `map_err(|e| V(e.to_string()))`
# destroys it at exactly the seam where it mattered. A stringified cause cannot
# be matched on, walked, or classified — the transport layer maps an error to a
# status code BY TYPE, so a `PoolTimedOut` that arrived as a string cannot become
# a 503 rather than a 500 without parsing prose.
#
# No clippy lint covers this (checked against the pinned 1.96.1 lint set), so
# the check is grep-shaped, and it is honest about that: it catches the common
# spelling, not every possible one. Its value is stopping the count from growing
# while the sweep runs.
#
# THE ALLOWLIST IS THE HONEST PART. Three shapes legitimately flatten:
#
#   1. a published `openehr-*` crate must not leak a private dependency's error
#      type into its own public API — that would make a patch bump of that
#      dependency a breaking change for us (RFC 1105);
#   2. a cause that is genuinely a MESSAGE rather than an error (a parser's
#      positional diagnostic that already IS the human-readable answer);
#   3. test code.
#
# An entry added without one of those reasons makes this gate decoration.
#
# Usage: scripts/checks/error-source-chain.sh   (no arguments)
#
# The whole tree is always counted — there is no narrower scope to ask for, so
# there is no flag to pass.
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"
cd "$(dirname "$0")/../.." || exit 1

# The sweep (#2034) is CLOSED: every remaining site below was judged and carries
# its `// NOTE:` reason at the site or on its error type. Each budget is a COUNT
# that may only go DOWN — a new flattened site fails the gate, and lowering a
# budget needs the same judgement the sweep applied.
#
# What survives, and why each one is structural rather than unfinished:
#
#   app/ferroehr           `prometheus::Error` has no source-bearing variant.
#   app/ferroehr-ext       `fhir_model`'s builder error is a dependency type
#                          (RFC 1105 — see the site's NOTE).
#   app/ferroehr-viewer  `ViewerError` crosses the server-fn boundary, so
#                          `FromServerFnError` requires Serialize/Deserialize;
#                          `LiftError` is held in an `RwSignal`, so it must be
#                          Clone + Eq. No underlying error is any of those.
#
# A line that ALSO calls `.with_source(e)` is not flattened — it carries the
# cause and stringifies only for the message — so the count excludes it.
#
# `tools/*` is NOT swept, and that is an adjudication rather than an omission —
# the tree falls under the shapes #2034 itself names as legitimate:
#   tools/openehr-codegen   every site is in testsupport.rs — test code, which
#                           #2034 exempts outright.
#
# Both stay out of the budget list so the gate measures the request path, where
# a status code is chosen BY TYPE and a flattened cause actually costs something.
declare -a BUDGETS=(
  "app/ferroehr:1"
  "app/ferroehr-ext:1"
  "app/ferroehr-viewer:6"
)

fail=0
for entry in "${BUDGETS[@]}"; do
  tree="${entry%%:*}"
  budget="${entry##*:}"
  count="$(grep -rn 'map_err(|e| .*to_string())' "$tree" 2>/dev/null \
           | grep -v '/tests/' | grep -vc 'with_source' || true)"
  count="${count:-0}"
  printf '%-26s %3s flattened (budget %s)\n' "$tree" "$count" "$budget"
  if [[ "$count" -gt "$budget" ]]; then
    echo "::error::$tree grew from $budget to $count flattened error sites — an error carries its cause (RFC 0201, #2034)"
    fail=1
  fi
done

[[ "$fail" -eq 0 ]] || exit 1
echo "error-source-chain: no tree grew"
