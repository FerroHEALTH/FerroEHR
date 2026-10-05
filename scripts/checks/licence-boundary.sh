#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The Apache-2.0 crates never depend on a BUSL-1.1 crate.
#
# Six `crates/*` packages publish under Apache-2.0 (the five generated model
# crates and the `openehr-its` wire layer) so any Rust project can take them;
# three hand-written engines publish under BUSL-1.1. A normal or build
# dependency from the first set on the second would make every consumer of the
# Apache wire layer compile BUSL code, which the licence split exists to rule
# out. Dev-dependencies are exempt: cargo strips them at packaging, so one
# never reaches a consumer.
#
# Four assertions over every `crates/*/Cargo.toml`, all mechanical:
#   1. the package is classified on exactly one side of the boundary, so a new
#      crate cannot publish without being placed;
#   2. its `license` matches its side: an Apache crate names `Apache-2.0` and
#      never `BUSL-1.1`, a BUSL crate is exactly `BUSL-1.1`, so relicensing a
#      crate cannot move it across silently;
#   3. an Apache crate's `[dependencies]` and `[build-dependencies]`, including
#      the `[target.*]` forms and a dependency renamed with `package =`, name no
#      BUSL crate;
#   4. no feature of an Apache crate names a BUSL crate (`dep:x`, `x/feature`,
#      `x?/feature`, or the bare optional-dependency name).
#
# No openEHR spec governs any of this — our own design.
#
# Usage: scripts/checks/licence-boundary.sh [--self-test]
#   no args     → every manifest under crates/
#   --self-test → prove each refusal on a mutated copy of the manifests
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
cd "$(dirname "$0")/../.."

readonly -a APACHE_CRATES=(openehr-base openehr-lang openehr-term openehr-rm openehr-am openehr-its)
readonly -a BUSL_CRATES=(openehr-query openehr-adl openehr-sdt)

for tool in yq jq; do
  command -v "$tool" >/dev/null || { echo "error: $tool is required" >&2; exit 1; }
done

# Whether $1 is one of the remaining arguments.
in_set() {
  local wanted="$1" member
  shift
  for member in "$@"; do
    [[ "$member" == "$wanted" ]] && return 0
  done
  return 1
}

