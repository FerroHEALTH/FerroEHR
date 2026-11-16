#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Test-module guard: every `.rs` file under a `tests/it/` directory is a module
# its `main.rs` reaches (#5765).
#
# Each crate's integration tests are one binary, `tests/it/main.rs`, which
# compiles only the files its `mod` declarations name
# (.claude/rules/testing.md, one integration-test binary per crate). A file it
# never declares compiles nowhere, so its tests never run and nothing says so:
# `app/ferroehr-rest/tests/it/pmir_feed_http.rs` sat unregistered with a broken
# fixture (#5764). The guard follows `mod name;` declarations, a `#[path]`
# attribute included, from `main.rs` through `name.rs` and `name/mod.rs`
# (the Rust Reference, Modules, "Module Source Filenames"), and refuses every
# file it does not reach. No openEHR spec governs test layout: our own design.
#
# Usage: scripts/checks/test-modules.sh [--self-test | <root>]
#   no args     → every tests/it/ directory of this tree
#   <root>      → every tests/it/ directory under <root>
#   --self-test → runs the guard over fixtures with an unregistered file, a
#                 nested unregistered file and a clean tree, and asserts the
#                 first two are refused and the third accepted.
set -euo pipefail
export LC_ALL=C

# Appends to `queue` the files `file` declares as modules, whether or not
# they exist.
declared() {
  local file="$1" base dir stem line name path=""
  base="${file%/*}"
  stem="${file##*/}"
  stem="${stem%.rs}"
  case "$stem" in
    main | mod | lib) dir="$base" ;;
    *) dir="$base/$stem" ;;
  esac
  while IFS= read -r line; do
    if [[ "$line" =~ ^[[:space:]]*#\[path[[:space:]]*=[[:space:]]*\"([^\"]+)\"\][[:space:]]*$ ]]; then
      path="${BASH_REMATCH[1]}"
      continue
    fi
    if [[ "$line" =~ ^[[:space:]]*(pub(\([a-z:_ ]+\))?[[:space:]]+)?mod[[:space:]]+([A-Za-z_][A-Za-z0-9_]*)[[:space:]]*\;[[:space:]]*$ ]]; then
      name="${BASH_REMATCH[3]}"
      if [[ -n "$path" ]]; then
        queue+=("$base/$path")
      elif [[ -f "$dir/$name/mod.rs" ]]; then
        queue+=("$dir/$name/mod.rs")
      else
        queue+=("$dir/$name.rs")
      fi
    fi
    # NOTE: an attribute applies to the next item only, so any other line clears a pending path.
    [[ "$line" =~ ^[[:space:]]*#\[ ]] || path=""
  done <"$file"
}

# Sets `normalized` to `path` with every `.` and `name/..` segment folded
# away.
normal() {
  local part
  local -a out=() parts=()
  IFS='/' read -r -a parts <<<"$1"
  for part in "${parts[@]}"; do
    case "$part" in
      . | '') ;;
      ..) ((${#out[@]})) && unset 'out[${#out[@]}-1]' ;;
      *) out+=("$part") ;;
    esac
  done
  local IFS='/'
  normalized="${1%%[^/]*}${out[*]}"
}

# The files under `it` that `it/main.rs` does not reach, one per line.
unreached() {
  local it="$1" file normalized head=0
  queue=("$it/main.rs")
  local -A reached=()
  while ((head < ${#queue[@]})); do
    normal "${queue[head]}"
    head=$((head + 1))
    file="$normalized"
    [[ -f "$file" && -z "${reached[$file]:-}" ]] || continue
    reached[$file]=1
    declared "$file"
  done
  while IFS= read -r file; do
    [[ -n "${reached[$file]:-}" ]] || echo "$file"
  done < <(find "$it" -type f -name '*.rs' | sort)
}

if [[ "${1:-}" == "--self-test" ]]; then
  self_root="$(mktemp -d)"
  trap 'rm -rf "$self_root"' EXIT
  self_fail=0
  # <label>|<expected exit: 0 accept, 1 reject>|<files: path=content@…>
  self_cases=(
    "a clean tree|0|main.rs=mod a;\nmod b;@a.rs=@b/mod.rs=mod c;@b/c.rs="
    "an unregistered file|1|main.rs=mod a;@a.rs=@stray.rs="
    "an unregistered nested file|1|main.rs=pub(crate) mod a;@a.rs=mod b;@a/b.rs=@a/c.rs="
    "a path attribute|0|main.rs=#[path = \"support/x.rs\"]\nmod x;@support/x.rs="
  )
  for self_case in "${self_cases[@]}"; do
    label="${self_case%%|*}"
    rest="${self_case#*|}"
    want="${rest%%|*}"
    files="${rest#*|}"
    rm -rf "$self_root/crate"
    mkdir -p "$self_root/crate/tests/it"
    IFS='@' read -r -a entries <<<"$files"
    for entry in "${entries[@]}"; do
      path="${entry%%=*}"
      mkdir -p "$(dirname "$self_root/crate/tests/it/$path")"
      printf '%b\n' "${entry#*=}" >"$self_root/crate/tests/it/$path"
    done
    got=0
    bash "$0" "$self_root" >/dev/null 2>&1 || got=1
    if [[ "$got" != "$want" ]]; then
      echo "::error::--self-test: ${label}: expected exit ${want}, got ${got}." >&2
      self_fail=1
    fi
  done
  if ((self_fail)); then
    exit 1
  fi
  echo "test-modules: --self-test passed (${#self_cases[@]} cases)."
  exit 0
fi

root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
status=0
count=0
while IFS= read -r it; do
  count=$((count + 1))
  if [[ ! -f "$it/main.rs" ]]; then
    echo "::error::test-modules: ${it#"$root"/} has no main.rs." >&2
    status=1
    continue
  fi
  while IFS= read -r stray; do
    echo "::error file=${stray#"$root"/}::test-modules: ${stray#"$root"/} is no module of ${it#"$root"/}/main.rs, so its tests never compile or run; declare it with \`mod\`." >&2
    status=1
  done < <(unreached "$it")
done < <(cd "$root" && find . \( -name target -o -name .claude -o -name node_modules -o -name .git \) -prune \
  -o -type d -path '*/tests/it' -print | sed "s#^\\.#$root#" | sort)
if ((status == 0)); then
  echo "test-modules: every file of ${count} tests/it/ trees is reachable from its main.rs."
fi
exit "$status"
