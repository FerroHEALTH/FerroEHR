#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Access-layer provenance guard (owner directive 2026-08-06, issue #1963).
#
# The authentication/authorization layer is OUR OWN DESIGN, grounded in the IETF
# OAuth2/JOSE RFCs, NIST SP 800-162 (ABAC), ANSI/INCITS 359 (Core RBAC) and the
# OWASP cheat sheets. Its comments must say so — not compare the behaviour to
# another product, and not carry internal phase markers.
#
# Three refusals, each a class the #1963 sweep removed:
#   1. `EHRbase` / `v1 parity` / `v1-compatible` — another product is prior art,
#      never the reason a rule exists. Every rationale cites an RFC, a standards
#      publication, or the vendored openEHR text under docs/specs/openehr/.
#   2. `Stage-1` / `Stage-2` — plan-phase vocabulary that outlived its plan and
#      says nothing to a reader (`.claude/rules/comments.md` bans phase markers).
#   3. a `CLAUDE.md` citation — internal markdown moves and dies, so it is never
#      an authority (the spec-adherence hard rule).
#
# Scope is the access layer plus its configuration, because that is where the
# stale attribution lived and where a rationale is most load-bearing.
#
# Usage: scripts/checks/access-provenance.sh   (no arguments)
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"
cd "$(dirname "$0")/../.."

PATHS=(
  "app/ferroehr-rest/src/extensions/access"
  "app/ferroehr-rest/src/smart"
  "app/ferroehr/src/config/auth.rs"
  "app/ferroehr/src/config/authz.rs"
  "app/ferroehr/src/config/smart.rs"
  "app/ferroehr-rest/examples/policies"
)

failures=0
for pattern in 'EHRbase' 'v1 parity' 'v1-compatible' 'Stage-1' 'Stage-2' 'CLAUDE\.md'; do
  hits=$(grep -rnE "$pattern" "${PATHS[@]}" 2>/dev/null || true)
  if [[ -n "$hits" ]]; then
    printf 'access-provenance: %s must not appear under the access layer:\n' "$pattern" >&2
    printf '%s\n' "$hits" | sed 's/^/    /' >&2
    failures=$((failures + 1))
  fi
done

if [[ "$failures" -gt 0 ]]; then
  printf 'access-provenance: %d stale-attribution class(es) — cite an RFC, a \n' "$failures" >&2
  printf '    standards publication, or docs/specs/openehr/ instead.\n' >&2
  exit 1
fi
echo "access-provenance: OK."
