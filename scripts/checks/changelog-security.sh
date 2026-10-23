#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The Security rules of a changelog section (Keep a Changelog 1.1.0 plus the
# Cyber Resilience Act, Regulation (EU) 2024/2847).
#
# Two rules, for the one section named (the release being cut, or
# [Unreleased] on every pull request):
#   1. A `### Security` subsection beside any other change subsection opens
#      with the line `Shipped with functional changes because: <reason>`, with
#      a reason on that line. Annex I Part II(2) has security updates
#      "provided separately from functionality updates" where technically
#      feasible, so a release that mixes them says why it could not.
#   2. Every list item under `### Security` names its advisory: a GitHub
#      security advisory id (GHSA-xxxx-xxxx-xxxx), or the path of a VEX
#      statement under the VEX directory that exists (Annex I Part II(4) and
#      (8)). A VEX path that does not exist is refused whatever else the item
#      names.
#
# Only the named section is read, so a section released before these rules is
# never checked retroactively.
#
# Usage:
#   scripts/checks/changelog-security.sh [--section <X.Y.Z|Unreleased>] [<file>]
#   scripts/checks/changelog-security.sh --self-test
#
# <file> defaults to CHANGELOG.md, --section to Unreleased. A VEX path is
# resolved against the directory that holds <file>. VEX_DIR names the VEX
# directory relative to it (default security/vex). CHANGELOG_RULE names the
# document a refusal points at (default .claude/rules/changelog.md).
#
# Exit codes: 0 clean, 1 a finding, 2 a usage error or a missing section.
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
readonly GRAMMAR="[--section <X.Y.Z|Unreleased>] [<file>] | --self-test"
guard_known_flags "$GRAMMAR" "--section --self-test" "$@"

readonly REASON_PREFIX="Shipped with functional changes because: "

# check FILE SECTION: prints the findings, one per line, and returns 1 when
# there is any; returns 2 when the section is absent.
check() {
  local file="$1" section="$2" vex_dir="${VEX_DIR:-security/vex}" root
  [[ -f "$file" ]] || { echo "error: $0: no changelog at $file" >&2; return 2; }
  root="$(dirname "$file")"

  # The scan prints three kinds of record, tab-separated, for the shell to act
  # on: `missing`, `finding <text>`, and `vex <line> <path> <item>` for every
  # VEX path an item names, whose existence only the shell can test.
  local records
  records="$(awk -v want="$section" -v prefix="$REASON_PREFIX" -v vexdir="$vex_dir" -v path="$file" '
    function flush_item(   rest, p, m) {
      if (item == "") return
      has_ghsa = (item ~ /GHSA-[0-9a-z][0-9a-z][0-9a-z][0-9a-z]-[0-9a-z][0-9a-z][0-9a-z][0-9a-z]-[0-9a-z][0-9a-z][0-9a-z][0-9a-z]/)
      rest = item; p = 0
      while (match(rest, vexre)) {
        m = substr(rest, RSTART, RLENGTH)
        sub(/[.,;:)]+$/, "", m)
        print "vex\t" item_line "\t" m "\t" summary
        p = 1
        rest = substr(rest, RSTART + RLENGTH)
      }
      if (!has_ghsa && !p)
        print "finding\t" path ":" item_line ": the Security item \"" summary "\" names no GHSA id and no " vexdir "/ path"
      item = ""
    }
    BEGIN {
      vexre = vexdir
      gsub(/\./, "\\.", vexre)
      vexre = vexre "/[A-Za-z0-9._/#-]+"
    }
    /^## \[/ {
      if (in_section) { flush_item(); in_section = 0; done = 1 }
      name = $0; sub(/^## \[/, "", name); sub(/\].*$/, "", name)
      if (name == want && !done) { in_section = 1; found = 1 }
      next
    }
    !in_section { next }
    /^### / {
      flush_item()
      heading = $0; sub(/^### /, "", heading); sub(/[[:space:]]+$/, "", heading)
      in_security = (heading == "Security")
      if (in_security) { security_line = FNR; first = 1 } else others++
      next
    }
    !in_security { next }
    /^[[:space:]]*$/ { next }
    first {
      first = 0
      if (index($0, prefix) == 1) {
        reason = substr($0, length(prefix) + 1)
        gsub(/[[:space:]]/, "", reason)
        has_reason = (reason != "")
        reason_line = FNR
      }
    }
    /^[-*+] / {
      flush_item()
      item = $0; item_line = FNR
      summary = $0; sub(/^[-*+] +/, "", summary)
      if (length(summary) > 72) summary = substr(summary, 1, 69) "..."
      next
    }
    /^[[:space:]]/ { if (item != "") item = item " " $0; next }
    { flush_item() }
    END {
      if (in_section) flush_item()
      if (!found) { print "missing"; exit }
      if (security_line && others > 0 && !has_reason) {
        if (reason_line)
          print "finding\t" path ":" reason_line ": the [" want "] Security subsection gives no reason after \"" prefix "\""
        else
          print "finding\t" path ":" security_line ": the [" want "] Security subsection sits beside other change subsections but does not open with \"" prefix "<reason>\" (CRA Annex I Part II(2))"
      }
    }
  ' "$file")"

  local kind a b d n=0 out=""
  while IFS=$'\t' read -r kind a b d; do
    case "$kind" in
    missing)
      echo "error: $0: $file has no [$section] section" >&2
      return 2
      ;;
    finding)
      out+="  $a"$'\n'
      n=$((n + 1))
      ;;
    vex)
      # a = line, b = path (with an optional #fragment), d = the item
      if [[ ! -f "$root/${b%%#*}" ]]; then
        out+="  $file:$a: the Security item \"$d\" names $b, which does not exist"$'\n'
        n=$((n + 1))
      fi
      ;;
    "") ;;
    *)
      echo "error: $0: unexpected scan record '$kind'" >&2
      return 2
      ;;
    esac
  done <<<"$records"

  if [[ "$n" -gt 0 ]]; then
    printf '%s' "$out"
    return 1
  fi
}

