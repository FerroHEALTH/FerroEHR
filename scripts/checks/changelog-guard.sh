#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The changelog guard (no specification governs it: our own design): a pull
# request that changes a user-visible surface records its effect in the
# changelog, as a fragment under changelog.d/ (any file there but the README).
# scripts/release/changelog.sh --check judges the fragment's form; this guard
# asks whether there is one, and refuses a change that writes an entry under
# [Unreleased] in CHANGELOG.md, since that section is what two open pull
# requests used to conflict on.
#
#   changelog-guard.sh [--waived] <base-ref> [head-ref]
#   changelog-guard.sh --self-test
#
# The user-visible surfaces are app/, crates/, deploy/, docker/, the compose
# files and the Cargo manifests; a change that touches none needs no entry. An
# edit of CHANGELOG.md outside [Unreleased] (the release cut, or a correction
# of a released section) counts as the change's record. --waived is the CI
# form when the `no-changelog` label is set: the entry is not required, and an
# entry written under [Unreleased] is still refused. This script reads no
# labels.
#
# The change is what the head added since it forked from the base, read from
# the diff against the merge base of the two, so a fragment that reached the
# base after the fork is never counted as the change's own.
#
# Exit 0 when the change passes, 1 when it carries no entry or writes under
# [Unreleased], and 2 on a usage error or a base and head with no merge base.
set -euo pipefail

readonly SURFACES='^(app/|crates/|deploy/|docker/|docker-compose|Cargo\.toml|Cargo\.lock)'

# The self-test builds a stub repository in a temporary directory, with a copy
# of this script in it, and runs the guard there as CI does: the base is
# main's tip and the head is the branch.
self_test() {
  local script status
  # Global, so the EXIT trap still sees it after the function returns.
  work="$(mktemp -d)"
  # git writes its objects read-only, so the tree is made writable before it
  # is removed.
  trap 'chmod -R u+w "$work" && rm -r -- "$work"' EXIT
  script="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
  mkdir -p "$work/scripts/checks" "$work/changelog.d" "$work/app" "$work/docs"
  cp "$script" "$work/scripts/checks/changelog-guard.sh"

  # A repository of its own, so no global hook, signing key or identity of the
  # caller's reaches it.
  stub_git() {
    git -C "$work" -c user.name=self-test -c user.email=self-test@example.org \
      -c commit.gpgsign=false -c core.hooksPath=/dev/null "$@"
  }
  commit() {
    local message="$1"
    stub_git add -A
    stub_git commit -q -m "$message"
  }
  branch() {
    local name="$1"
    stub_git checkout -q main
    stub_git checkout -q -b "$name"
  }
  code() { echo 'more code' >> "$work/app/code.txt"; }
  # unreleased TEXT: CHANGELOG.md with TEXT as the body of [Unreleased].
  unreleased() {
    printf '# Changelog\n\n## [Unreleased]\n%s\n## [1.0.0] - 2026-10-01\n\n### Added\n\n- The first release.\n\n[unreleased]: https://example.org/r/compare/v1.0.0...HEAD\n' \
      "$1" > "$work/CHANGELOG.md"
  }
  # expect NAME WANT BRANCH [FLAG]: the guard over BRANCH against main exits
  # WANT.
  expect() {
    local name=$1 want=$2 branch=$3 flag=${4:-}
    status=0
    (cd "$work" && bash scripts/checks/changelog-guard.sh ${flag:+"$flag"} "${base_ref:-main}" "$branch") > "$work/out" 2>&1 || status=$?
    if [[ "$status" -ne "$want" ]]; then
      echo "changelog-guard: self-test failed: $name exited $status, wanted $want." >&2
      cat "$work/out" >&2
      exit 1
    fi
  }
  says() {
    local name=$1 text=$2
    grep -q "$text" "$work/out" || {
      echo "changelog-guard: self-test failed: $name did not say \"$text\"." >&2
      cat "$work/out" >&2
      exit 1
    }
  }

  stub_git init -q -b main
  unreleased ''
  printf 'The fragment format.\n' > "$work/changelog.d/README.md"
  printf -- '- An entry already released.\n' > "$work/changelog.d/1-old.added.md"
  echo 'code' > "$work/app/code.txt"
  echo 'a page' > "$work/docs/page.md"
  commit "the fork point"

  branch fragment
  code
  printf -- '- An entry.\n' > "$work/changelog.d/2-new.added.md"
  commit "a change with a fragment"
  branch unreleased
  code
  printf -- '- An entry.\n' > "$work/changelog.d/2-new.added.md"
  unreleased $'\n### Added\n\n- An entry written into CHANGELOG.md.\n'
  commit "a change with a fragment that also writes under [Unreleased]"
  branch cut
  code
  stub_git rm -q -- changelog.d/1-old.added.md
  printf '# Changelog\n\n## [Unreleased]\n\n## [1.1.0] - 2026-10-02\n\n### Added\n\n- An entry already released.\n\n## [1.0.0] - 2026-10-01\n\n### Added\n\n- The first release.\n\n[unreleased]: https://example.org/r/compare/v1.1.0...HEAD\n' \
    > "$work/CHANGELOG.md"
  commit "the release cut"
  branch none
  code
  commit "a change with no entry"
  branch invisible
  echo 'a clearer page' >> "$work/docs/page.md"
  commit "a change to no user-visible surface"
  branch readme
  code
  echo 'A clearer note.' >> "$work/changelog.d/README.md"
  commit "a change to the README alone"
  branch reworded
  code
  printf -- '- An entry, reworded.\n' > "$work/changelog.d/1-old.added.md"
  commit "a change that rewords a fragment it did not add"
  branch removed
  code
  stub_git rm -q -- changelog.d/1-old.added.md
  commit "a change that removes a fragment"
  branch moved
  unreleased $'\n### Fixed\n\n- An entry a change moves out.\n'
  commit "an entry under [Unreleased]"
  stub_git checkout -q -b moved-out
  code
  unreleased ''
  printf -- '- An entry a change moves out.\n' > "$work/changelog.d/3-moved.fixed.md"
  commit "the entry moves into a fragment"

  stub_git checkout -q main
  printf -- '- An entry from main.\n' > "$work/changelog.d/4-main.fixed.md"
  commit "main gains a fragment after every branch forked"

  expect "a change that adds a fragment" 0 fragment
  expect "a change that writes under [Unreleased]" 1 unreleased
  says "the [Unreleased] refusal" 'writes under \[Unreleased\]'
  expect "a waived change that writes under [Unreleased]" 1 unreleased --waived
  expect "the release cut" 0 cut
  expect "a change with no entry, behind a main that gained a fragment" 1 none
  says "the refusal" 'no changelog entry'
  expect "a waived change with no entry" 0 none --waived
  expect "a change to no user-visible surface" 0 invisible
  expect "a change to the README alone" 1 readme
  expect "a change that rewords a fragment it did not add" 1 reworded
  expect "a change that removes a fragment" 1 removed
  base_ref=moved expect "a change that moves an [Unreleased] entry into a fragment" 0 moved-out
  says "the moved entry" "adds the fragment"
  echo "changelog-guard: self-test passed."
}

