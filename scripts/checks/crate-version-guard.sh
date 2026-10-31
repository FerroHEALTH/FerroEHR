#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The crate-version guard (.claude/rules/crates-publishing.md). The nine
# published crates/openehr-* members share one lockstep 0.0.x version, and
# the publish step sets it (#4555): scripts/release/publish-crates.sh reads
# crates.io, stamps the next version into the nine manifests, the internal
# requirements and both lockfiles of the publishing checkout, and uploads.
# A pull request therefore never edits that version. Two open pull requests
# that both took "the next version" always conflicted on nine manifests and
# two lockfiles.
#
# Packaged content is what crate-packaged-paths.sh classifies, plus a root
# [workspace.dependencies] or [workspace.package] entry a member inherits
# with `workspace = true` (`cargo package` renders the concrete value, #3036).
#
# Two modes:
#
#   <base> <head>      the pull-request check. Refuses a change that edits
#                      the lockstep version of a member or an internal
#                      `path = "../openehr-*", version = "…"` requirement,
#                      and reports whether packaged content changed, which
#                      the next publish turns into a new version.
#   changed <from> <to>
#                      the publish step's question: did packaged content
#                      change between the commit a published version was
#                      packaged from and <to>? Exit 0 when it did, 1 when it
#                      did not. publish-crates.sh asks it to pick the version
#                      and to refuse changed content under a version crates.io
#                      already holds.
#
# Both read the change from the merge base of <head> with <base> (#4129), so
# a branch behind main is not judged on main's paths. The `no-crate-bump`
# label is the CI job's to read; this script never sees it.
#
# Every file is read whole into a variable and matched from a here-string: a
# reader that stops early (`grep -q`, `grep -m1`) on a pipe kills its writer
# with SIGPIPE, and under pipefail a match then reads as a miss.
#
# Usage: scripts/checks/crate-version-guard.sh <base> <head>
#          exit 0 when the change leaves the lockstep version alone,
#          1 when it is refused, 2 on a usage error
#        scripts/checks/crate-version-guard.sh changed <from> <to>
#          exit 0 when packaged content changed, 1 when it did not,
#          2 on a usage error, 3 when it cannot be judged
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
readonly USAGE="<base> <head> | changed <from> <to> | --self-test"

refuse() {
  echo "::error::$1" >&2
  exit 1
}

# The `[package]` table's own version of a manifest's text, bare; never a
# dependency's `version =`.
manifest_version() {
  awk -F'"' '/^\[package\]/{p=1; next} /^\[/{p=0} p && /^version[[:space:]]*=/{print $2; exit}' <<<"$1"
}

# A manifest at a commit, or nothing when the file is absent there.
manifest_at() {
  git show "$1:$2" 2>/dev/null || true
}