run() {
  local section="Unreleased" file="CHANGELOG.md" out rc
  while [[ "$#" -gt 0 ]]; do
    case "$1" in
    --section)
      local value="${2:-}"
      [[ "$#" -ge 2 && -n "$value" ]] || guard_usage "$GRAMMAR"
      section="$value"
      shift 2
      ;;
    *)
      file="$1"
      shift
      [[ "$#" -eq 0 ]] || guard_usage "$GRAMMAR"
      ;;
    esac
  done
  set +e
  out="$(check "$file" "$section")"
  rc=$?
  set -e
  case "$rc" in
  0) echo "changelog security OK ([$section] of $file)" ;;
  1)
    echo "changelog security check FAILED ([$section] of $file):" >&2
    # In line order, so the subsection's own finding precedes its items.
    printf '%s\n' "$out" | sort -t: -k2,2n >&2
    echo "(the rule: ${CHANGELOG_RULE:-.claude/rules/changelog.md, Security fixes, advisories and the per-release reviews})" >&2
    exit 1
    ;;
  *) exit 2 ;;
  esac
}

# ── Self-test: one fixture per rule, each proven to pass and to fail ─────────
self_test() {
  local work out rc fails=0
  work="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand now: $work is local and gone at EXIT time
  trap "rm -rf '$work'" EXIT
  mkdir -p "$work/security/vex"
  echo '{}' >"$work/security/vex/rust-advisories.openvex.json"

  # expect NAME WANT_RC NEEDLE SECTION BODY: writes BODY as the changelog
  # under `## [SECTION]` and checks the exit code and that NEEDLE is printed.
  expect() {
    local name="$1" want="$2" needle="$3" section="$4" body="$5"
    printf '# Changelog\n\n## [%s] - 2026-10-23\n\n%s\n\n## [1.0.0] - 2020-01-01\n\n### Added\n\n### Security\n\n- an old entry with nothing\n' \
      "$section" "$body" >"$work/CHANGELOG.md"
    set +e
    out="$(check "$work/CHANGELOG.md" "$section" 2>&1)"
    rc=$?
    set -e
    if [[ "$rc" != "$want" || "$out" != *"$needle"* ]]; then
      echo "self-test: $name: exit $rc (want $want), output: $out" >&2
      fails=$((fails + 1))
    fi
  }

  expect "security alone with a GHSA id" 0 "" 2.0.0 \
    "$(printf '### Security\n\n- **dep 1.2** (GHSA-abcd-2345-wxyz). Wrapped\n  over two lines.')"
  # shellcheck disable=SC2016 # the backticks are Markdown in the fixture, not a command
  expect "security alone with an existing VEX path" 0 "" 2.0.0 \
    "$(printf '### Security\n\n- dep bump (`security/vex/rust-advisories.openvex.json`).')"
  expect "a VEX path with a statement fragment" 0 "" 2.0.0 \
    "$(printf '### Security\n\n- dep bump (security/vex/rust-advisories.openvex.json#RUSTSEC-2026-0001)')"
  expect "the GHSA id on a continuation line" 0 "" 2.0.0 \
    "$(printf '### Security\n\n- dep bump, a long first line\n  (GHSA-abcd-2345-wxyz).')"
  expect "mixed with a reason" 0 "" 2.0.0 \
    "$(printf '### Fixed\n\n- a fix\n\n### Security\n\n%sthe fix needs\nthe behaviour change.\n\n- dep (GHSA-abcd-2345-wxyz)' "$REASON_PREFIX")"
  expect "a section without Security" 0 "" 2.0.0 "$(printf '### Added\n\n- a feature')"
  expect "an empty Unreleased" 0 "" Unreleased ""

  expect "mixed without a reason" 1 "does not open with" 2.0.0 \
    "$(printf '### Changed\n\n- a change\n\n### Security\n\n- dep (GHSA-abcd-2345-wxyz)')"
  expect "mixed with the reason line not first" 1 "does not open with" 2.0.0 \
    "$(printf '### Added\n\n- a feature\n\n### Security\n\n- dep (GHSA-abcd-2345-wxyz)\n\n%sbecause' "$REASON_PREFIX")"
  expect "mixed with an empty reason" 1 "gives no reason" 2.0.0 \
    "$(printf '### Added\n\n- a feature\n\n### Security\n\n%s\n\n- dep (GHSA-abcd-2345-wxyz)' "$REASON_PREFIX")"
  expect "a mixed section after Security" 1 "does not open with" 2.0.0 \
    "$(printf '### Security\n\n- dep (GHSA-abcd-2345-wxyz)\n\n### Fixed\n\n- a fix')"
  expect "an item with no advisory" 1 '"dep bump (RUSTSEC-2026-0001)" names no GHSA id' 2.0.0 \
    "$(printf '### Security\n\n- dep (GHSA-abcd-2345-wxyz)\n- dep bump (RUSTSEC-2026-0001)')"
  expect "an upper-case GHSA id" 1 "names no GHSA id" 2.0.0 \
    "$(printf '### Security\n\n- dep (GHSA-ABCD-2345-WXYZ)')"
  expect "a VEX path that does not exist" 1 "security/vex/gone.openvex.json, which does not exist" 2.0.0 \
    "$(printf '### Security\n\n- dep (security/vex/gone.openvex.json)')"
  expect "a missing VEX path beside a GHSA id" 1 "does not exist" 2.0.0 \
    "$(printf '### Security\n\n- dep (GHSA-abcd-2345-wxyz, security/vex/gone.openvex.json)')"
  printf '## [Unreleased]\n' >"$work/CHANGELOG.md"
  set +e
  out="$(check "$work/CHANGELOG.md" 9.9.9 2>&1)"
  rc=$?
  set -e
  [[ "$rc" == 2 && "$out" == *"has no [9.9.9] section"* ]] || {
    echo "self-test: a missing section: exit $rc, output: $out" >&2
    fails=$((fails + 1))
  }
  # VEX_DIR moves the directory an item has to name.
  printf '## [2.0.0]\n\n### Security\n\n- dep (advisories/vex/x.json)\n' >"$work/CHANGELOG.md"
  mkdir -p "$work/advisories/vex" && echo '{}' >"$work/advisories/vex/x.json"
  set +e
  out="$(VEX_DIR=advisories/vex check "$work/CHANGELOG.md" 2.0.0 2>&1)"
  rc=$?
  set -e
  [[ "$rc" == 0 ]] || {
    echo "self-test: VEX_DIR was not honoured: exit $rc, output: $out" >&2
    fails=$((fails + 1))
  }

  if [[ "$fails" -gt 0 ]]; then
    echo "changelog-security self-test FAILED ($fails case(s))" >&2
    exit 1
  fi
  echo "changelog-security self-test OK"
}

if [[ "${1:-}" == "--self-test" ]]; then
  [[ "$#" -eq 1 ]] || guard_usage "$GRAMMAR"
  self_test
else
  run "$@"
fi
