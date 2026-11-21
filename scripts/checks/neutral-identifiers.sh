#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Nothing FerroEHR writes or sends is named after the product (#6143).
#
# An identifier that ends up in stored openEHR content, in a published
# definition, on the wire to another system or in a token is either an
# openEHR or international code, or a value the deployment configures with a
# neutral default, so that another CDR or client reads what FerroEHR stores
# and sends. Names that never leave the product (database roles, the
# configuration file, metrics, the viewer's own client id, test fixtures)
# keep the product name and are out of this guard's scope.
#
# The scope is the shipped definitions and the hand-written Rust that is no
# test: every tracked `.rs` file under `app/`, `crates/` and `tools/` outside
# a `tests/` directory, a `tests.rs` file or a `tests` module, and every
# `.adls` file under `app/ferroehr/assets/definitions/` and
# `deploy/definitions/`. The detector refuses, in any case, an archetype or
# template id whose concept names `ferro`, a URN with `ferro` in it, and an
# issuer, or an `*ISSUER` constant, whose value names `ferro`, all but the
# family's SCIM extension schema URN.
#
# Usage: scripts/checks/neutral-identifiers.sh [--self-test]
#   no args     → the scope above
#   --self-test → prove the detector in both directions
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
cd "$(dirname "$0")/../.."

# An archetype or template id naming the product, a URN naming it, and an
# issuer (a field, a key or an `*ISSUER` constant) whose value names it.
readonly BANNED='openehr-[a-z_]+-[a-z_]+\.[a-z0-9_-]*ferro[a-z0-9_-]*\.v[0-9]|urn:[^"[:space:]]*ferro|issuer[a-z_]*"?[[:space:]]*(:|=|=>)[[:space:]]*(some\()?"[^"]*ferro|issuer[a-z_]*:[[:space:]]*&str[[:space:]]*=[[:space:]]*"[^"]*ferro'

# The one exception: the FerroHEALTH family's SCIM extension schema
# (app/ferroehr-rest/src/scim/endpoint/schemas.rs lines 43 and 48,
# app/ferroehr/src/config/binding/person.rs line 41), the provisioning
# interface the family's products share, an API contract and no stored
# content; #6154 settles it with them.
readonly FAMILY_SCIM='urn:ietf:params:scim:schemas:extension:ferrohealth:2\.0:'

sources() {
  git ls-files -- 'app/*.rs' 'crates/*.rs' 'tools/*.rs' |
    grep -Ev '(^|/)tests/|(^|/)tests\.rs$|_tests\.rs$|/vendor/' || true
  git ls-files -- 'app/ferroehr/assets/definitions/*.adls' 'deploy/definitions/*.adls'
}

# Report every banned mention in the files named on stdin as `file:line: text`,
# reading a Rust file only up to its `#[cfg(test)]` module.
findings() {
  local -a files=()
  local f
  while IFS= read -r f; do
    [[ -n "$f" && -f "$f" ]] && files+=("$f")
  done
  [[ "${#files[@]}" -gt 0 ]] || return 0
  awk '
    FNR == 1 { skip = 0; test = 0 }
    skip { next }
    /^[[:space:]]*#\[cfg\(test\)\]/ { test = 1; next }
    test && /^[[:space:]]*(pub(\([a-z]+\))? )?mod [a-z_]+/ { skip = 1; next }
    { test = 0; print FILENAME ":" FNR ": " $0 }
  ' "${files[@]}" | grep -iE "^[^:]+:[0-9]+: .*($BANNED)" | grep -vE "$FAMILY_SCIM" || true
}

self_test() {
  local tmp
  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand $tmp now, while it is still in scope
  trap "rm -rf '$tmp'" RETURN
  printf 'const ARCHETYPE: &str = "openEHR-EHR-ITEM_TREE.access_log_details.v1";\nlet key = "ferroehr_clinical";\nconst ISSUER: &str = "urn:example:login";\n' >"$tmp/clean.rs"
  printf 'const ARCHETYPE: &str = "openEHR-EHR-ITEM_TREE.ferroehr_access_details.v0";\n' >"$tmp/archetype.rs"
  printf '    openEHR-DEMOGRAPHIC-ROLE.ferroehr_grant.v1.0.0\n' >"$tmp/definition.adls"
  printf 'const ROLE_VOCABULARY: &str = "urn:ferroehr:role";\n' >"$tmp/urn.rs"
  printf 'const LOCAL_ISSUER: &str = "ferroehr";\n' >"$tmp/constant.rs"
  printf '    issuer: Some("FerroEHR".to_owned()),\n' >"$tmp/field.rs"
  printf 'fn ok() {}\n#[cfg(test)]\nmod tests {\n    const ISSUER: &str = "urn:ferroehr:test";\n}\n' >"$tmp/module.rs"

  local found failures=0 expected
  found="$(printf '%s\n' "$tmp"/clean.rs "$tmp"/archetype.rs "$tmp"/definition.adls \
    "$tmp"/urn.rs "$tmp"/constant.rs "$tmp"/field.rs "$tmp"/module.rs | findings)"
  for expected in archetype.rs definition.adls urn.rs constant.rs field.rs; do
    grep -q "$expected" <<<"$found" || {
      echo "self-test: $expected was not caught" >&2
      failures=1
    }
  done
  for expected in clean.rs module.rs; do
    if grep -q "$expected" <<<"$found"; then
      echo "self-test: $expected was wrongly caught" >&2
      failures=1
    fi
  done
  [[ "$failures" -eq 0 ]] || return 1
  echo "neutral-identifiers: self-test OK (a product archetype id, URN, issuer constant and issuer field caught, neutral ids, internal names and a test module allowed)."
}

report() {
  local found="$1"
  if [[ -z "$found" ]]; then
    echo "neutral-identifiers: OK (no archetype id, template id, URN or issuer names the product)."
    return 0
  fi
  echo "error: an identifier FerroEHR writes or sends names the product" >&2
  echo >&2
  printf '%s\n' "$found" >&2
  echo >&2
  echo "Use an openEHR or international code, a neutral shipped definition, or a" >&2
  echo "value the deployment configures whose default is neutral, such as one" >&2
  echo "derived from [api] system_id (#6143)." >&2
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
