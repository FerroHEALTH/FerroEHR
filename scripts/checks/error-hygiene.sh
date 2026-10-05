#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Error-body hygiene guard (OWASP REST Security Cheat Sheet, issue #2021):
# "Do not pass technical details (e.g. call stacks or other internal hints) to
# the client."
#
# The cheat sheet's first sentence — "respond with generic error messages" — is
# deliberately NOT followed here, and this guard encodes the distinction that
# replaces it.
#
# A 4xx describes the CLIENT's request, and specificity there is a feature: the
# strict canonical reader names the JSON path it refused, the AQL engine names
# the construct it cannot plan, the AOM2 validator returns catalogue codes.
# Integrators building against a standard this large need that, and none of it
# is internal information — it is a restatement of what the caller sent.
#
# A 5xx describes the SERVER, and specificity there is a leak: an `sqlx::Error`
# reaching a body would disclose schema and SQL, a `reqwest::Error` would
# disclose an internal terminology-server URL, and a formatted `thiserror`
# chain discloses whatever its innermost source knows. Those details belong on
# the trace record, where the operator reads them, and nowhere else.
#
# So the rule is one-directional: a 5xx constructor must not interpolate an
# error value. The tree satisfies this today (the storage boundary substitutes a
# fixed message per SQLSTATE class and logs the driver error; the terminology
# client's `provider_fault` keeps the upstream diagnostic off the wire), and
# this guard is what keeps it true as new error paths are added.
#
# What it CANNOT see, stated so nobody reads a pass as a proof: it is a textual
# scan of a three-line window around a constructor, so an error value that is
# stringified into a differently-named local first (`let detail =
# e.to_string();`), interpolated more than two lines below the constructor, or
# formatted inside a helper the constructor merely calls, passes it.
#
# Usage: scripts/checks/error-hygiene.sh   (no arguments)
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"
cd "$(dirname "$0")/../.."

# The guarded constructors — every spelling that renders a free-form string into
# a 5xx body:
#
#   - `ApiError::Internal` / `ApiError::ServiceUnavailable` — the 5xx bodies.
#     (`ValidationFailed` and `NotImplemented` carry no free-form string.)
#   - `Health::down` / `Health::degraded` — the health family is ALWAYS-ON and
#     UNAUTHENTICATED for orchestrator probes, so its `detail` is the most
#     exposed string the server emits. Two indicators used to interpolate the
#     raw `sqlx::Error` there, whose Display can name the DSN host, the
#     database, the role or SQL text (found and fixed under issue #2022).
#   - `SmError::exception` and `SmError::new(CallStatusType::Exception, …)` —
#     the SECOND route to a 500 body (issue #2125). A service chapter method
#     returns `Result<_, SmError>`, and `sm_api_error`
#     (app/ferroehr-rest/src/overview/error.rs) renders the `exception`,
#     `success` and `file_not_writable` statuses as `ApiError::Internal(message)`
#     — so that `message` IS a 500 body, with no curation in between. The
#     multi-line spelling is caught by matching the status token on its own
#     line, which is where `SmError::new(` wraps.
#
# NOT guarded, deliberately: `ServiceError::exception` /
# `ServiceError::Internal`. Both `From<ServiceError>` bridges
# (app/ferroehr/src/service/error.rs) discard that message and substitute
# `INTERNAL_MESSAGE`, tracing the detail instead — the message is a log detail
# by construction and never reaches a client.
CONSTRUCTORS='ApiError::(Internal|ServiceUnavailable)\(|Health::(down|degraded)\(|SmError::exception\(|SmError::new\([[:space:]]*CallStatusType::Exception|^[[:space:]]*CallStatusType::Exception[,[:space:]]*$'
# The shapes that put an error's Display into a string: a `{e}`-style
# interpolation of a conventionally-named error binding, or an explicit
# `.to_string()` on one.
ERROR_VALUE='\{(e|err|error|source|cause)(:[^}]*)?\}|(\b(e|err|error|source|cause))\.to_string\(\)'

failures=0
# A constructor call plus the two lines after it, so a wrapped `format!` is seen.
# Comment lines are dropped first: this guard's own rule is documented in prose
# above several of the sites it scans.
hits=$(grep -rn -A 2 -E "${CONSTRUCTORS}" app/ferroehr-rest/src app/ferroehr/src crates/openehr-its/src 2>/dev/null \
  | grep -vE '^[^:]+[:-][0-9]*[:-]?[[:space:]]*//' \
  | grep -E "$ERROR_VALUE" || true)
if [[ -n "$hits" ]]; then
  printf 'error-hygiene: %s\n' "$hits" >&2
  failures=$(printf '%s\n' "$hits" | wc -l | tr -d ' ')
fi

if [[ "$failures" -gt 0 ]]; then
  cat >&2 <<'MSG'
error-hygiene: a 5xx error body or a health `detail` interpolates an error value —
that can carry SQL, a DSN, a filesystem path, an internal URL or a whole
thiserror chain to a client (and the health family is unauthenticated).
Substitute a fixed message and put the detail on the trace record instead (see
`classify_sqlx` in app/ferroehr/src/storage/error.rs for the pattern), or carry
the failure as a source (`SmError::with_source`) so the operator can walk the
chain while the client sees none of it. A 4xx may keep its specific,
client-facing detail; only 5xx is guarded.
MSG
  exit 1
fi
echo "error-hygiene: OK."
