#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Proves crate_version_bump_guard.sh end to end (#3573, #4555): a scratch repository
# with an `origin/main` ref stands in for the project, each outgoing change is
# committed on a branch, a `git push` tool call is fed to the hook, and the
# exit code is asserted. Run by the shellcheck CI job beside the lane, and by
# hand: bash .claude/hooks/crate_version_bump_guard_test.sh
set -euo pipefail
cd "$(dirname "$0")/../.." || exit 1
readonly HOOK="$PWD/.claude/hooks/crate_version_bump_guard.sh"
readonly CRATES='base rm am adl term lang query its sdt'

tmp="$(mktemp -d)"
# shellcheck disable=SC2064 # expand $tmp now, while it is still in scope
trap "rm -rf '$tmp'" EXIT

git_() { git -C "$tmp" -c user.name=test -c user.email=test@example.invalid -c commit.gpgsign=false "$@"; }

# Write every crate manifest and the fuzz lock at one lockstep version, each
# member but openehr-base requiring it by path (openehr-rm at <req>, default
# the version).
write_version() {
  local ver="$1" req="${2:-$1}" c dep
  : >"$tmp/fuzz/Cargo.lock"
  for c in $CRATES; do
    printf '[package]\nname = "openehr-%s"\nversion = "%s"\n' "$c" "$ver" >"$tmp/crates/openehr-$c/Cargo.toml"
    dep="$ver"
    [[ "$c" == rm ]] && dep="$req"
    [[ "$c" == base ]] || printf '\n[dependencies]\nopenehr-base = { path = "../openehr-base", version = "%s" }\n' "$dep" >>"$tmp/crates/openehr-$c/Cargo.toml"
    printf '[[package]]\nname = "openehr-%s"\nversion = "%s"\n\n' "$c" "$ver" >>"$tmp/fuzz/Cargo.lock"
  done
}

git_ init -q --initial-branch=main .
mkdir -p "$tmp/fuzz"
for c in $CRATES; do mkdir -p "$tmp/crates/openehr-$c"; done
write_version 0.0.1
printf 'Business Source License 1.1\n' >"$tmp/crates/openehr-sdt/LICENSE"
printf 'Apache License 2.0\n' >"$tmp/crates/openehr-rm/LICENSE-APACHE-2.0"
mkdir -p "$tmp/crates/openehr-sdt/tests"
printf '// a test\n' >"$tmp/crates/openehr-sdt/tests/flat.rs"
git_ add -A
git_ commit -qm base
git_ update-ref refs/remotes/origin/main HEAD

failures=0
# expect <0|2> <label> — run the hook against the current branch tip.
expect() {
  local want="$1" label="$2" got payload
  payload="$(jq -cn '{tool_input: {command: "git push origin HEAD"}}')"
  if (cd "$tmp" && bash "$HOOK" <<<"$payload" 2>/dev/null); then got=0; else got=$?; fi
  if [[ "$got" != "$want" ]]; then
    echo "crate_version_bump_guard_test: $label: expected exit $want, got $got" >&2
    failures=1
  fi
}

# expect_escape <0|2> <label>: the same, with the escape on the command.
expect_escape() {
  local want="$1" label="$2" got payload
  payload="$(jq -cn '{tool_input: {command: "FERROEHR_SKIP_CRATE_BUMP_GUARD=1 git push origin HEAD"}}')"
  if (cd "$tmp" && bash "$HOOK" <<<"$payload" 2>/dev/null); then got=0; else got=$?; fi
  if [[ "$got" != "$want" ]]; then
    echo "crate_version_bump_guard_test: $label: expected exit $want, got $got" >&2
    failures=1
  fi
}

# change <branch> <file> — a fresh branch off main whose one commit edits <file>.
change() {
  git_ switch -qc "$1" main
  printf 'edited\n' >>"$tmp/$2"
  git_ commit -qam "$1"
}

# A pull request changes packaged content and leaves the version alone; the
# publish step bumps (#4555).
change bare-licence crates/openehr-sdt/LICENSE
expect 0 'a bare LICENSE change without a version edit'

change suffixed-licence crates/openehr-rm/LICENSE-APACHE-2.0
expect 0 'a LICENSE-APACHE-2.0 change without a version edit'

change bumped crates/openehr-sdt/LICENSE
write_version 0.0.2
git_ commit -qam bump
expect 2 'a lockstep bump in the pushed commits'

change requirement-moved crates/openehr-sdt/LICENSE
write_version 0.0.1 0.0.2
git_ commit -qam bump
expect 2 'an internal version requirement moved by hand'

# The escape passes even a refused push.
change escaped crates/openehr-sdt/LICENSE
write_version 0.0.2
git_ commit -qam bump
expect_escape 0 'a refused push with FERROEHR_SKIP_CRATE_BUMP_GUARD=1'

change test-only crates/openehr-sdt/tests/flat.rs
expect 0 'a change outside the packaged content'

if [[ "$failures" -ne 0 ]]; then
  exit 1
fi
echo "crate_version_bump_guard_test: OK (packaged changes pass without a version edit, a hand bump and a moved internal requirement refused, the escape honoured, unpackaged changes ignored)."
