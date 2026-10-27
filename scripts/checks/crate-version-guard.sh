#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The crate-version guard (.claude/rules/crates-publishing.md): a change to
# PACKAGED content of the nine published crates/openehr-* members bumps their
# lockstep 0.0.x version in the same change, because a published version is
# immutable. The `crate-version-guard` CI job runs it on every pull request;
# run it locally before you push.
#
# Packaged content is what crate-packaged-paths.sh classifies, plus a root
# [workspace.dependencies] entry a member consumes with `workspace = true`
# (`cargo package` renders the concrete requirement, #3036). When packaged
# content moved, the guard requires that:
#
#   1. crates/openehr-base carries a new version;
#   2. all nine crates carry that one version;
#   3. fuzz/Cargo.lock records each of the nine at it (fuzz/ is its own
#      workspace, so the root lock refresh never reaches it, #2762);
#   4. every internal `path = "../openehr-*", version = "…"` requirement
#      names it.
#
# The change is read from the merge base of <head> with <base> (#4129), so a
# branch behind main is neither judged on main's paths nor credited with
# main's bump. The `no-crate-bump` label is the CI job's to read; this script
# never sees it.
#
# Every file is read whole into a variable and matched from a here-string: a
# reader that stops early (`grep -q`, `grep -m1`) on a pipe kills its writer
# with SIGPIPE, and under pipefail a match then reads as a miss.
#
# Usage: scripts/checks/crate-version-guard.sh <base> <head>
#          exit 0 when the change needs no bump or carries a correct one,
#          1 when it is refused, 2 on a usage error
#        scripts/checks/crate-version-guard.sh --self-test
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
# shellcheck source=scripts/lib/changed-paths.sh
. "$(dirname "$0")/../lib/changed-paths.sh"

readonly CRATES="base rm am adl term lang query its sdt"
HERE="$(cd "$(dirname "$0")" && pwd)"
readonly HERE
SELF="$HERE/$(basename "$0")"
readonly SELF
readonly PACKAGED_PATHS="$HERE/crate-packaged-paths.sh"

refuse() {
  echo "::error::$1" >&2
  exit 1
}

# The first `version = "…"` of a manifest's text, bare.
manifest_version() {
  local line
  line="$(grep -m1 '^version = ' <<<"$1" || true)"
  sed -E 's/^version = "([^"]+)".*/\1/' <<<"$line"
}

# Whether a root Cargo.toml change moves a [workspace.dependencies] entry
# some crates/openehr-* member consumes.
workspace_dependency_moved() {
  local fork="$1" head="$2" manifest table diff names name
  manifest="$(git show "$head:Cargo.toml")"
  table="$(awk '/^\[workspace\.dependencies\]/{f=1;next} /^\[/{f=0} f && /^[A-Za-z0-9_-]+[. =]/{print $1}' <<<"$manifest" | sed 's/\.workspace$//' | sort -u)"
  diff="$(git diff "$fork" "$head" -- Cargo.toml)"
  names="$({ grep -E '^[+-][A-Za-z0-9_-]+(\.workspace)?[[:space:]]*=' <<<"$diff" || true; } | sed -E 's/^[+-]//; s/[[:space:]]*=.*//; s/\.workspace$//' | sort -u)"
  for name in $names; do
    grep -qx -- "$name" <<<"$table" || continue
    if git grep -qE "^${name}(\.workspace)?[[:space:]]*=" "$head" -- 'crates/openehr-*/Cargo.toml'; then
      echo "workspace dependency '$name' changed and crates/* members consume it: packaged requirements move."
      return 0
    fi
  done
  return 1
}