# The findings for the manifests under "$1/crates", one `name: message` line
# each; nothing when the tree is clean.
findings() {
  local root="$1" manifest json name license dep
  local busl_json
  busl_json="$(printf '%s\n' "${BUSL_CRATES[@]}" | jq -R . | jq -sc .)"
  for manifest in "$root"/crates/*/Cargo.toml; do
    json="$(yq -p toml -o json '.' "$manifest")"
    name="$(jq -r '.package.name // ""' <<<"$json")"
    license="$(jq -r '.package.license // ""' <<<"$json")"
    if in_set "$name" "${APACHE_CRATES[@]}"; then
      if ! grep -qwF 'Apache-2.0' <<<"$license" || grep -qwF 'BUSL-1.1' <<<"$license"; then
        echo "$name: license is \"$license\"; an Apache-side crate names Apache-2.0 and never BUSL-1.1"
      fi
    elif in_set "$name" "${BUSL_CRATES[@]}"; then
      [[ "$license" == 'BUSL-1.1' ]] ||
        echo "$name: license is \"$license\"; a BUSL-side crate is exactly BUSL-1.1"
      continue
    else
      echo "${name:-$manifest}: not classified on either side of the licence boundary"
      continue
    fi
    # Every normal and build dependency as `<table key> <package>`, so a
    # renamed dependency is judged by the package it actually pulls.
    while IFS= read -r dep; do
      [[ -n "$dep" ]] && echo "$name: depends on the BUSL-1.1 crate $dep outside [dev-dependencies]"
    done < <(jq -r --argjson busl "$busl_json" '
      [ (.dependencies // {}), (."build-dependencies" // {}),
        ((.target // {}) | .[] | (.dependencies // {}), (."build-dependencies" // {})) ]
      | map(to_entries[]) | .[]
      | (if (.value | type) == "object" then (.value.package // .key) else .key end) as $pkg
      | select(any($busl[]; . == $pkg))
      | $pkg' <<<"$json")
    while IFS= read -r dep; do
      [[ -n "$dep" ]] && echo "$name: a feature names the BUSL-1.1 crate $dep"
    done < <(jq -r --argjson busl "$busl_json" '
      (.features // {}) | to_entries[] | .key as $feature | .value[]
      | sub("^dep:"; "") | sub("\\??/.*$"; "")
      | . as $crate | select(any($busl[]; . == $crate))
      | "\(.) (feature \($feature))"' <<<"$json")
  done
}

report() {
  local found="$1"
  if [[ -z "$found" ]]; then
    echo "licence-boundary: OK (no Apache-2.0 crate depends on a BUSL-1.1 crate)."
    return 0
  fi
  echo "error: the Apache-2.0 / BUSL-1.1 crate boundary is crossed" >&2
  echo >&2
  printf '%s\n' "$found" >&2
  echo >&2
  echo "The Apache-2.0 crates (${APACHE_CRATES[*]}) publish so any project can" >&2
  echo "take them; a normal or build dependency on a BUSL-1.1 crate" >&2
  echo "(${BUSL_CRATES[*]}) would make every consumer compile BUSL code. Keep such" >&2
  echo "a dependency in [dev-dependencies], path-only, or move the code." >&2
  return 1
}

# Proves every refusal rather than trusting it: each mutation runs on a fresh
# copy of the real manifests, and the clean copy and a dev-dependency on a
# BUSL crate must both pass.
self_test() {
  local tmp failures=0
  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand $tmp now, while it is still in scope
  trap "rm -rf '$tmp'" RETURN

  fresh() {
    rm -rf "$tmp/crates"
    local manifest crate
    for manifest in crates/*/Cargo.toml; do
      crate="$(basename "$(dirname "$manifest")")"
      mkdir -p "$tmp/crates/$crate"
      cp "$manifest" "$tmp/crates/$crate/Cargo.toml"
    done
  }
  # $1 = case label, $2 = expected (clean | refused), $3 = expected text.
  expect() {
    local found
    found="$(findings "$tmp")"
    if [[ "$2" == clean && -n "$found" ]]; then
      echo "self-test: $1 was wrongly refused: $found" >&2
      failures=1
    elif [[ "$2" == refused ]] && ! grep -qF "$3" <<<"$found"; then
      echo "self-test: $1 was not refused (wanted: $3; got: ${found:-nothing})" >&2
      failures=1
    fi
  }
  local its="$tmp/crates/openehr-its/Cargo.toml"

  fresh
  expect 'the real manifests' clean ''

  fresh
  perl -0pi -e 's/^\[dependencies\]\n/[dependencies]\nopenehr-sdt = { path = "..\/openehr-sdt" }\n/m' "$its"
  expect 'a normal dependency' refused 'openehr-its: depends on the BUSL-1.1 crate openehr-sdt'

  fresh
  perl -0pi -e 's/^\[dependencies\]\n/[dependencies]\nsdt = { package = "openehr-sdt", path = "..\/openehr-sdt" }\n/m' "$its"
  expect 'a renamed dependency' refused 'openehr-its: depends on the BUSL-1.1 crate openehr-sdt'

  fresh
  printf '\n[build-dependencies]\nopenehr-query = { path = "../openehr-query" }\n' >>"$its"
  expect 'a build dependency' refused 'openehr-its: depends on the BUSL-1.1 crate openehr-query'

  fresh
  printf '\n[target.%s.dependencies]\nopenehr-adl = { path = "../openehr-adl" }\n' "'cfg(unix)'" >>"$its"
  expect 'a target dependency' refused 'openehr-its: depends on the BUSL-1.1 crate openehr-adl'

  fresh
  perl -0pi -e 's/^\[features\]\n/[features]\nboundary-probe = ["dep:openehr-adl"]\n/m' "$its"
  expect 'a dep: feature entry' refused 'openehr-its: a feature names the BUSL-1.1 crate openehr-adl (feature boundary-probe)'

  fresh
  perl -0pi -e 's/^\[features\]\n/[features]\nboundary-probe = ["openehr-sdt?\/flat"]\n/m' "$its"
  expect 'a crate/feature entry' refused 'openehr-its: a feature names the BUSL-1.1 crate openehr-sdt (feature boundary-probe)'

  fresh
  perl -pi -e 's/^license = .*/license = "BUSL-1.1"/' "$tmp/crates/openehr-rm/Cargo.toml"
  expect 'an Apache crate relicensed' refused 'openehr-rm: license is "BUSL-1.1"'

  fresh
  perl -pi -e 's/^license = .*/license = "Apache-2.0"/' "$tmp/crates/openehr-sdt/Cargo.toml"
  expect 'a BUSL crate relicensed' refused 'openehr-sdt: license is "Apache-2.0"'

  fresh
  mkdir -p "$tmp/crates/openehr-new"
  printf '[package]\nname = "openehr-new"\nlicense = "Apache-2.0"\n' >"$tmp/crates/openehr-new/Cargo.toml"
  expect 'an unclassified crate' refused 'openehr-new: not classified'

  fresh
  perl -0pi -e 's/^\[dev-dependencies\]\n/[dev-dependencies]\nopenehr-sdt = { path = "..\/openehr-sdt" }\n/m' "$its"
  expect 'a dev-dependency' clean ''

  [[ "$failures" -eq 0 ]] || return 1
  echo "licence-boundary: self-test OK (every crossing refused, the dev-dependency and the real manifests allowed)."
}

case "${1:-}" in
--self-test)
  self_test
  ;;
"")
  [[ "$#" -eq 0 ]] || guard_usage "[--self-test]"
  report "$(findings .)"
  ;;
*)
  guard_usage "[--self-test]"
  ;;
esac
