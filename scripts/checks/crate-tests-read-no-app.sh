#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# A published crate's tests never read the application tree (#3408).
#
# The nine `crates/openehr-*` packages publish to crates.io and the
# dependency arrows point one way: `app/*` reads `crates/*`, never the
# reverse. A test that opens `../../app/ferroehr/tests/resources/...` breaks
# that in the one place the compiler cannot see it, because the path is a
# string. The crate then cannot be tested from its own package, and the
# application cannot move a fixture without breaking a sibling's suite.
#
# Shared material lives under `corpus/`, which every side reaches by a
# workspace-relative path of its own.
#
# Scope is the RUST sources under `crates/*/tests/`, because those are what
# resolve a path at run time. The crate sources are out of scope: they have no
# business naming a path outside their package either, and `cargo package`
# already holds them to it. So is the prose beside the fixtures. A
# `PROVENANCE.md` that names the application test which commits a corpus body
# is a record, not a read.
#
# Usage: scripts/checks/crate-tests-read-no-app.sh [--self-test]
#   no args     → every tracked Rust source under crates/*/tests/
#   --self-test → prove the detector in both directions
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
cd "$(dirname "$0")/../.."

# Two spellings reach the application from a crate test: the relative climb out
# of the package, and the workspace-relative name of an application crate. Both
# are matched as they appear inside a string literal.
readonly BANNED='\.\./\.\./app/|app/ferroehr'

# The test sources of every crate, as tracked by git: a path that git does not
# know is a build artifact, and a scope derived from the tree cannot go stale
# the way a hand-kept file list does.
sources() {
  git ls-files -- 'crates/*' | grep -E '^crates/[^/]+/tests/.*\.rs$' || true
}

# Report every banned mention in the files named on stdin, one
# `file:line: text` per finding. `-H` is explicit so a single file still names
# itself, and the list is fed in as arguments so a file whose name carries a
# space is still one argument.
findings() {
  local -a files=()
  local f
  while IFS= read -r f; do
    [[ -n "$f" && -f "$f" ]] && files+=("$f")
  done
  [[ "${#files[@]}" -gt 0 ]] || return 0
  grep -HnE "$BANNED" "${files[@]}" 2>/dev/null || true
}

# Proves the detector rather than trusting it: a scratch tree with one clean
# test file and one file per banned spelling, so a guard whose pattern silently
# matched nothing would fail here.
self_test() {
  local tmp
  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand $tmp now, while it is still in scope
  trap "rm -rf '$tmp'" RETURN
  mkdir -p "$tmp"
  cat >"$tmp/clean.rs" <<'RS'
fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/fixtures/service")
}
RS
  printf 'const D: &str = "../../app/ferroehr-rest/tests/fixture.json";\n' >"$tmp/climb.rs"
  printf 'let p = manifest.join("app/ferroehr/tests/resources/service");\n' >"$tmp/named.rs"

  local found failures=0 expected
  found="$(printf '%s\n' "$tmp"/clean.rs "$tmp"/climb.rs "$tmp"/named.rs | findings)"
  for expected in climb.rs named.rs; do
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
  echo "crate-tests-read-no-app: self-test OK (both spellings caught, the corpus path allowed)."
}

report() {
  local found="$1"
  if [[ -z "$found" ]]; then
    echo "crate-tests-read-no-app: OK (no test under crates/ reads the application tree)."
    return 0
  fi
  echo "error: a test in a published crate reads a path under app/" >&2
  echo >&2
  printf '%s\n' "$found" >&2
  echo >&2
  echo "The dependency arrows run app/* -> crates/*. A crates/ test that opens" >&2
  echo "a file under app/ cannot run from its own package and pins a fixture" >&2
  echo "the application is then not free to move. Put the material under" >&2
  echo "corpus/ and reach it by a workspace-relative path from both sides." >&2
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
