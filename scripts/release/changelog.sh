#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# Changelog fragments (no specification governs this: our own design). Every
# change with a user-visible effect adds one file under changelog.d/, so no two
# pull requests edit the same lines of CHANGELOG.md, and the release cut
# assembles the fragments into CHANGELOG.md, which follows Keep a Changelog
# 1.1.0 (https://keepachangelog.com/en/1.1.0/).
#
# A fragment is changelog.d/<issue>-<kebab-slug>.<section>.md, where <section>
# is added, changed, deprecated, removed, fixed or security, and its content is
# one or more Markdown list items exactly as they read in CHANGELOG.md: a line
# that opens with "- ", continued by lines indented by at least two spaces.
# changelog.d/README.md is the format note and is never read as a fragment.
#
# One more fragment kind, <issue>-<kebab-slug>.security-reason.md, carries the
# line a release's Security section opens with when it ships beside functional
# changes: "Shipped with functional changes because: <reason>" (CRA Annex I
# Part II(2), .claude/rules/changelog.md). It is one paragraph, at most one
# such fragment exists, and the cut places it first under "### Security".
#
#   changelog.sh --check
#       Validates every fragment's name and content. Exit 0 when every one
#       holds, 1 otherwise, naming each defect.
#   changelog.sh --assemble <version> <date>
#       Writes a new "## [<version>] - <date>" section under a fresh, empty
#       [Unreleased], holding the entries still under [Unreleased] followed by
#       the fragments, section by section in the Keep a Changelog order, as a
#       loose list (a blank line between entries); moves the [Unreleased] link
#       reference on and adds the version's; then `git rm`s the fragments.
#       Exit 1 on any defect, with nothing written.
#   changelog.sh --preview
#       Prints CHANGELOG.md with every fragment folded into [Unreleased], as the
#       next cut would write that section, and writes nothing.
#   changelog.sh --check-preview
#       Runs scripts/checks/changelog-structure.sh and
#       scripts/checks/changelog-security.sh over the preview, so the release
#       section's rules hold on every pull request, not first at the cut.
#   changelog.sh --self-test
#       Proves every mode over a stub repository.
#
# The output is deterministic: fragments are read by section in the order
# above, then by file name in the C locale. Exit 2 on a usage error.
set -euo pipefail
export LC_ALL=C

# The Keep a Changelog 1.1.0 sections, in the order a release lists them.
readonly SECTIONS=(added changed deprecated removed fixed security)
readonly FRAGMENT_DIR=changelog.d
readonly NAME_RE='^[0-9]+-[a-z0-9]+(-[a-z0-9]+)*\.(added|changed|deprecated|removed|fixed|security|security-reason)\.md$'
readonly REASON_PREFIX="Shipped with functional changes because: "

die() {
  echo "changelog: $*" >&2
  exit 1
}

usage() {
  echo "usage: changelog.sh --check | --assemble <version> <date> | --preview | --check-preview | --self-test" >&2
  exit 2
}

# title SECTION: the heading a section carries in CHANGELOG.md.
title() {
  local section="$1"
  printf '%s%s' "$(tr '[:lower:]' '[:upper:]' <<<"${section:0:1}")" "${section:1}"
}

# content_defect FILE: prints why FILE is no list item, or nothing when it is.
content_defect() {
  local file="$1"
  awk '
    { lines[NR] = $0 }
    END {
      last = NR
      while (last > 0 && lines[last] ~ /^[[:space:]]*$/) last--
      if (last == 0) { print "it is empty"; exit }
      if (lines[1] !~ /^- [^[:space:]]/) { print "line 1 does not open a list item with \"- \""; exit }
      for (i = 2; i <= last; i++) {
        if (lines[i] ~ /^- [^[:space:]]/ || lines[i] ~ /^  +[^[:space:]]/) continue
        if (lines[i] ~ /^[[:space:]]*$/) { print "line " i " is blank inside the entry"; exit }
        print "line " i " neither opens a list item nor continues one indented by two spaces"
        exit
      }
    }
  ' "$file"
}

