#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# The SemVer judge of the nine published openehr-* crates
# (.claude/rules/crates-publishing.md, #5353). The rule is The Cargo Book,
# "SemVer Compatibility" (https://doc.rust-lang.org/cargo/reference/semver.html)
# alone: a 0.y.z release "can treat changes in "y" as a major release, and "z"
# as a minor release", and "0.0.z releases are always major changes".
# cargo-semver-checks (https://github.com/obi1kenobi/cargo-semver-checks)
# compares each crate's public API in the tree with its newest release on
# crates.io, the baseline: main holds the manifests at a floor below the
# published line, and the publish step stamps the version (#4555). A public
# dependency moved to another compatibility line (scripts/lib/public-deps.sh)
# breaks a crate where the judge cannot see it, so it counts as a break.
#
# Each crate is judged on the judge's default feature set (every feature but
# the unstable-named ones) and, for a crate with features, on
# --only-explicit-features, what a dependant built with
# `default-features = false` compiles against: the cargo-semver-checks README
# (v0.51.0) says it does not catch "breaking changes that exist when only a
# subset of all crate features are activated".
#
# No specification governs where the judge runs: our own design. CI runs
# `report` on a pull request and in the merge queue, publish-crates.sh plan
# takes the version from `next`, and scripts/release/plan.sh runs `check`
# over the version the release stamps.
#
#   semver.sh report
#       Judge the tree and say what the next publish needs; refuse nothing a
#       later stamp can still answer. Exit 1 only when the judge cannot judge.
#   semver.sh next <published>
#       Print the lockstep version that follows <published> for the change
#       the judge finds: 0.1.0 while <published> is 0.0.z (owner decision
#       2026-11-10, #5353), then 0.(y+1).0 for a break and 0.y.(z+1) for a
#       compatible change.
#   semver.sh check <version>
#       Exit 1 when publishing <version> does not answer the change since
#       each crate's newest release.
#   semver.sh --pin
#       Print the cargo-semver-checks version docs/VERSIONS.md pins.
#   semver.sh --self-test
#       Prove the version arithmetic and the summary reading offline.
#
# The installed cargo-semver-checks must be the pinned version. Exit 2 on a
# usage or setup error. Every match reads a here-string
# (scripts/checks/pipe-early-exit.sh).
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"
# shellcheck source=scripts/lib/public-deps.sh
. scripts/lib/public-deps.sh

readonly AGENT='User-Agent: ferroehr-semver (scripts/checks/semver.sh)'

usage() {
  echo "usage: semver.sh report | next <published> | check <version> | --pin | --self-test" >&2
  exit 2
}

# The Pin cell of the cargo-semver-checks row of docs/VERSIONS.md.
pin() {
  awk -F'|' '{ k = $2; gsub(/[` ]/, "", k) } k == "cargo-semver-checks" { v = $3; sub(/\(.*/, "", v); gsub(/[^0-9.]/, "", v); print v; exit }' docs/VERSIONS.md
}

require_tool() {
  local want installed
  want="$(pin)"
  [[ -n "$want" ]] || { echo "semver: docs/VERSIONS.md pins no cargo-semver-checks version" >&2; exit 2; }
  if ! installed="$(cargo semver-checks --version 2>/dev/null)"; then
    echo "semver: cargo-semver-checks is not installed; install $want (cargo install --locked cargo-semver-checks@$want)" >&2
    exit 2
  fi
  [[ "$installed" == "cargo-semver-checks $want" ]] || {
    echo "semver: found '$installed', docs/VERSIONS.md pins $want" >&2
    exit 2
  }
}

# The crates.io API document at path $1 under /api/v1/crates/.
crates_io() {
  curl --proto '=https' --tlsv1.2 -sSL --fail --retry 4 --retry-connrefused \
    -H "$AGENT" "https://crates.io/api/v1/crates/$1"
}

# The newest stable release of crate $1 on crates.io; fatal when unknown,
# because an unanswered request must not read as "never published".
newest_release() {
  local body version
  body="$(crates_io "$1")" || { echo "semver: crates.io did not answer for $1" >&2; exit 2; }
  version="$(jq -r '.crate.max_stable_version // empty' <<<"$body")"
  [[ -n "$version" ]] || { echo "semver: $1 has no stable release on crates.io" >&2; exit 2; }
  echo "$version"
}

