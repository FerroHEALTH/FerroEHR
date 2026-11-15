#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The access decision holds no rule of one country (#5691).
#
# Who may read or write is decided by one evaluator over coded content the
# CDR holds: a controller states a national rule, such as Wabvpz Art. 15f, as
# a rule of its ACCESS_POLICY. A decision in Rust that branches on a country
# or a jurisdiction value, or names one country's law, puts that rule back in
# the binary, where no deployment can read or change it.
#
# The scope is the code that admits a caller and decides an access: the
# authentication and access extension of the REST crate, the evaluator, the
# policy check and the party binding. The detector refuses an ISO 3166-1
# alpha-2 code compared or matched as a string literal, a `jurisdiction`
# value read in a condition, and a country-prefixed attribute such as
# `Field::NlRole`.
#
# Usage: scripts/checks/access-decides-no-country.sh [--self-test]
#   no args     → every tracked Rust source in the scope above
#   --self-test → prove the detector in both directions
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
cd "$(dirname "$0")/../.."

# A two-letter upper-case literal compared or matched, a jurisdiction in a
# condition, and a country-prefixed federation field or refusal code.
readonly BANNED='(==|!=)[[:space:]]*"[A-Z]{2}"|"[A-Z]{2}"[[:space:]]*(==|!=|=>|\|)|\bjurisdiction\b[^;]*(==|!=)|Field::(Nl|De|Fr|Be|Ch|At)[A-Z][A-Za-z]*|"refused-(wabvpz|nl|de)-'

sources() {
  git ls-files -- \
    'app/ferroehr-rest/src/extensions/access' \
    'app/ferroehr/src/access_eval' \
    'app/ferroehr/src/access_policy' \
    'app/ferroehr/src/party_binding' |
    grep -E '\.rs$' || true
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
  printf 'let refused = rule.applies(&caller) && rule.effect == Effect::Deny;\n' >"$tmp/clean.rs"
  printf 'if agreement.jurisdiction == "NL" { return refuse(); }\n' >"$tmp/jurisdiction.rs"
  printf 'match country { "DE" => german(), _ => none() }\n' >"$tmp/arm.rs"
  printf 'let role = external.field(Field::NlRole);\n' >"$tmp/field.rs"
  printf 'code: "refused-wabvpz-role",\n' >"$tmp/code.rs"

  local found failures=0 expected
  found="$(printf '%s\n' "$tmp"/clean.rs "$tmp"/jurisdiction.rs "$tmp"/arm.rs \
    "$tmp"/field.rs "$tmp"/code.rs | findings)"
  for expected in jurisdiction.rs arm.rs field.rs code.rs; do
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
  echo "access-decides-no-country: self-test OK (a country literal, a jurisdiction, a country field and a national refusal caught, a coded rule allowed)."
}

report() {
  local found="$1"
  if [[ -z "$found" ]]; then
    echo "access-decides-no-country: OK (the access decision names no country)."
    return 0
  fi
  echo "error: the access decision holds a rule of one country" >&2
  echo >&2
  printf '%s\n' "$found" >&2
  echo >&2
  echo "State the rule as coded content: a deny rule of the controller's" >&2
  echo "ACCESS_POLICY over coded roles, organisation types and purposes, with" >&2
  echo "the provision it rests on in its legal_grounds." >&2
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