guard() {
  local base="$1" head="$2" fork changed verdict=0 packaged=0
  local old new ver c v locked fuzz_lock grepped reqs
  fork="$(change_fork "$base" "$head")"
  changed="$(change_paths "$base" "$head")"
  printf '%s\n' "$changed" | bash "$PACKAGED_PATHS" || verdict=$?
  case "$verdict" in
  0) packaged=1 ;;
  1) ;;
  *) refuse "$PACKAGED_PATHS failed (exit $verdict)." ;;
  esac
  if [[ "$packaged" -eq 0 ]] && grep -qx 'Cargo.toml' <<<"$changed"; then
    workspace_dependency_moved "$fork" "$head" && packaged=1
  fi
  if [[ "$packaged" -eq 0 ]]; then
    echo "No packaged crate content touched: guard not required."
    return 0
  fi
  old="$(manifest_version "$(git show "$fork:crates/openehr-base/Cargo.toml")")"
  new="$(manifest_version "$(git show "$head:crates/openehr-base/Cargo.toml")")"
  [[ -n "$new" ]] || refuse "could not read the version of crates/openehr-base/Cargo.toml at $head."
  if [[ "$old" == "$new" ]]; then
    refuse "This change touches packaged content of the published crates/* members but does not bump the lockstep 0.0.x crate version (bump all nine 'version' fields and the internal version requirements, .claude/rules/crates-publishing.md), and a pull request needs the 'no-crate-bump' label to skip it."
  fi
  ver="$new"
  fuzz_lock="$(git show "$head:fuzz/Cargo.lock")"
  for c in $CRATES; do
    v="$(manifest_version "$(git show "$head:crates/openehr-$c/Cargo.toml")")"
    [[ "$v" == "$ver" ]] || refuse "crates/openehr-$c is at ${v:-nothing} but the lockstep version is $ver: bump all nine together."
    locked="$(awk -v n="\"openehr-$c\"" '$1 == "name" && $3 == n { hit = 1; next } hit && $1 == "version" { gsub(/"/, "", $3); print $3; exit }' <<<"$fuzz_lock")"
    [[ "$locked" == "$ver" ]] || refuse "fuzz/Cargo.lock records openehr-$c ${locked:-nothing} but the lockstep version is $ver: refresh it with 'cargo update --manifest-path fuzz/Cargo.toml --workspace' and commit the lock in this change."
  done
  grepped="$(git grep -hoE 'path = "\.\./openehr-[a-z]+", version = "[0-9.]+"' "$head" -- 'crates/*/Cargo.toml' || true)"
  reqs="$({ grep -oE '[0-9]+\.[0-9]+\.[0-9]+' <<<"$grepped" || true; } | sort -u)"
  [[ "$reqs" == "$ver" ]] || refuse "internal openehr-* version requirements (${reqs//$'\n'/ }) do not all match the lockstep version $ver: update every 'version =' on the path dependencies too."
  echo "Lockstep crate version bumped to $ver: OK."
}

# --- self-test -------------------------------------------------------------

# Write the nine manifests at a version, with one internal requirement each
# (openehr-rm's at <req>, default the version) and a workspace dependency.
_write_manifests() {
  local ver="$1" req="${2:-$1}" c dep
  for c in $CRATES; do
    mkdir -p "crates/openehr-$c/src" "crates/openehr-$c/tests"
    dep="$ver"
    [[ "$c" == rm ]] && dep="$req"
    printf '[package]\nname = "openehr-%s"\nversion = "%s"\n\n[dependencies]\nserde.workspace = true\n' "$c" "$ver" >"crates/openehr-$c/Cargo.toml"
    [[ "$c" == base ]] || printf 'openehr-base = { path = "../openehr-base", version = "%s" }\n' "$dep" >>"crates/openehr-$c/Cargo.toml"
  done
}

_write_fuzz_lock() {
  local ver="$1" c
  mkdir -p fuzz
  : >fuzz/Cargo.lock
  for c in $CRATES; do
    printf '[[package]]\nname = "openehr-%s"\nversion = "%s"\n\n' "$c" "$ver" >>fuzz/Cargo.lock
  done
}

# A root manifest whose [workspace.dependencies] table opens with serde and
# then runs past a pipe buffer, so a reader stopping at serde would kill its
# writer.
_write_root_manifest() {
  local serde="$1" i
  {
    printf '[workspace]\nmembers = ["crates/*"]\n\n[workspace.dependencies]\n'
    printf 'serde = "%s"\n' "$serde"
    for ((i = 0; i < 3000; i++)); do
      printf 'padding-dependency-number-%05d = "1.0.0"\n' "$i"
    done
  } >Cargo.toml
}