if [[ "${1:-}" = "--self-test" && $# -eq 1 ]]; then
  self_test
  exit 0
fi

cd "$(dirname "$0")/../.."

waived=0
if [[ "${1:-}" = "--waived" ]]; then
  waived=1
  shift
fi
if [[ $# -lt 1 || $# -gt 2 || -z "${1:-}" || "${1:-}" = -* ]]; then
  echo "usage: changelog-guard.sh [--waived] <base-ref> [head-ref] | --self-test" >&2
  exit 2
fi
base="$1"
head="${2:-HEAD}"

if ! fork="$(git merge-base "$base" "$head")"; then
  echo "changelog-guard: $base and $head have no merge base; check out the full history (fetch-depth: 0)." >&2
  exit 2
fi

# The lines the change adds to the [Unreleased] body of the head's
# CHANGELOG.md: the body runs from its heading to the next version heading or
# the link reference block, and a blank line is no entry.
written=""
if git cat-file -e "$head:CHANGELOG.md" 2> /dev/null; then
  read -r first last < <(git show "$head:CHANGELOG.md" | awk '
    !start && /^## \[Unreleased\][[:space:]]*$/ { start = NR; next }
    start && (/^## \[/ || /^\[[^]]+\]: /) { print start + 1, NR - 1; found = 1; exit }
    END { if (!found) print (start ? start + 1 : 0), (start ? NR : -1) }
  ')
  written="$(git diff --no-renames -U0 "$fork" "$head" -- CHANGELOG.md | awk -v first="$first" -v last="$last" '
    /^@@ / {
      split($3, range, ",")
      line = substr(range[1], 2) + 0
      next
    }
    /^\+\+\+ / { next }
    /^\+/ {
      if (line >= first && line <= last && $0 ~ /[^[:space:]]/) print "  CHANGELOG.md:" line ": " substr($0, 2)
      line++
    }
  ')"
fi
if [[ -n "$written" ]]; then
  echo "::error::the change writes under [Unreleased] in CHANGELOG.md; move each entry into a fragment, changelog.d/<issue>-<kebab-slug>.<section>.md, as changelog.d/README.md describes:" >&2
  echo "$written" >&2
  exit 1
fi

if [[ "$waived" -eq 1 ]]; then
  echo "changelog-guard: waived; the change writes nothing under [Unreleased]."
  exit 0
fi

if ! git diff --no-renames --name-only "$fork" "$head" | grep -qE "$SURFACES"; then
  echo "changelog-guard: the change touches no user-visible surface; no entry is required."
  exit 0
fi

# A rename is read as a removal and an addition, so a fragment renamed into
# place counts as added and one renamed away does not.
added="$(git diff --no-renames --name-only --diff-filter=A "$fork" "$head" -- 'changelog.d/*' ':!changelog.d/README.md')"
if [[ -n "$added" ]]; then
  echo "changelog-guard: the change adds the fragment(s):"
  while IFS= read -r path; do echo "  $path"; done <<<"$added"
  exit 0
fi
if [[ -n "$(git diff --name-only "$fork" "$head" -- CHANGELOG.md)" ]]; then
  echo "changelog-guard: the change edits CHANGELOG.md outside [Unreleased]."
  exit 0
fi
echo "::error::no changelog entry: the change touches a user-visible surface (app/, crates/, deploy/, docker/, the compose files, the Cargo manifests); add a fragment, changelog.d/<issue>-<kebab-slug>.<section>.md, as changelog.d/README.md describes, or apply the 'no-changelog' label when the change has no user-visible effect." >&2
exit 1