# The level of the step from $1 to $2 by Cargo's rule, as cargo-semver-checks
# classifies it: major when the left-most non-zero component moves, minor
# when the next one does, patch otherwise.
step_level() {
  local a="$1" b="$2" a1 a2 a3 b1 b2 b3
  IFS=. read -r a1 a2 a3 <<<"$a"
  IFS=. read -r b1 b2 b3 <<<"$b"
  if [[ "$a1" != "$b1" ]]; then
    echo major
  elif [[ "$a2" != "$b2" ]]; then
    if [[ "$b1" == 0 ]]; then echo major; else echo minor; fi
  elif [[ "$a3" != "$b3" ]]; then
    if [[ "$b1" == 0 && "$b2" == 0 ]]; then
      echo major
    elif [[ "$b1" == 0 ]]; then
      echo minor
    else
      echo patch
    fi
  else
    echo none
  fi
}

rank() {
  case "$1" in
  none | patch) echo 0 ;;
  minor) echo 1 ;;
  major) echo 2 ;;
  *) echo "semver: unknown level '$1'" >&2; exit 2 ;;
  esac
}

# The higher of two judged levels.
max_level() {
  if [[ "$(rank "$1")" -ge "$(rank "$2")" ]]; then echo "$1"; else echo "$2"; fi
}

# The version after $1 for the judged level $2 (none, minor or major).
next_version() {
  local published="$1" needed="$2" x y z
  IFS=. read -r x y z <<<"$published"
  if [[ "$x" == 0 && "$y" == 0 ]]; then
    # NOTE: owner decision 2026-11-10 (#5353): the line leaves 0.0.z at its next publish.
    echo "0.1.0"
  elif [[ "$x" == 0 ]]; then
    if [[ "$needed" == major ]]; then echo "0.$((y + 1)).0"; else echo "0.$y.$((z + 1))"; fi
  else
    case "$needed" in
    major)
      echo "semver: a break past $published needs the major, which is an owner decision (.claude/rules/crates-publishing.md)" >&2
      return 1
      ;;
    minor) echo "$x.$((y + 1)).0" ;;
    *) echo "$x.$y.$((z + 1))" ;;
    esac
  fi
}

