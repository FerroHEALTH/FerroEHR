#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Screenshot freshness (.claude/rules/leptos-ui.md): a change to the viewer UI
# re-captures the published book screenshots in the same pull request.
#
# The paths are read from the merge base of <base> and <head> (`base...head`),
# so only the pull request's own commits are judged. A two-point diff also
# counted what main gained since the branch forked: a branch behind main
# failed on main's viewer changes, and passed on main's screenshot refresh
# (#4117).
#
# The `no-ui-visual-change` label is read by the CI job, not here.
#
# Usage: scripts/checks/ui-screenshot.sh <base> <head> | --self-test
#   <base> <head> → exit 0 when the change needs no capture or carries one,
#                   1 when it touches the viewer UI without one
#   --self-test   → prove the verdict on a branch behind its base, both ways
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"

readonly GRAMMAR='<base> <head> | --self-test'
readonly UI_PATHS='^app/ferroehr-viewer/(src|style)/'
readonly SHOT_PATHS='^website/book/src/viewer/img/'

# The verdict for the commits <head> adds over its merge base with <base>,
# run in the repository of the working directory.
verdict() {
  local base="$1" head="$2" changed
  changed="$(git diff --name-only "$base...$head")"
  if ! grep -E "$UI_PATHS" >/dev/null <<<"$changed"; then
    echo "No viewer UI paths touched: guard not required."
    return 0
  fi
  if grep -E "$SHOT_PATHS" >/dev/null <<<"$changed"; then
    echo "Screenshots refreshed: OK."
    return 0
  fi
  echo "::error::This PR changes the viewer UI but does not refresh website/book/src/viewer/img/ (run scripts/ui-e2e.sh with UI_E2E_DOCS_SHOTS=1 and commit the captures), and does not carry the 'no-ui-visual-change' label." >&2
  return 1
}

# Commit one file change in the scratch repository.
commit_file() {
  mkdir -p "$(dirname "$1")"
  printf '%s\n' "$2" >"$1"
  git add "$1"
  git -c user.name=self-test -c user.email=self-test@invalid -c commit.gpgsign=false commit -qm "$1"
}

# Every case forks a branch from `fork`, then moves main past it, so each
# branch is behind its base the way #4117's was. `base-ui` is main after a UI
# change alone, `base` is main after its capture too.
self_test() {
  local tmp failures=0
  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand $tmp now, while it is still in scope
  trap "rm -rf '$tmp'" RETURN
  (
    cd "$tmp"
    git init -q -b main
    commit_file README.md fork
    git branch fork
    commit_file app/ferroehr-viewer/src/main.rs main-ui
    git branch base-ui
    commit_file website/book/src/viewer/img/main.png main-shot
    git branch base

    git checkout -q -b other fork
    commit_file app/ferroehr/src/lib.rs server
    git checkout -q -b ui-only fork
    commit_file app/ferroehr-viewer/style/app.css ui
    git checkout -q -b ui-shot fork
    commit_file app/ferroehr-viewer/src/page.rs ui
    commit_file website/book/src/viewer/img/page.png shot
  ) >/dev/null

  local case base branch want got
  for case in base-ui:other:0 base:other:0 base:ui-only:1 base:ui-shot:0; do
    IFS=: read -r base branch want <<<"$case"
    got=0
    (cd "$tmp" && verdict "$base" "$branch") >/dev/null 2>&1 || got=1
    [[ "$got" == "$want" ]] || {
      echo "self-test: branch $branch behind $base exited $got, want $want" >&2
      failures=1
    }
  done
  [[ "$failures" -eq 0 ]] || return 1
  echo "ui-screenshot: self-test OK (main's own UI change and capture ignored on a branch behind it)."
}

case "${1-}" in
--self-test)
  [[ "$#" -eq 1 ]] || guard_usage "$GRAMMAR"
  self_test
  ;;
-*) guard_known_flags "$GRAMMAR" "--self-test" "$@" ;;
*)
  [[ "$#" -eq 2 ]] || guard_usage "$GRAMMAR"
  verdict "$1" "$2"
  ;;
esac
