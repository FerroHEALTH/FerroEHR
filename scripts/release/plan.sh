#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# The release facts the `plan` job of .github/workflows/release.yml verifies
# at the tagged commit, as ONE ordered list (no specification governs the list
# or its order: our own design). The plan job runs it in CI and
# scripts/release/preflight.sh runs it on a release branch before the tag is
# pushed, so the two cannot check different things.
#
#   plan.sh checks <tag> [--notes <file>]
#       Runs every check below at the working tree, in order, and writes the
#       release notes to <file> (default release-notes.md). A failing check is
#       named and the list goes on, so one run reports every finding; checks
#       that read the notes are skipped when the notes could not be composed.
#       Exit 1 when any check failed.
#   plan.sh prev-tag <tag>
#       Prints the release tag before <tag> by version order, read from
#       `git ls-remote` of origin: a pushed tag counts whether or not its
#       release published, and ancestry plays no part.
#   plan.sh notes <tag> <file>
#       Writes the release notes for <tag>: its changelog section, the support
#       period and the CRA user-information link. A pre-release's support
#       period says it is not supported and names the stable release that is.
#   plan.sh fragments <tag>
#       Runs the fragment check of the list alone: no fragment left at a
#       stable tag, every fragment carried by the section at a pre-release
#       tag. CI runs it in the merge queue on a change that moves the
#       workspace version, so a fragment merged ahead of a release PR fails
#       that PR before its tag can.
#   plan.sh support-end <YYYY-MM-DD>
#       Prints the month the support period of a release of that date ends.
#   plan.sh --self-test
#       Proves prev-tag's ordering, the pre-release rule, the support
#       statement and support-end's arithmetic offline.
#
# The milestone check reads the tracker through gh (GH_TOKEN in CI, the
# logged-in gh locally); the chart check needs helm. Exit 2 on a usage error.
set -euo pipefail

readonly TAG_RE='^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'

usage() {
  echo "usage: plan.sh checks <tag> [--notes <file>] | prev-tag <tag> | notes <tag> <file> | fragments <tag> | support-end <YYYY-MM-DD> | --self-test" >&2
  exit 2
}

valid_tag() {
  local tag="$1"
  [[ "$tag" =~ $TAG_RE ]] || {
    echo "::error::plan: '$tag' is not a vX.Y.Z tag" >&2
    exit 2
  }
}

# ── The previous release ─────────────────────────────────────────────────────

# order_prev TAG: reads tag names on stdin and prints the release tag directly
# below TAG's release (its pre-release suffix dropped) by version order.
order_prev() {
  local base="${1%%-*}"
  grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | grep -vxF "$base" |
    { cat; echo "$base"; } | sort -V | awk -v t="$base" '$0==t{print p; exit} {p=$0}'
}

prev_tag() {
  local tag="$1" prev
  prev="$(git ls-remote --tags --refs origin 'v*' | sed 's#.*refs/tags/##' | order_prev "$tag")"
  [[ -n "$prev" ]] || {
    echo "::error::plan: no release tag below ${tag%%-*}" >&2
    return 1
  }
  printf '%s\n' "$prev"
}

# fetch_tag TAG: makes refs/tags/TAG resolvable for a --since comparison. The
# CI checkout is shallow, so it fetches one commit; a full local clone fetches
# without --depth, which would otherwise make the clone shallow.
fetch_tag() {
  local tag="$1"
  if [[ "$(git rev-parse --is-shallow-repository)" == "true" ]]; then
    git fetch --no-tags --depth=1 origin "+refs/tags/$tag:refs/tags/$tag"
  elif ! git rev-parse -q --verify "refs/tags/$tag^{commit}" >/dev/null; then
    git fetch --no-tags origin "+refs/tags/$tag:refs/tags/$tag"
  fi
}

# ── The support period ───────────────────────────────────────────────────────