# reason_defect FILE: prints why FILE is no reason paragraph, or nothing when
# it is one.
reason_defect() {
  local file="$1"
  awk -v prefix="$REASON_PREFIX" '
    { lines[NR] = $0 }
    END {
      last = NR
      while (last > 0 && lines[last] ~ /^[[:space:]]*$/) last--
      if (last == 0) { print "it is empty"; exit }
      if (index(lines[1], prefix) != 1 || lines[1] == prefix) {
        print "line 1 does not open with \"" prefix "<reason>\""
        exit
      }
      for (i = 2; i <= last; i++) {
        if (lines[i] ~ /^[[:space:]]*$/) { print "line " i " is blank inside the reason"; exit }
        if (lines[i] ~ /^[-*+] /) { print "line " i " opens a list item; the reason is one paragraph"; exit }
      }
    }
  ' "$file"
}

# check: every fragment's name and content, every defect named.
check() {
  local fail=0 count=0 reasons=0 path name defect
  [[ -d "$FRAGMENT_DIR" ]] || die "$FRAGMENT_DIR/ is missing"
  for path in "$FRAGMENT_DIR"/* "$FRAGMENT_DIR"/.[!.]*; do
    [[ -e "$path" ]] || continue
    name="${path#"$FRAGMENT_DIR"/}"
    [[ "$name" != README.md ]] || continue
    if [[ ! -f "$path" ]]; then
      echo "changelog: $path is not a file; a fragment is one file directly under $FRAGMENT_DIR/." >&2
      fail=1
      continue
    fi
    if [[ ! "$name" =~ $NAME_RE ]]; then
      echo "changelog: $path is not named <issue>-<kebab-slug>.<section>.md with <section> one of ${SECTIONS[*]}." >&2
      fail=1
      continue
    fi
    if [[ "$name" == *.security-reason.md ]]; then
      reasons=$((reasons + 1))
      defect="$(reason_defect "$path")"
      if [[ -n "$defect" ]]; then
        echo "changelog: $path is not a reason paragraph: $defect." >&2
        fail=1
        continue
      fi
      count=$((count + 1))
      continue
    fi
    defect="$(content_defect "$path")"
    if [[ -n "$defect" ]]; then
      echo "changelog: $path is not a Markdown list item: $defect." >&2
      fail=1
      continue
    fi
    count=$((count + 1))
  done
  if [[ "$reasons" -gt 1 ]]; then
    echo "changelog: $FRAGMENT_DIR/ holds $reasons *.security-reason.md fragments; a release's Security section opens with one reason, so merge them." >&2
    fail=1
  fi
  [[ "$fail" -eq 0 ]] || exit 1
  echo "changelog: $count fragment(s) under $FRAGMENT_DIR/, every one well formed."
}

# assemble VERSION DATE [preview]: the cut, written to CHANGELOG.md in one
# move. With "preview" it prints the file with the fragments folded into
# [Unreleased] instead, and touches nothing.
assemble() {
  local version="$1" date="$2" mode="${3:-cut}" work start end total section file prev base out
  if [[ "$mode" == cut ]]; then
    [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || die "'$version' is not a version such as 0.0.9 or 0.0.9-rc.1"
    [[ "$date" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || die "'$date' is not a date written YYYY-MM-DD"
  fi
  [[ -f CHANGELOG.md ]] || die "CHANGELOG.md is missing"
  check > /dev/null

  if [[ "$mode" == cut ]] && awk -v heading="## [$version]" 'index($0, heading) == 1 { found = 1 } END { exit !found }' CHANGELOG.md; then
    die "CHANGELOG.md already has a [$version] section"
  fi
  start="$(grep -n -m1 -E '^## \[Unreleased\][[:space:]]*$' CHANGELOG.md | cut -d: -f1 || true)"
  [[ -n "$start" ]] || die "CHANGELOG.md has no '## [Unreleased]' heading"
  total="$(wc -l < CHANGELOG.md | tr -d ' ')"
  # The [Unreleased] body ends at the next version heading or at the link
  # reference block, whichever comes first.
  end="$(awk -v s="$start" 'NR > s && (/^## \[/ || /^\[[^]]+\]: /) { print NR - 1; exit }' CHANGELOG.md)"
  [[ -n "$end" ]] || end="$total"

  # The link reference label matches in either case (CHANGELOG.md writes
  # "[unreleased]"), and the cut keeps the spelling it found.
  label="$(sed -nE 's|^\[([Uu]nreleased)\]: .*/compare/[^/]+\.\.\.HEAD$|\1|p' CHANGELOG.md)"
  prev="$(sed -nE 's|^\[[Uu]nreleased\]: (.*)/compare/([^/]+)\.\.\.HEAD$|\2|p' CHANGELOG.md)"
  base="$(sed -nE 's|^\[[Uu]nreleased\]: (.*)/compare/([^/]+)\.\.\.HEAD$|\1|p' CHANGELOG.md)"
  [[ "$mode" == preview || (-n "$prev" && -n "$base") ]] || die "CHANGELOG.md has no '[Unreleased]: <repository>/compare/<tag>...HEAD' link reference"

  work="$(mktemp -d)"
  # Global, so the EXIT trap still sees it after the function returns.
  assemble_work="$work"
  trap 'rm -r -- "$assemble_work"' EXIT

  # The entries still under [Unreleased], one file per section. Anything
  # outside a known section heading is refused, never dropped.
  sed -n "$((start + 1)),${end}p" CHANGELOG.md | awk -v dir="$work" '
    /^### / {
      heading = substr($0, 5)
      sub(/[[:space:]]+$/, "", heading)
      section = tolower(heading)
      if (section !~ /^(added|changed|deprecated|removed|fixed|security)$/) {
        print "changelog: [Unreleased] has a section \"" heading "\", which Keep a Changelog 1.1.0 does not define." > "/dev/stderr"
        exit 1
      }
      next
    }
    section == "" && /[^[:space:]]/ {
      print "changelog: [Unreleased] holds text outside a section heading: " $0 > "/dev/stderr"
      exit 1
    }
    section != "" { print > (dir "/old." section) }
  ' || exit 1

  # Each section of the release: the [Unreleased] entries first, in their
  # order, then the fragments by file name. The Security section opens with
  # the reason fragment, when there is one.
  local fragments=() entries=0 reason=""
  for file in "$FRAGMENT_DIR"/*.security-reason.md; do
    [[ -e "$file" ]] || continue
    reason="$file"
  done
  for section in "${SECTIONS[@]}"; do
    {
      if [[ "$section" == security && -n "$reason" ]]; then
        if [[ -f "$work/old.security" ]] && grep -qF "$REASON_PREFIX" <<<"$(trim "$work/old.security" | sed -n 1p)"; then
          die "[Unreleased] already opens its Security section with a reason, and $reason gives another; keep one"
        fi
        tracked "$reason" "$mode"
        fragments+=("$reason")
        trim "$reason"
        printf '\n'
      fi
      [[ ! -f "$work/old.$section" ]] || trim "$work/old.$section"
      for file in "$FRAGMENT_DIR"/*."$section".md; do
        [[ -e "$file" ]] || continue
        tracked "$file" "$mode"
        fragments+=("$file")
        trim "$file" | name_issue "$file"
      done
    } > "$work/new.$section.tight"
    loosen "$work/new.$section.tight" > "$work/new.$section"
    [[ ! -s "$work/new.$section" ]] || entries=$((entries + 1))
  done
  if [[ -n "$reason" ]] && ! grep -q '^- ' "$work/new.security"; then
    die "$reason gives a reason for a Security section that holds no entry"
  fi

  if [[ "$mode" == preview ]]; then
    sed -n "1,${start}p" CHANGELOG.md
    for section in "${SECTIONS[@]}"; do
      [[ -s "$work/new.$section" ]] || continue
      printf '\n### %s\n\n' "$(title "$section")"
      cat "$work/new.$section"
    done
    printf '\n'
    [[ "$end" -ge "$total" ]] || sed -n "$((end + 1)),${total}p" CHANGELOG.md
    return 0
  fi
  [[ "$entries" -gt 0 ]] || die "[$version] would be empty: no fragment under $FRAGMENT_DIR/ and no entry under [Unreleased]"

  out="$work/CHANGELOG.md"
  {
    sed -n "1,${start}p" CHANGELOG.md
    printf '\n## [%s] - %s\n' "$version" "$date"
    for section in "${SECTIONS[@]}"; do
      [[ -s "$work/new.$section" ]] || continue
      printf '\n### %s\n\n' "$(title "$section")"
      cat "$work/new.$section"
    done
    printf '\n'
    if [[ "$end" -lt "$total" ]]; then
      sed -n "$((end + 1)),${total}p" CHANGELOG.md | awk -v ver="$version" -v prev="$prev" -v base="$base" -v label="$label" '
        /^\[[Uu]nreleased\]: / {
          print "[" label "]: " base "/compare/v" ver "...HEAD"
          print "[" ver "]: " base "/compare/" prev "...v" ver
          next
        }
        { print }
      '
    fi
  } > "$out"

  cat "$out" > CHANGELOG.md
  if [[ "${#fragments[@]}" -gt 0 ]]; then
    git rm -q -- "${fragments[@]}"
  fi
  echo "changelog: [$version] - $date assembled from ${#fragments[@]} fragment(s) and the [Unreleased] entries; review CHANGELOG.md and commit."
}

# tracked FILE MODE: refuses an untracked fragment at the cut; the preview reads
# the working tree as it stands.
tracked() {
  local file="$1" mode="$2"
  [[ "$mode" == preview ]] && return 0
  git ls-files --error-unmatch -- "$file" > /dev/null 2>&1 || die "$file is not tracked by git; commit it before the cut"
}

# check_preview: the release-section checks over the preview, written beside a
# link to security/ so a VEX path an entry names resolves as it does at the cut.
check_preview() {
  local dir rc=0
  dir="$(mktemp -d)"
  check_preview_dir="$dir"
  trap 'rm -r -- "$check_preview_dir"' EXIT
  (assemble "" "" preview) > "$dir/CHANGELOG.md" || return 1
  [[ ! -d security ]] || ln -s "$PWD/security" "$dir/security"
  bash scripts/checks/changelog-structure.sh "$dir/CHANGELOG.md" || rc=1
  CHANGELOG_RULE="${CHANGELOG_RULE:-.claude/rules/changelog.md, and changelog.d/README.md for the fragments}" \
    bash scripts/checks/changelog-security.sh --section Unreleased "$dir/CHANGELOG.md" || rc=1
  return "$rc"
}

# loosen FILE: FILE with one blank line before every list item that does not
# already follow one, the loose list every section of CHANGELOG.md is written
# in, so a fragment keeps the tight form changelog.d/README.md gives it.
loosen() {
  local file="$1"
  awk '
    /^- / && NR > 1 && previous !~ /^[[:space:]]*$/ { print "" }
    { print; previous = $0 }
  ' "$file"
}

# trim FILE: FILE without its leading and trailing blank lines.
trim() {
  local file="$1"
  awk '
    { lines[NR] = $0 }
    END {
      first = 1
      while (first <= NR && lines[first] ~ /^[[:space:]]*$/) first++
      last = NR
      while (last >= first && lines[last] ~ /^[[:space:]]*$/) last--
      for (i = first; i <= last; i++) print lines[i]
    }
  ' "$file"
}

# name_issue FILE: the entries on stdin, each ending its first paragraph with
# the issue the fragment's file name starts with, `(#NNN)`, unless the entry
# already names that issue. A trailing full stop stays last.
name_issue() {
  local issue
  issue="$(basename "$1")"
  issue="${issue%%-*}"
  awk -v issue="$issue" '
    function flush(   i) {
      if (count == 0) return
      if (!named) {
        if (lines[last] ~ /\.$/) {
          lines[last] = substr(lines[last], 1, length(lines[last]) - 1) " (#" issue ")."
        } else {
          lines[last] = lines[last] " (#" issue ")"
        }
      }
      for (i = 1; i <= count; i++) print lines[i]
      count = 0
      named = 0
    }
    /^- / { flush(); paragraph = 1; last = 1 }
    {
      lines[++count] = $0
      if ($0 ~ ("#" issue "([^0-9]|$)")) named = 1
      if (count > 1) {
        if ($0 ~ /^[[:space:]]+[-*] /) paragraph = 0
        else if (paragraph) last = count
      }
    }
    END { flush() }
  '
}

self_test() {
  local script status entry="- An entry."
  # Global, so the EXIT trap still sees it after the function returns.
  work="$(mktemp -d)"
  # git writes its objects read-only, so the tree is made writable before it
  # is removed.
  trap 'chmod -R u+w "$work" && rm -r -- "$work"' EXIT
  script="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
  mkdir -p "$work/scripts/release" "$work/scripts/checks" "$work/scripts/lib" "$work/$FRAGMENT_DIR"
  cp "$script" "$work/scripts/release/changelog.sh"
  # The release-section checks --check-preview runs, from the same tree.
  cp "$(dirname "$script")/../checks/changelog-structure.sh" "$(dirname "$script")/../checks/changelog-security.sh" \
    "$work/scripts/checks/"
  cp "$(dirname "$script")/../lib/guard-args.sh" "$work/scripts/lib/"

  # A repository of its own, so no global hook, signing key or identity of the
  # caller's reaches it.
  stub_git() {
    git -C "$work" -c user.name=self-test -c user.email=self-test@example.org \
      -c commit.gpgsign=false -c core.hooksPath=/dev/null "$@"
  }
  flunk() {
    echo "changelog: self-test failed: $*" >&2
    cat "$work/out" >&2
    exit 1
  }
  # expect NAME WANT ARGS...: the script run in the stub exits WANT.
  expect() {
    local name="$1" want="$2"
    shift 2
    status=0
    (cd "$work" && bash scripts/release/changelog.sh "$@") > "$work/out" 2>&1 || status=$?
    [[ "$status" -eq "$want" ]] || flunk "$name exited $status, wanted $want."
  }
  fragment() {
    local name="$1"
    shift
    printf '%s\n' "$@" > "$work/$FRAGMENT_DIR/$name"
  }
  unfragment() {
    local name="$1"
    rm -- "$work/$FRAGMENT_DIR/$name"
  }

  stub_git init -q -b main
  printf 'The fragment format.\n' > "$work/$FRAGMENT_DIR/README.md"

  # --check: the names and contents it accepts, and each it refuses.
  expect "a directory holding only the README" 0 --check
  fragment 3-a-change.added.md '- An entry that runs' '  over two lines.' '  - and nests a list item.' \
    '- A second entry of fragment 3, unpunctuated'
  expect "a well-formed fragment beside the README" 0 --check
  local bad
  for bad in 3-Upper.added.md a-no-issue.added.md 3-a.unknown.md 3-a.added.txt 3-a_b.fixed.md 3-.fixed.md; do
    fragment "$bad" "$entry"
    expect "the misnamed fragment $bad" 1 --check
    grep -q "$bad is not named" "$work/out" || flunk "the misnamed fragment $bad failed without naming it."
    unfragment "$bad"
  done
  fragment 4-heading.fixed.md '### Fixed' '' "$entry"
  expect "a fragment holding a heading" 1 --check
  unfragment 4-heading.fixed.md
  fragment 4-prose.fixed.md 'A sentence, no list item.'
  expect "a fragment of prose" 1 --check
  unfragment 4-prose.fixed.md
  fragment 4-blank.fixed.md '- One entry.' '' '- Another entry.'
  expect "a fragment with a blank line inside" 1 --check
  grep -q 'line 2 is blank' "$work/out" || flunk "the blank line was not named."
  unfragment 4-blank.fixed.md
  fragment 4-flush.fixed.md '- One entry' 'continued flush left.'
  expect "a fragment continued without indentation" 1 --check
  unfragment 4-flush.fixed.md
  printf '' > "$work/$FRAGMENT_DIR/4-empty.fixed.md"
  expect "an empty fragment" 1 --check
  unfragment 4-empty.fixed.md
  mkdir "$work/$FRAGMENT_DIR/5-dir.added.md"
  expect "a directory named as a fragment" 1 --check
  rmdir "$work/$FRAGMENT_DIR/5-dir.added.md"

  # --assemble: the [Unreleased] entries an in-flight change wrote, merged with
  # the fragments, section by section and by file name in the C locale, as the
  # loose list CHANGELOG.md is written in, under a link label spelled in lower
  # case as CHANGELOG.md spells it.
  cat > "$work/CHANGELOG.md" <<'EOF'
# Changelog

## [Unreleased]

### Added

- An entry written into CHANGELOG.md
  before the fragments.

- A second entry written into CHANGELOG.md.

### Fixed

- A fix written into CHANGELOG.md.

## [0.0.1] - 2026-10-01

### Added

- The first release.

[unreleased]: https://example.org/r/compare/v0.0.1...HEAD
[0.0.1]: https://example.org/r/releases/tag/v0.0.1
EOF
  fragment 12-b.added.md '- Fragment 12-b.' '- Fragment 12-b names another issue (#99) and its own (#12) already.'
  fragment 7-y.changed.md '- Fragment 7-y.' '' ''
  fragment 5-x.security.md '- Fragment 5-x.'
  stub_git add -A
  stub_git commit -q -m "the stub"

  expect "a version that is no version" 1 --assemble 0.0 2026-10-10
  expect "a date that is no date" 1 --assemble 0.0.2 10-10-2026
  expect "a missing argument" 2 --assemble 0.0.2
  expect "an existing version" 1 --assemble 0.0.1 2026-10-10
  expect "the cut" 0 --assemble 0.0.2 2026-10-10

  cat > "$work/want" <<'EOF'
# Changelog

## [Unreleased]

## [0.0.2] - 2026-10-10

### Added

- An entry written into CHANGELOG.md
  before the fragments.

- A second entry written into CHANGELOG.md.

- Fragment 12-b (#12).

- Fragment 12-b names another issue (#99) and its own (#12) already.

- An entry that runs
  over two lines (#3).
  - and nests a list item.

- A second entry of fragment 3, unpunctuated (#3)

### Changed

- Fragment 7-y (#7).

### Fixed

- A fix written into CHANGELOG.md.

### Security

- Fragment 5-x (#5).

## [0.0.1] - 2026-10-01

### Added

- The first release.

[unreleased]: https://example.org/r/compare/v0.0.2...HEAD
[0.0.2]: https://example.org/r/compare/v0.0.1...v0.0.2
[0.0.1]: https://example.org/r/releases/tag/v0.0.1
EOF
  diff -u "$work/want" "$work/CHANGELOG.md" > "$work/out" || flunk "the assembled CHANGELOG.md differs from the expected one."
  if [[ -n "$(stub_git ls-files -- "$FRAGMENT_DIR" | grep -v '/README.md$' || true)" ]]; then
    flunk "a fragment is still tracked after the cut."
  fi
  [[ -f "$work/$FRAGMENT_DIR/README.md" ]] || flunk "the cut removed the README."
  expect "a check after the cut" 0 --check

  # A second cut with nothing to release, and an [Unreleased] the cut cannot
  # read, are refused with CHANGELOG.md untouched.
  stub_git commit -q -a -m "the cut"
  expect "a release with no entry" 1 --assemble 0.0.3 2026-10-11
  grep -q 'would be empty' "$work/out" || flunk "the empty release was not named."
  printf '%s\n' '# Changelog' '' '## [Unreleased]' '' '### Improved' '' "$entry" '' \
    '[Unreleased]: https://example.org/r/compare/v0.0.2...HEAD' > "$work/CHANGELOG.md"
  cp "$work/CHANGELOG.md" "$work/before"
  expect "a section Keep a Changelog does not define" 1 --assemble 0.0.3 2026-10-11
  cmp -s "$work/before" "$work/CHANGELOG.md" || flunk "a refused cut wrote CHANGELOG.md."
  printf '%s\n' '# Changelog' '' '## [Unreleased]' '' 'Loose prose.' '' '### Added' '' "$entry" '' \
    '[Unreleased]: https://example.org/r/compare/v0.0.2...HEAD' > "$work/CHANGELOG.md"
  expect "text outside a section heading" 1 --assemble 0.0.3 2026-10-11
  printf '%s\n' '# Changelog' '' '## [Unreleased]' '' '### Added' '' "$entry" > "$work/CHANGELOG.md"
  expect "no [Unreleased] link reference" 1 --assemble 0.0.3 2026-10-11

  # The reason fragment: its form, its uniqueness, and its place first under
  # "### Security" in the preview and at the cut.
  cat > "$work/CHANGELOG.md" <<'EOF'
# Changelog

## [Unreleased]

## [0.0.2] - 2026-10-10

### Added

- The second release.

[Unreleased]: https://example.org/r/compare/v0.0.2...HEAD
[0.0.2]: https://example.org/r/releases/tag/v0.0.2
EOF
  fragment 20-reason.security-reason.md 'An entry that runs'
  expect "a reason without its prefix" 1 --check
  grep -q 'is not a reason paragraph' "$work/out" || flunk "the reason without its prefix was not named."
  fragment 20-reason.security-reason.md "$REASON_PREFIX"
  expect "a reason with nothing after its prefix" 1 --check
  fragment 20-reason.security-reason.md "${REASON_PREFIX}one." '- an entry'
  expect "a reason holding a list item" 1 --check
  fragment 20-reason.security-reason.md "${REASON_PREFIX}the fix needs" 'the console rewrite.'
  fragment 21-other.security-reason.md "${REASON_PREFIX}a second reason."
  expect "two reasons" 1 --check
  grep -q 'merge them' "$work/out" || flunk "the second reason was not named."
  unfragment 21-other.security-reason.md
  expect "a reason for a Security section with no entry" 1 --preview
  fragment 22-dep.security.md '- A dependency bump (GHSA-abcd-2345-wxyz).'
  fragment 23-feature.added.md '- A feature.'
  expect "the preview" 0 --preview
  cat > "$work/want" <<EOF
# Changelog

## [Unreleased]

### Added

- A feature (#23).

### Security

${REASON_PREFIX}the fix needs
the console rewrite.

- A dependency bump (GHSA-abcd-2345-wxyz) (#22).

## [0.0.2] - 2026-10-10

### Added

- The second release.

[Unreleased]: https://example.org/r/compare/v0.0.2...HEAD
[0.0.2]: https://example.org/r/releases/tag/v0.0.2
EOF
  diff -u "$work/want" "$work/out" > "$work/diff" || { cp "$work/diff" "$work/out"; flunk "the preview differs from the expected one."; }
  [[ -f "$work/$FRAGMENT_DIR/22-dep.security.md" ]] || flunk "the preview removed a fragment."
  expect "the preview's release checks with a reason and a GHSA id" 0 --check-preview
  unfragment 20-reason.security-reason.md
  expect "the preview's release checks without the reason" 1 --check-preview
  grep -q 'does not open with' "$work/out" || flunk "the missing reason was not named."
  fragment 20-reason.security-reason.md "${REASON_PREFIX}the fix needs" 'the console rewrite.'
  fragment 22-dep.security.md '- A dependency bump with no advisory.'
  expect "the preview's release checks with an entry naming no advisory" 1 --check-preview
  grep -q 'names no GHSA id' "$work/out" || flunk "the entry without an advisory was not named."
  fragment 22-dep.security.md '- A dependency bump (GHSA-abcd-2345-wxyz).'
  stub_git add -A
  stub_git commit -q -m "the reason"
  expect "the cut with a reason" 0 --assemble 0.0.3 2026-10-12
  awk '/^### Security/ { getline; getline; print; exit }' "$work/CHANGELOG.md" > "$work/out"
  grep -qF "${REASON_PREFIX}the fix needs" "$work/out" || flunk "the cut did not open the Security section with the reason."
  [[ ! -e "$work/$FRAGMENT_DIR/20-reason.security-reason.md" ]] || flunk "the cut left the reason fragment behind."

  echo "changelog: self-test passed."
}

case "${1:-}" in
--self-test)
  [[ $# -eq 1 ]] || usage
  self_test
  ;;
--check)
  [[ $# -eq 1 ]] || usage
  cd "$(dirname "$0")/../.."
  check
  ;;
--assemble)
  [[ $# -eq 3 ]] || usage
  cd "$(dirname "$0")/../.."
  assemble "$2" "$3"
  ;;
--preview)
  [[ $# -eq 1 ]] || usage
  cd "$(dirname "$0")/../.."
  assemble "" "" preview
  ;;
--check-preview)
  [[ $# -eq 1 ]] || usage
  cd "$(dirname "$0")/../.."
  check_preview
  ;;
*) usage ;;
esac
