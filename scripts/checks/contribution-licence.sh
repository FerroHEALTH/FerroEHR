#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Every pull request from a person records acceptance of the contribution
# licensing terms (CONTRIBUTING.md § Licensing of contributions).
#
# The terms turn a contribution into part of one Licensed Work under one
# Licensor; a merge without a recorded acceptance is a line whose relicensing
# right was never granted. A checkbox in the pull request template is the
# record, and this gate is what makes it binding: a body without the ticked
# line fails, whether the box was left empty or the section deleted.
#
# Bots cannot accept terms, and their changes carry no copyrightable expression
# of their own (dependency and pin bumps); the workflow skips them by author
# type before this script runs.
#
# Usage: PR_BODY="$body" scripts/checks/contribution-licence.sh   (no arguments)
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"

readonly ACCEPTED='^[[:space:]]*[-*] \[[xX]\] I accept the terms in \[CONTRIBUTING\.md § Licensing of contributions\]'
readonly UNTICKED='^[[:space:]]*[-*] \[ \] I accept the terms in \[CONTRIBUTING\.md § Licensing of contributions\]'

body="${PR_BODY:-}"
if printf '%s\n' "$body" | grep -qE "$ACCEPTED"; then
  echo "ok: the contribution licensing terms are accepted in the pull request body"
  exit 0
fi
if printf '%s\n' "$body" | grep -qE "$UNTICKED"; then
  echo "error: the licensing checkbox in the pull request body is not ticked" >&2
else
  echo "error: the pull request body carries no licensing acceptance line" >&2
  echo "       (the pull request template's 'Licensing of contributions' section was removed)" >&2
fi
echo >&2
echo "Tick the box under 'Licensing of contributions' in the pull request" >&2
echo "description (edit the description; the check re-runs on the edit). The" >&2
echo "terms are CONTRIBUTING.md § Licensing of contributions." >&2
exit 1