_commit() {
  local message="$1"
  git add -A
  git -c user.name=self-test -c user.email=self-test@invalid -c commit.gpgsign=false \
    commit -qm "$message"
}

# <label> <want exit> <setup function>: branch from main, apply the setup,
# commit, and run the guard on main..branch.
_case() {
  local label="$1" want="$2" setup="$3" got=0
  git checkout -q -b "case-$setup" main
  "$setup"
  _commit "$label"
  bash "$SELF" main HEAD >/dev/null 2>&1 || got=$?
  git checkout -q main
  if [[ "$got" -ne "$want" ]]; then
    echo "self-test: $label: expected exit $want, got $got" >&2
    return 1
  fi
  echo "  self-test: $label: exit $got"
}

_src_change() { echo "// changed" >>crates/openehr-adl/src/lib.rs; }
_bumped() { _src_change; _write_manifests 0.0.2; _write_fuzz_lock 0.0.2; }
_no_bump() { _src_change; }
_lock_behind() { _src_change; _write_manifests 0.0.2; }
_one_crate_behind() {
  _bumped
  sed -i.bak 's/^version = "0.0.2"/version = "0.0.1"/' crates/openehr-sdt/Cargo.toml
  rm crates/openehr-sdt/Cargo.toml.bak
}
_requirement_behind() { _src_change; _write_manifests 0.0.2 0.0.1; _write_fuzz_lock 0.0.2; }
_tests_only() { echo "// test" >>crates/openehr-adl/tests/it.rs; }
_workspace_dependency() { _write_root_manifest 1.0.1; }
_workspace_dependency_bumped() { _write_root_manifest 1.0.1; _write_manifests 0.0.2; _write_fuzz_lock 0.0.2; }
# More changed paths than a pipe buffer holds, Cargo.toml sorted first among
# them: a reader stopping at the match would kill its writer.
_many_paths() {
  local i
  _write_root_manifest 1.0.1
  mkdir -p docs/padding
  for ((i = 0; i < 2000; i++)); do
    printf '%s\n' "$i" >"docs/padding/a-long-padding-file-name-to-fill-the-pipe-$i.md"
  done
}

self_test() {
  local tmp failures=0 c
  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand $tmp now, while it is still in scope
  trap "rm -rf '$tmp'" EXIT
  cd "$tmp"
  git init -q -b main
  # A commit past the loose-object limit starts `git maintenance --auto` detached,
  # which still writes the packs under .git while the EXIT trap removes it (#4237).
  git config gc.auto 0
  git config maintenance.auto false
  _write_root_manifest 1.0.0
  _write_manifests 0.0.1
  _write_fuzz_lock 0.0.1
  for c in $CRATES; do
    echo "// lib" >"crates/openehr-$c/src/lib.rs"
    echo "// it" >"crates/openehr-$c/tests/it.rs"
  done
  _commit fork
  _case "a packaged change with the lockstep bump passes" 0 _bumped || failures=1
  _case "a packaged change without a bump is refused" 1 _no_bump || failures=1
  _case "a bump with fuzz/Cargo.lock out of step is refused" 1 _lock_behind || failures=1
  _case "one of the nine left behind is refused" 1 _one_crate_behind || failures=1
  _case "an internal requirement left behind is refused" 1 _requirement_behind || failures=1
  _case "a tests-only change needs no bump" 0 _tests_only || failures=1
  _case "a consumed workspace dependency moved without a bump is refused (manifest past a pipe buffer)" 1 _workspace_dependency || failures=1
  _case "a consumed workspace dependency moved with the bump passes" 0 _workspace_dependency_bumped || failures=1
  _case "a workspace dependency moved among paths past a pipe buffer is refused" 1 _many_paths || failures=1
  [[ "$failures" -eq 0 ]] || return 1
  echo "crate-version-guard: self-test OK."
}

case "${1:-}" in
--self-test)
  [[ "$#" -eq 1 ]] || guard_usage "<base> <head> | --self-test"
  self_test
  ;;
-*)
  guard_usage "<base> <head> | --self-test"
  ;;
*)
  [[ "$#" -eq 2 ]] || guard_usage "<base> <head> | --self-test"
  guard "$1" "$2"
  ;;
esac
