#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# What a change itself touched: the paths <head> changed since its merge base
# with <base>, the one diff every pull-request guard reads (#4129).
#
# A two-point diff, `git diff <base> <head>`, also counts what <base> gained
# after the branch forked: a branch behind main is judged on main's changes as
# if they were its own, failing on a file main touched and passing on a
# refresh main carried (#4117). The merge base reads only the branch's own
# commits, and a branch that merged main in still reads only its own.
#
# There is no fallback to the two-point form. Without a merge base (a shallow
# checkout, an unrelated history) the functions fail, and the caller decides,
# most often by treating the change as touching everything.
#
# Source it from a guard as:
#
#   # shellcheck source=scripts/lib/changed-paths.sh
#   . "$(dirname "$0")/../lib/changed-paths.sh"
#
# or run it:
#
#   scripts/lib/changed-paths.sh <base> <head> [pathspec...]
#                                     print the paths, exit 1 without a merge base
#   scripts/lib/changed-paths.sh --self-test
#                                     prove it on branches behind and ahead of main

# change_fork <base> <head>: print the merge base, or say why there is none
# and return 1.
change_fork() {
  local fork
  if ! fork="$(git merge-base "$1" "$2" 2>/dev/null)" || [[ -z "$fork" ]]; then
    echo "changed-paths: no merge base between $1 and $2 (check out with fetch-depth: 0)" >&2
    return 1
  fi
  printf '%s\n' "$fork"
}

# change_paths <base> <head> [pathspec...]: print the paths <head> changed
# since its merge base with <base>, limited to the pathspecs when given.
change_paths() {
  local base="$1" head="$2" fork
  shift 2
  fork="$(change_fork "$base" "$head")" || return 1
  git diff --no-renames --name-only "$fork" "$head" -- "$@"
}

# One commit in the scratch repository: write, or with an empty content
# delete, the file.
_changed_paths_commit() {
  local path="$1" content="$2"
  if [[ -z "$content" ]]; then
    git rm -q "$path"
  else
    mkdir -p "$(dirname "$path")"
    printf '%s\n' "$content" >"$path"
    git add "$path"
  fi
  git -c user.name=self-test -c user.email=self-test@invalid -c commit.gpgsign=false \
    commit -qm "$path"
}

_changed_paths_self_test() {
  local tmp failures=0
  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand $tmp now, while it is still in scope
  trap "rm -rf '$tmp'" RETURN
  (
    set -e
    cd "$tmp"
    git init -q -b main
    _changed_paths_commit kept.txt fork
    _changed_paths_commit gone-on-main.txt fork
    _changed_paths_commit gone-on-branch.txt fork
    git branch fork
    # main moves on: one file added, one changed, one deleted.
    _changed_paths_commit main-added.txt main
    _changed_paths_commit kept.txt main
    _changed_paths_commit gone-on-main.txt ""
    # A branch forked before all of it, so it is behind main.
    git checkout -q -b behind fork
    _changed_paths_commit own.txt branch
    _changed_paths_commit gone-on-branch.txt ""
    # The same branch with main merged in.
    git checkout -q -b merged behind
    git -c user.name=self-test -c user.email=self-test@invalid -c commit.gpgsign=false \
      merge -q --no-edit main
    # A branch with nothing of its own.
    git checkout -q -b empty fork
    # An unrelated history.
    git checkout -q --orphan unrelated
    git rm -rqf .
    _changed_paths_commit alone.txt alone
    git checkout -q main
  ) >/dev/null 2>&1 || {
    echo "self-test: could not build the scratch repository" >&2
    return 1
  }

  local name base head want got pathspec
  while IFS='|' read -r name base head pathspec want; do
    # shellcheck disable=SC2086 # the pathspec column is a word list
    got="$(cd "$tmp" && change_paths "$base" "$head" $pathspec | paste -sd, -)"
    if [[ "$got" != "$want" ]]; then
      echo "self-test: $name: got '$got', want '$want'" >&2
      failures=1
    fi
  done <<'CASES'
a branch behind main reads its own add and delete, none of main's|main|behind||gone-on-branch.txt,own.txt
main behind the branch reads main's own changes, none of the branch's|behind|main||gone-on-main.txt,kept.txt,main-added.txt
a branch that merged main in still reads only its own|main|merged||gone-on-branch.txt,own.txt
a pathspec narrows the paths|main|behind|own.txt|own.txt
a branch with nothing of its own reads nothing|main|empty||
CASES

  if (cd "$tmp" && change_paths main unrelated) >/dev/null 2>&1; then
    echo "self-test: an unrelated history had a merge base" >&2
    failures=1
  fi
  if [[ "$(cd "$tmp" && change_fork main behind)" != "$(cd "$tmp" && git rev-parse fork)" ]]; then
    echo "self-test: change_fork did not name the fork point" >&2
    failures=1
  fi
  [[ "$failures" -eq 0 ]] || return 1
  echo "changed-paths: self-test OK (main's changes ignored on a branch behind it, and the branch's ignored on main)."
}

# Run, not sourced: the command-line form.
if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  set -euo pipefail
  case "${1-}" in
  --self-test)
    [[ "$#" -eq 1 ]] || { echo "usage: $0 <base> <head> [pathspec...] | --self-test" >&2; exit 2; }
    _changed_paths_self_test
    ;;
  -*)
    echo "error: $0: unknown flag $1" >&2
    echo "usage: $0 <base> <head> [pathspec...] | --self-test" >&2
    exit 2
    ;;
  *)
    [[ "$#" -ge 2 ]] || { echo "usage: $0 <base> <head> [pathspec...] | --self-test" >&2; exit 2; }
    change_paths "$@"
    ;;
  esac
fi