# The internal `path = "../openehr-*", version = "…"` requirements at a
# commit, one `<file>: <requirement>` per line, sorted.
internal_requirements() {
  local grepped
  grepped="$(git grep -oE 'path = "\.\./openehr-[a-z]+", version = "[^"]+"' "$1" -- 'crates/*/Cargo.toml' || true)"
  sed -E "s/^$1://" <<<"$grepped" | sort
}

# Whether a root Cargo.toml change moves a [workspace.dependencies] or
# [workspace.package] entry some crates/openehr-* member inherits.
workspace_inheritance_moved() {
  local fork="$1" head="$2" manifest table diff names name inherits
  manifest="$(git show "$head:Cargo.toml")"
  table="$(awk '/^\[workspace\.(dependencies|package)\]/{f=1;next} /^\[/{f=0} f && /^[A-Za-z0-9_-]+[. =]/{print $1}' <<<"$manifest" | sed 's/\.workspace$//' | sort -u)"
  diff="$(git diff "$fork" "$head" -- Cargo.toml)"
  names="$({ grep -E '^[+-][A-Za-z0-9_-]+(\.workspace)?[[:space:]]*=' <<<"$diff" || true; } | sed -E 's/^[+-]//; s/[[:space:]]*=.*//; s/\.workspace$//' | sort -u)"
  for name in $names; do
    grep -qx -- "$name" <<<"$table" || continue
    inherits="^${name}(\.workspace[[:space:]]*=[[:space:]]*true|[[:space:]]*=[[:space:]]*\{[^}]*workspace[[:space:]]*=[[:space:]]*true)"
    if git grep -qE "$inherits" "$head" -- 'crates/openehr-*/Cargo.toml'; then
      echo "workspace entry '$name' changed and crates/openehr-* members inherit it: packaged manifests move."
      return 0
    fi
  done
  return 1
}

# Exit 0 when <head> changed packaged content since its merge base with
# <base>, 1 when it did not, 3 when that cannot be judged.
packaged_changed() {
  local base="$1" head="$2" fork changed verdict=0
  fork="$(change_fork "$base" "$head")" || return 3
  changed="$(change_paths "$base" "$head")" || return 3
  printf '%s\n' "$changed" | bash "$PACKAGED_PATHS" || verdict=$?
  case "$verdict" in
  0) return 0 ;;
  1) ;;
  *)
    echo "::error::$PACKAGED_PATHS failed (exit $verdict)." >&2
    return 3
    ;;
  esac
  if grep -qx 'Cargo.toml' <<<"$changed"; then
    workspace_inheritance_moved "$fork" "$head" && return 0
  fi
  return 1
}

guard() {
  local base="$1" head="$2" fork c path before after verdict=0
  fork="$(change_fork "$base" "$head")" || refuse "no merge base between $base and $head."
  for c in $CRATES; do
    path="crates/openehr-$c/Cargo.toml"
    before="$(manifest_version "$(manifest_at "$fork" "$path")")"
    after="$(manifest_version "$(manifest_at "$head" "$path")")"
    # A member new in this change has no version to keep.
    [[ -z "$before" || "$before" == "$after" ]] || refuse "This change moves crates/openehr-$c from $before to ${after:-nothing}. The lockstep version is set by the publish step (scripts/release/publish-crates.sh), never in a pull request: revert the version edit (.claude/rules/crates-publishing.md). A pull request that must set it by hand carries the 'no-crate-bump' label."
  done
  [[ "$(internal_requirements "$fork")" == "$(internal_requirements "$head")" ]] || refuse "This change edits an internal 'path = \"../openehr-*\", version = \"…\"' requirement. The publish step stamps those with the lockstep version: revert the edit (.claude/rules/crates-publishing.md)."
  packaged_changed "$base" "$head" || verdict=$?
  case "$verdict" in
  0) echo "Packaged crate content changed: the next publish stamps a new lockstep version. Nothing to bump here." ;;
  1) echo "No packaged crate content touched." ;;
  *) refuse "could not judge the packaged content of $base..$head." ;;
  esac
}

# --- self-test -------------------------------------------------------------

# Write the nine manifests at a version, with one internal requirement each
# (openehr-rm's at <req>, default the version) and two inherited entries.
_write_manifests() {
  local ver="$1" req="${2:-$1}" c dep
  for c in $CRATES; do
    mkdir -p "crates/openehr-$c/src" "crates/openehr-$c/tests"
    dep="$ver"
    [[ "$c" == rm ]] && dep="$req"
    printf '[package]\nname = "openehr-%s"\nversion = "%s"\nrust-version.workspace = true\n\n[dependencies]\nserde.workspace = true\n' "$c" "$ver" >"crates/openehr-$c/Cargo.toml"
    [[ "$c" == base ]] || printf 'openehr-base = { path = "../openehr-base", version = "%s" }\n' "$dep" >>"crates/openehr-$c/Cargo.toml"
  done
}