# `crate level` lines, one per `Checking` the judge's log on stdin printed,
# the level read from the `Summary` after it: major, minor or none. The
# colour codes a CI terminal adds around the status word are dropped first.
read_summaries() {
  awk '
    { gsub(/\033\[[0-9;]*m/, "") }
    $1 == "Checking" { crate = $2; next }
    $1 == "Summary" && crate != "" {
      level = "none"
      if ($0 ~ /requires new major version/) level = "major"
      else if ($0 ~ /requires new minor version/) level = "minor"
      print crate, level
      crate = ""
    }
  '
}

# The public dependency lines of crate $1 in the tree, and of release $2 of it.
tree_lines() {
  manifest_requirements Cargo.toml "crates/$1/Cargo.toml" | public_lines
}
release_lines() {
  local body
  body="$(crates_io "$1/$2/dependencies")" || { echo "semver: crates.io did not answer for $1 $2" >&2; exit 2; }
  jq -r '.dependencies[] | select(.kind == "normal") | "\(.crate_id) \(.req)"' <<<"$body" | public_lines
}

note() {
  echo "$*" >&2
  [[ -z "${GITHUB_STEP_SUMMARY:-}" ]] || echo "$*" >>"$GITHUB_STEP_SUMMARY"
}

# Judge every crate against its newest release, and print the level the
# change needs over the nine (none, minor or major) on stdout, with the
# per-crate verdict on stderr. Exit 1 when the judge cannot judge a crate.
judge() {
  local crates crate newest baseline=() group selection log status got want level="none" moved
  require_tool
  crates=()
  while read -r crate; do crates+=("$crate"); done < <(bash scripts/release/publish-crates.sh list)
  for crate in "${crates[@]}"; do
    newest="$(newest_release "$crate")"
    baseline+=("$crate $newest")
  done
  log="$(mktemp)"
  # shellcheck disable=SC2064 # expand $log now
  trap "rm -f '$log'" RETURN
  # One judge run per baseline version and feature set; lockstep makes it
  # one version unless a crate was published on its own.
  while read -r newest; do
    for selection in default only-explicit; do
      group=()
      for crate in "${crates[@]}"; do
        grep -Fxq "$crate $newest" <<<"$(printf '%s\n' "${baseline[@]}")" || continue
        if [[ "$selection" == only-explicit ]] && ! grep -q '^\[features\]' "crates/$crate/Cargo.toml"; then
          continue
        fi
        group+=(--package "$crate")
      done
      [[ "${#group[@]}" -gt 0 ]] || continue
      local -a run=(check-release "${group[@]}" --baseline-version "$newest" --release-type patch --color never)
      [[ "$selection" == default ]] || run+=(--only-explicit-features)
      note "semver: judging $(( ${#group[@]} / 2 )) crate(s) against $newest on the $selection feature set"
      status=0
      cargo semver-checks "${run[@]}" </dev/null >"$log" 2>&1 || status=$?
      cat "$log" >&2
      got="$(read_summaries <"$log")"
      want=$(( ${#group[@]} / 2 ))
      if [[ "$(grep -c . <<<"$got" || true)" -ne "$want" ]]; then
        echo "::error::semver: cargo-semver-checks judged $(grep -c . <<<"$got" || true) of $want crates against $newest on the $selection feature set (exit $status); see its output above." >&2
        return 1
      fi
      while read -r crate got; do
        note "semver: $crate on the $selection feature set needs: $got"
        level="$(max_level "$level" "$got")"
      done <<<"$got"
    done
  done < <(printf '%s\n' "${baseline[@]}" | awk '{ print $2 }' | sort -u)
  for crate in "${crates[@]}"; do
    newest="$(awk -v c="$crate" '$1 == c { print $2 }' <<<"$(printf '%s\n' "${baseline[@]}")")"
    moved="$(moved_lines "$(release_lines "$crate" "$newest")" "$(tree_lines "$crate")")"
    if [[ -n "$moved" ]]; then
      note "semver: $crate moves a public dependency to another line ($moved), a break"
      level=major
    fi
  done
  echo "$level"
}

# The newest published version of the set, as publish-crates.sh plan reads it.
newest_of_set() {
  local crate newest top="0.0.0" list
  list="$(bash scripts/release/publish-crates.sh list)"
  while read -r crate; do
    newest="$(newest_release "$crate")"
    top="$(printf '%s\n%s\n' "$top" "$newest" | sort -V | tail -n 1)"
  done <<<"$list"
  echo "$top"
}

# The words for a judged level.
describe() {
  case "$1" in
  major) echo "a break" ;;
  minor) echo "an addition" ;;
  *) echo "no change to the public API" ;;
  esac
}

do_report() {
  local level published next
  level="$(judge)"
  published="$(newest_of_set)"
  if next="$(next_version "$published" "$level")"; then
    note "semver: against $published the judge finds $(describe "$level"); the next publish stamps $next"
  else
    note "semver: against $published the judge finds $(describe "$level"), which the line cannot take without an owner decision"
  fi
}

do_next() {
  local published="$1" level=none
  [[ "$published" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || usage
  # A step out of 0.0.z answers every change, so the judge is not asked.
  [[ "$published" == 0.0.* ]] || level="$(judge)"
  next_version "$published" "$level"
}

do_check() {
  local version="$1" published needed step
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || usage
  published="$(newest_of_set)"
  if [[ "$version" == "$published" ]]; then
    note "semver: $version is the newest release; the publish uploads nothing new"
    return 0
  fi
  [[ "$(printf '%s\n%s\n' "$published" "$version" | sort -V | tail -n 1)" == "$version" ]] || {
    echo "::error::semver: $version is below the newest release $published" >&2
    return 1
  }
  needed="$(judge)"
  step="$(step_level "$published" "$version")"
  if [[ "$(rank "$needed")" -gt "$(rank "$step")" ]]; then
    echo "::error::semver: $published -> $version is a $step-level step, and the change needs $needed (The Cargo Book, SemVer Compatibility; .claude/rules/crates-publishing.md)" >&2
    return 1
  fi
  note "semver: $published -> $version ($step) answers the change ($needed)"
}

self_test() {
  local fails=0
  expect() {
    if [[ "$2" != "$3" ]]; then
      echo "self-test: $1: got '$2', want '$3'" >&2
      fails=$((fails + 1))
    fi
  }
  expect "0.0.z to 0.1.0 is a major step" "$(step_level 0.0.136 0.1.0)" major
  expect "0.0.z to 0.0.z+1 is a major step" "$(step_level 0.0.136 0.0.137)" major
  expect "0.y.z to 0.y.z+1 is a minor step" "$(step_level 0.1.0 0.1.1)" minor
  expect "0.y to 0.y+1 is a major step" "$(step_level 0.1.4 0.2.0)" major
  expect "x.y to x.y+1 is a minor step" "$(step_level 1.2.3 1.3.0)" minor
  expect "x.y.z to x.y.z+1 is a patch step" "$(step_level 1.2.3 1.2.4)" patch
  expect "the line leaves 0.0.z for 0.1.0" "$(next_version 0.0.136 none)" 0.1.0
  expect "a break on 0.0.z still goes to 0.1.0" "$(next_version 0.0.136 major)" 0.1.0
  expect "a compatible change on 0.y moves the patch" "$(next_version 0.1.3 none)" 0.1.4
  expect "an addition on 0.y moves the patch" "$(next_version 0.1.3 minor)" 0.1.4
  expect "a break on 0.y moves the minor" "$(next_version 0.1.3 major)" 0.2.0
  expect "an addition past 1.0 moves the minor" "$(next_version 1.2.3 minor)" 1.3.0
  expect "a break past 1.0 is refused" "$(next_version 1.2.3 major 2>/dev/null || echo refused)" refused
  expect "a minor-level need fits a 0.y patch step" "$([[ "$(rank minor)" -le "$(rank "$(step_level 0.1.0 0.1.1)")" ]] && echo fits)" fits
  expect "a break does not fit a 0.y patch step" "$([[ "$(rank major)" -le "$(rank "$(step_level 0.1.0 0.1.1)")" ]] || echo refused)" refused
  expect "the summaries are read per crate" "$(read_summaries <<'EOF'
    Checking openehr-base v0.1.0 -> v0.1.0 (assume patch change)
     Summary no semver update required
    Checking openehr-rm v0.1.0 -> v0.1.0 (assume patch change)
     Summary semver requires new major version: 1 major and 0 minor checks failed
    Checking openehr-its v0.1.0 -> v0.1.0 (assume patch change)
     Summary semver requires new minor version: 0 major and 2 minor checks failed
EOF
)" $'openehr-base none\nopenehr-rm major\nopenehr-its minor'
  expect "a coloured summary is read" "$(printf '\033[1m\033[32m    Checking\033[0m openehr-adl v0.0.136 -> v0.0.128 (assume patch change)\n\033[1m\033[32m     Summary\033[0m no semver update required\n' | read_summaries)" "openehr-adl none"
  expect "a caret line of 0.y" "$(compat_line 0.16.3)" 0.16
  expect "an exact line of 0.0.z" "$(compat_line "=0.0.7")" 0.0.7
  expect "a moved line is named" "$(moved_lines $'ferrohealth-secret 0.2' $'ferrohealth-secret 0.3')" "ferrohealth-secret 0.2->0.3"
  expect "a sibling is no public dependency" "$(public_dependency openehr-base && echo public || echo private)" private
  [[ -n "$(pin)" ]] || { echo "self-test: docs/VERSIONS.md pins no cargo-semver-checks" >&2; fails=$((fails + 1)); }
  if [[ "$fails" -gt 0 ]]; then
    echo "semver.sh self-test FAILED ($fails case(s))" >&2
    exit 1
  fi
  echo "semver.sh self-test OK"
}

case "${1:-}" in
report) [[ "$#" -eq 1 ]] || usage; do_report ;;
next) [[ "$#" -eq 2 ]] || usage; do_next "$2" ;;
check) [[ "$#" -eq 2 ]] || usage; do_check "$2" ;;
--pin) [[ "$#" -eq 1 ]] || usage; pin ;;
--self-test) [[ "$#" -eq 1 ]] || usage; self_test ;;
*) usage ;;
esac