# support_end YYYY-MM-DD: the month and year five years after the date, as GNU
# `date -d "<date> +5 years" '+%B %Y'` gives it (a 29 February rolls into
# March), computed here so the preflight runs on BSD date too.
support_end() {
  local d="$1" y m day ey em
  local -a months=(January February March April May June July August September October November December)
  local -a mdays=(31 28 31 30 31 30 31 31 30 31 30 31)
  [[ "$d" =~ ^([0-9]{4})-([0-9]{2})-([0-9]{2})$ ]] || return 1
  y=$((10#${BASH_REMATCH[1]}))
  m=$((10#${BASH_REMATCH[2]}))
  day=$((10#${BASH_REMATCH[3]}))
  ((m >= 1 && m <= 12 && day >= 1)) || return 1
  leap() {
    local year="$1"
    ((year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)))
  }
  if ((m == 2)) && leap "$y"; then ((day <= 29)) || return 1; else ((day <= mdays[m - 1])) || return 1; fi
  ey=$((y + 5))
  em=$m
  if ((m == 2 && day == 29)) && ! leap "$ey"; then em=3; fi
  printf '%s %s\n' "${months[em - 1]}" "$ey"
}

# ── The release notes ────────────────────────────────────────────────────────

# stable_below VERSION: reads CHANGELOG.md on stdin and prints the first
# release heading below [VERSION] that carries no pre-release suffix.
stable_below() {
  awk -v heading="## [$1]" '
    index($0, heading) == 1 { found = 1; next }
    found && /^## \[[0-9]+\.[0-9]+\.[0-9]+\]/ { sub(/^## \[/, ""); sub(/\].*/, ""); print; exit }
  '
}

# support_statement VERSION END STABLE: the body of the notes' support period.
# A pre-release is a test build of the version it precedes, so it promises no
# support of its own and names the stable release that stays supported.
support_statement() {
  local version="$1" end="$2" stable="$3"
  if prerelease "$version"; then
    echo "This is a pre-release of ${version%%-*} for testing, and it is not a supported release: it receives no fixes, and the next pre-release or ${version%%-*} itself replaces it. Do not run it in production. ${stable} stays the newest supported release until ${version%%-*} is published, and security fixes ship only in the newest release (Regulation (EU) 2024/2847 Art 13(10)); see [SECURITY.md](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md#supported-versions)."
    return 0
  fi
  echo "Only the newest release is supported. When a new release is published, every earlier release stops receiving fixes: there are no maintenance branches, no long-term-support line and no backports. If you are not on the newest release, upgrade; upgrading is free. Under the Cyber Resilience Act, Cadasto B.V. commits to keep publishing security fixes for at least five years after each release (Art 13(8)) and ships them only in the newest release (Art 13(10)). For this release that commitment runs until the end of $end (Regulation (EU) 2024/2847 Art 13(19)); see [SECURITY.md](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md#supported-versions)."
}

notes() {
  local tag="$1" out="$2" version="${1#v}" released end url
  if ! grep -q "^## \[$version\]" CHANGELOG.md; then
    echo "::error::CHANGELOG.md has no section for $version — releases are changelog-driven (Keep a Changelog). Add '## [$version] - YYYY-MM-DD' first." >&2
    return 1
  fi
  # Everything between "## [<version>]" and the next "## [" heading.
  awk -v ver="$version" '
    $0 ~ "^## \\[" ver "\\]" {found=1; next}
    found && /^## \[/ {exit}
    found {print}
  ' CHANGELOG.md >"$out"
  if ! grep -q '[^[:space:]]' "$out"; then
    echo "::error::The [$version] changelog section is empty." >&2
    return 1
  fi
  # CRA Art 13(19) and Annex II point 7: the end of the support commitment, at
  # least month and year, five years from the release date.
  released="$(sed -n "s/^## \[$version\] - \([0-9-]*\).*/\1/p" CHANGELOG.md | sed -n 1p)"
  if ! end="$(support_end "$released")"; then
    echo "::error::The [$version] heading carries no release date to count the support period from." >&2
    return 1
  fi
  if prerelease "$version"; then
    stable="$(stable_below "$version" <CHANGELOG.md)"
    if [[ -z "$stable" ]]; then
      echo "::error::CHANGELOG.md has no stable release below [$version] to name as the supported one." >&2
      return 1
    fi
  fi
  {
    echo
    echo "### Support period"
    echo
    support_statement "$version" "$end" "${stable:-}"
  } >>"$out"
  # CRA Art 13(18): the Annex II information and instructions to the user, in
  # the book the docs-freeze leg cuts for this tag.
  url="$(bash scripts/checks/release-notes.sh --url "$tag")"
  {
    echo
    echo "### Information and instructions to the user (CRA Annex II)"
    echo
    echo "[CRA information and instructions to the user for $tag]($url): the manufacturer, the vulnerability contact, the support period, secure use, updates and decommissioning (Regulation (EU) 2024/2847 Art 13(18) and Annex II)."
  } >>"$out"
  # EHDS Art. 30(1)(d) and Art. 37: the revision of the EHR system's documents
  # in the FerroHEALTH book this release relies on, from its pin.
  {
    echo
    bash scripts/checks/release-notes.sh --ehds
  } >>"$out"
}

# ── The checks, one function each, run in the order of `list` ───────────────

# A release is cut only when its milestone is empty
# (.claude/rules/issue-workflow.md).
check_milestone() { bash scripts/release/milestone.sh check "$TAG"; }

check_version() {
  local manifest
  manifest="$(grep -m1 '^version = ' Cargo.toml | cut -d'"' -f2)"
  if [[ "$manifest" != "$VERSION" ]]; then
    echo "::error::Tag $TAG does not match workspace version $manifest in Cargo.toml." >&2
    return 1
  fi
  echo "workspace version $manifest matches $TAG"
}

check_changelog_structure() { bash scripts/checks/changelog-structure.sh; }

# CRA Annex I Part II(2), (4) and (8): the section being cut says why a
# security fix ships with functional changes and names every advisory.
check_changelog_security() {
  bash scripts/checks/changelog-security.sh --self-test
  bash scripts/checks/changelog-security.sh --section "$VERSION"
}

# prerelease VERSION: true for a version with a "-" suffix (5.6.0-rc1), the
# rule .github/actions/release-kind applies to the tag.
prerelease() { [[ "$1" == *-* ]]; }

# The cut assembles every fragment under changelog.d/ into the section it
# writes and deletes them (scripts/release/changelog.sh --assemble), so a
# fragment left at the tagged commit is an entry the release notes would miss.
# A pre-release keeps the fragments for the stable cut, so its section must
# carry each of them instead.
check_changelog_fragments() {
  local left
  bash scripts/release/changelog.sh --check
  if prerelease "$VERSION"; then
    bash scripts/release/changelog.sh --covered "$VERSION"
    echo "a pre-release keeps the fragments under changelog.d/ for the stable cut"
    return 0
  fi
  left="$(find changelog.d -maxdepth 1 -type f ! -name README.md | sort)"
  if [[ -n "$left" ]]; then
    echo "::error::changelog.d/ still holds fragments the cut did not assemble; run scripts/release/changelog.sh --assemble $VERSION <date> on the release branch:" >&2
    echo "$left" >&2
    return 1
  fi
  echo "no fragment is left under changelog.d/"
}

# EHDS Art. 30(2) and CRA Art. 13(3), 13(14): a release that changes the
# logging component or a declared security-relevant path carries its
# documentation; the previous release is taken by version.
check_change_control() {
  local prev
  prev="$(prev_tag "$TAG")"
  fetch_tag "$prev"
  bash scripts/checks/technical-documentation.sh --since "$prev"
  bash scripts/checks/cra-risk-assessment.sh --self-test
  bash scripts/checks/cra-risk-assessment.sh --since "$prev"
}

check_notes() {
  notes "$TAG" "$NOTES"
  echo "Release notes for $VERSION:"
  cat "$NOTES"
}

check_notes_content() {
  bash scripts/checks/release-notes.sh --self-test
  if ! bash scripts/checks/release-notes.sh "$NOTES" "$TAG"; then
    echo "::error::The release notes for $TAG do not carry the CRA Annex II user information link or the support period (Regulation (EU) 2024/2847 Art 13(18) and (19))." >&2
    return 1
  fi
}

check_chart_appversion() { bash scripts/checks/chart-appversion.sh; }

# Every release publishes a different packaged chart, so every release needs a
# chart version nobody has published (#2818).
check_chart_unpublished() {
  local ref="ghcr.io/ferrohealth/charts/ferroehr" ver
  command -v helm >/dev/null || {
    echo "::error::helm is required to read the published chart versions" >&2
    return 1
  }
  ver="$(awk '$1 == "version:" {print $2; exit}' deploy/helm/ferroehr/Chart.yaml)"
  if helm show chart "oci://${ref}" --version "$ver" >/dev/null 2>&1; then
    echo "::error::chart version ${ver} is already published at ${ref} — every release ships a distinct packaged chart (injected release facts), so bump deploy/helm/ferroehr/Chart.yaml version in the release PR." >&2
    return 1
  fi
  echo "chart ${ver} is unpublished — the chart leg can ship it"
}

check_statement_version() { bash scripts/checks/statement-version.sh; }

check_citation() {
  local cff_ver
  cff_ver="$(sed -nE 's/^version:[[:space:]]*"?([^"]+)"?[[:space:]]*$/\1/p' CITATION.cff)"
  if [[ "$cff_ver" != "$VERSION" ]]; then
    echo "::error::CITATION.cff version ($cff_ver) != released version ($VERSION) — the release cut bumps CITATION.cff version and date-released together." >&2
    return 1
  fi
  echo "CITATION.cff version $cff_ver matches the released version."
}

# Zenodo ignores CITATION.cff whenever a .zenodo.json is present
# (https://help.zenodo.org/docs/github/describe-software/zenodo-json/).
check_zenodo() { bash scripts/render/zenodo-json.sh --check; }

check_compose_tags() { bash scripts/checks/compose-image-tags.sh; }

check_docs_claims() { bash scripts/checks/docs-claims.sh --all; }

# The list: "<function> <title>", in the order the plan job has always run
# them. A check that reads the notes comes after check_notes.
list() {
  cat <<'EOF'
check_milestone The milestone titled as the tag holds no open item
check_version The tag matches the workspace version
check_changelog_structure Changelog structure (no duplicate subsections, canonical types)
check_changelog_security The Security section of the cut names its advisories and its reason
check_changelog_fragments The changelog fragments are assembled, or carried by a pre-release's section
check_change_control The technical documentation and the CRA risk assessment follow their paths
check_notes The changelog section for this tag becomes the release notes
check_notes_content The release notes carry the CRA user information and the support period
check_chart_appversion The committed chart release facts are coherent
check_chart_unpublished The chart version is not already published
check_statement_version The ferroehr party statement's product version matches the workspace
check_citation CITATION.cff version matches the workspace version
check_zenodo .zenodo.json is in sync with CITATION.cff
check_compose_tags The compose image tags match their policy
check_docs_claims Documented keys resolve in their source
EOF
}

checks() {
  TAG="$1"
  VERSION="${TAG#v}"
  NOTES="$2"
  local fn title notes_ok=1 rc
  local -a failed=()
  while read -r fn title; do
    if [[ "$fn" == check_notes_content && "$notes_ok" -eq 0 ]]; then
      echo "skipped: $title (no notes were composed)"
      failed+=("$title (skipped: no notes)")
      continue
    fi
    if [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then echo "::group::$title"; else echo "── $title"; fi
    set +e
    (
      set -e
      "$fn"
    )
    rc=$?
    set -e
    [[ "${GITHUB_ACTIONS:-}" == "true" ]] && echo "::endgroup::"
    if [[ "$rc" -ne 0 ]]; then
      echo "::error::plan: refused: $title" >&2
      failed+=("$title")
      [[ "$fn" == check_notes ]] && notes_ok=0
    fi
  done < <(list)
  if [[ "${#failed[@]}" -gt 0 ]]; then
    echo "plan: ${#failed[@]} check(s) refused $TAG:" >&2
    printf '  %s\n' "${failed[@]}" >&2
    return 1
  fi
  echo "plan: every release fact for $TAG verified at this tree"
}

self_test() {
  local fails=0 got refused="refused"
  expect() {
    local name="$1" have="$2" want="$3"
    if [[ "$have" != "$want" ]]; then
      echo "self-test: $name: got '$have', want '$want'" >&2
      fails=$((fails + 1))
    fi
  }
  got="$(printf '%s\n' v5.3.2 v5.10.0 v5.3.10 v4.9.9 v5.3.3-rc1 v5.4.0 | order_prev v5.4.0)"
  expect "version order, not lexical" "$got" "v5.3.10"
  got="$(printf '%s\n' v5.3.2 v5.3.3 v5.3.4 | order_prev v5.3.3)"
  expect "the tag itself is skipped" "$got" "v5.3.2"
  got="$(printf '%s\n' v5.3.2 v5.3.3 | order_prev v5.3.3-rc1)"
  expect "a pre-release compares as its release" "$got" "v5.3.2"
  got="$(printf '%s\n' v5.3.2 v5.3.3 | order_prev v5.4.0)"
  expect "an unpushed tag" "$got" "v5.3.3"
  got="$(printf '%s\n' v5.3.3 | order_prev v1.0.0)"
  expect "no tag below" "$got" ""
  expect "a release candidate is a pre-release" "$(prerelease 5.6.0-rc1 && echo yes || echo no)" "yes"
  expect "a dotted suffix is a pre-release" "$(prerelease 5.6.0-rc.1 && echo yes || echo no)" "yes"
  expect "a stable version is no pre-release" "$(prerelease 5.6.0 && echo yes || echo no)" "no"
  local log
  log="$(printf '%s\n' '## [Unreleased]' '' '## [5.6.0-rc2] - 2026-11-02' '' '## [5.6.0-rc1] - 2026-10-28' '' \
    '## [5.5.0] - 2026-10-27' '' '## [5.4.2] - 2026-10-25')"
  expect "the stable release below a release candidate" "$(stable_below 5.6.0-rc2 <<<"$log")" "5.5.0"
  expect "the stable release below a stable release" "$(stable_below 5.5.0 <<<"$log")" "5.4.2"
  expect "no stable release below" "$(stable_below 5.4.2 <<<"$log")" ""
  got="$(support_statement 5.6.0-rc1 "October 2031" 5.5.0)"
  [[ "$got" == *"not a supported release"* && "$got" == *"5.5.0 stays the newest supported release"* && "$got" != *"October 2031"* ]] ||
    expect "a pre-release promises no support of its own" "$got" "the pre-release statement naming 5.5.0"
  got="$(support_statement 5.5.0 "October 2031" "")"
  [[ "$got" == *"runs until the end of October 2031"* ]] ||
    expect "a release carries its support period" "$got" "the statement ending October 2031"
  expect "a plain date" "$(support_end 2026-10-23)" "October 2031"
  expect "a leap day into a common year" "$(support_end 2028-02-29)" "March 2033"
  expect "a leap day into a leap year" "$(support_end 2031-02-29 || echo "$refused")" "$refused"
  expect "the last day of a month" "$(support_end 2026-12-31)" "December 2031"
  expect "an empty date" "$(support_end "" || echo "$refused")" "$refused"
  expect "a month out of range" "$(support_end 2026-13-01 || echo "$refused")" "$refused"
  expect "a day out of range" "$(support_end 2026-04-31 || echo "$refused")" "$refused"
  fragments_self_test
  if [[ "$fails" -gt 0 ]]; then
    echo "plan.sh self-test FAILED ($fails case(s))" >&2
    exit 1
  fi
  echo "plan.sh self-test OK"
}

# fragments_self_test: `plan.sh fragments` over a stub tree, for a pre-release
# tag whose section carries every fragment, one whose section misses a
# fragment merged after the cut, and a stable tag with a fragment left.
fragments_self_test() {
  local here stub rc
  here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  stub="$(mktemp -d)"
  mkdir -p "$stub/scripts/release" "$stub/changelog.d"
  cp "$here/plan.sh" "$here/changelog.sh" "$stub/scripts/release/"
  printf 'The fragment format.\n' > "$stub/changelog.d/README.md"
  printf '%s\n' '- A fix.' > "$stub/changelog.d/30-fix.fixed.md"
  printf '%s\n' '# Changelog' '' '## [Unreleased]' '' '## [0.0.3] - 2026-10-12' '' '### Added' '' '- The third release.' '' \
    '[unreleased]: https://example.org/r/compare/v0.0.3...HEAD' '[0.0.3]: https://example.org/r/releases/tag/v0.0.3' \
    > "$stub/CHANGELOG.md"
  stub_git() {
    git -C "$stub" -c user.name=self-test -c user.email=self-test@example.org \
      -c commit.gpgsign=false -c core.hooksPath=/dev/null "$@"
  }
  run_fragments() {
    rc=0
    bash "$stub/scripts/release/plan.sh" fragments "$1" > "$stub/out" 2>&1 || rc=$?
  }
  stub_git init -q -b main
  stub_git add -A
  stub_git commit -q -m "the stub"
  bash "$stub/scripts/release/changelog.sh" --assemble 0.0.4-rc1 2026-10-13 > /dev/null
  run_fragments v0.0.4-rc1
  expect "a pre-release section that carries every fragment" "$rc" "0"
  printf '%s\n' '- A feature merged after the cut.' > "$stub/changelog.d/31-late.added.md"
  run_fragments v0.0.4-rc1
  expect "a pre-release section that misses a fragment" "$rc" "1"
  grep -q '31-late.added.md' "$stub/out" || expect "the missed fragment named" "$(cat "$stub/out")" "31-late.added.md"
  run_fragments v0.0.4
  expect "a stable tag with fragments left" "$rc" "1"
  chmod -R u+w "$stub"
  rm -r -- "$stub"
}

main() {
  case "${1:-}" in
  checks)
    local tag="${2:-}" flag="${3:-}" notes_file="${4:-}"
    [[ "$#" -eq 2 || ("$#" -eq 4 && "$flag" == "--notes" && -n "$notes_file") ]] || usage
    valid_tag "$tag"
    notes_file="${notes_file:-release-notes.md}"
    [[ "$notes_file" == /* ]] || notes_file="$PWD/$notes_file"
    cd "$(dirname "$0")/../.."
    checks "$tag" "$notes_file"
    ;;
  prev-tag)
    local tag="${2:-}"
    [[ "$#" -eq 2 ]] || usage
    valid_tag "$tag"
    prev_tag "$tag"
    ;;
  notes)
    local tag="${2:-}" out="${3:-}"
    [[ "$#" -eq 3 ]] || usage
    valid_tag "$tag"
    [[ "$out" == /* ]] || out="$PWD/$out"
    cd "$(dirname "$0")/../.."
    notes "$tag" "$out"
    ;;
  fragments)
    local tag="${2:-}"
    [[ "$#" -eq 2 ]] || usage
    valid_tag "$tag"
    cd "$(dirname "$0")/../.."
    TAG="$tag"
    VERSION="${tag#v}"
    check_changelog_fragments
    ;;
  support-end)
    local date="${2:-}"
    [[ "$#" -eq 2 ]] || usage
    support_end "$date" || {
      echo "::error::plan: '$date' is not a YYYY-MM-DD date" >&2
      exit 1
    }
    ;;
  --self-test)
    [[ "$#" -eq 1 ]] || usage
    self_test
    ;;
  *) usage ;;
  esac
}

main "$@"