# A root manifest whose [workspace.dependencies] table opens with serde and
# then runs past a pipe buffer, so a reader stopping at serde would kill its
# writer. The [workspace.package] table carries the product version, which no
# member inherits, and the inherited rust-version.
_write_root_manifest() {
  local serde="$1" product="${2:-1.0.0}" msrv="${3:-1.90}" i
  {
    printf '[workspace]\nmembers = ["crates/*"]\n\n[workspace.package]\nversion = "%s"\nrust-version = "%s"\n\n[workspace.dependencies]\n' "$product" "$msrv"
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

# <label> <want exit> <setup function> [mode]: branch from main, apply the
# setup, commit, and run the guard on main..branch (mode `changed` asks the
# publish step's question instead).
_case() {
  local label="$1" want="$2" setup="$3" mode="${4:-}" got=0
  git checkout -q -b "case-$setup-${mode:-pr}" main
  "$setup"
  _commit "$label"
  if [[ "$mode" == changed ]]; then
    bash "$SELF" changed main HEAD >/dev/null 2>&1 || got=$?
  else
    bash "$SELF" main HEAD >/dev/null 2>&1 || got=$?
  fi
  git checkout -q main
  if [[ "$got" -ne "$want" ]]; then
    echo "self-test: $label: expected exit $want, got $got" >&2
    return 1
  fi
  echo "  self-test: $label: exit $got"
}

_src_change() { echo "// changed" >>crates/openehr-adl/src/lib.rs; }
_bumped() { _src_change; _write_manifests 0.0.2; }
_one_crate_moved() {
  _src_change
  sed -i.bak 's/^version = "0.0.1"/version = "0.0.2"/' crates/openehr-sdt/Cargo.toml
  rm crates/openehr-sdt/Cargo.toml.bak
}
_requirement_moved() { _src_change; _write_manifests 0.0.1 0.0.2; }
_tests_only() { echo "// test" >>crates/openehr-adl/tests/it.rs; }
_workspace_dependency() { _write_root_manifest 1.0.1; }
_workspace_package() { _write_root_manifest 1.0.0 1.0.0 1.91; }
_product_version() { _write_root_manifest 1.0.0 1.0.1; }
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
  for c in $CRATES; do
    echo "// lib" >"crates/openehr-$c/src/lib.rs"
    echo "// it" >"crates/openehr-$c/tests/it.rs"
  done
  _commit fork
  # The pull-request rule: the content changes, the version stays.
  _case "a packaged change without a version edit passes" 0 _src_change || failures=1
  _case "a lockstep bump in a pull request is refused" 1 _bumped || failures=1
  _case "one of the nine moved by hand is refused" 1 _one_crate_moved || failures=1
  _case "an internal requirement moved by hand is refused" 1 _requirement_moved || failures=1
  _case "a tests-only change passes" 0 _tests_only || failures=1
  _case "a consumed workspace dependency moved passes (manifest past a pipe buffer)" 0 _workspace_dependency || failures=1
  # The publish step's question.
  _case "a source change is packaged content" 0 _src_change changed || failures=1
  _case "a tests-only change is not packaged content" 1 _tests_only changed || failures=1
  _case "a consumed workspace dependency moved is packaged content (manifest past a pipe buffer)" 0 _workspace_dependency changed || failures=1
  _case "an inherited [workspace.package] entry moved is packaged content" 0 _workspace_package changed || failures=1
  _case "the product version no member inherits is not packaged content" 1 _product_version changed || failures=1
  _case "a workspace dependency moved among paths past a pipe buffer is packaged content" 0 _many_paths changed || failures=1
  [[ "$failures" -eq 0 ]] || return 1
  echo "crate-version-guard: self-test OK."
}

case "${1:-}" in
--self-test)
  [[ "$#" -eq 1 ]] || guard_usage "$USAGE"
  self_test
  ;;
changed)
  [[ "$#" -eq 3 ]] || guard_usage "$USAGE"
  verdict=0
  packaged_changed "$2" "$3" || verdict=$?
  exit "$verdict"
  ;;
-*)
  guard_usage "$USAGE"
  ;;
*)
  [[ "$#" -eq 2 ]] || guard_usage "$USAGE"
  guard "$1" "$2"
  ;;
esac
