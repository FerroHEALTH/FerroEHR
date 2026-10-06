#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Withdraws a FerroEHR release its manufacturer, Cadasto B.V., found not to
# conform: Regulation (EU) 2025/327 Art 30(1)(i) and (j)
# (docs/law/eu/ehds/text.html) and Regulation (EU) 2024/2847 Art 13(21)
# (docs/law/eu/cra/text.html). The steps are our own design; the procedure
# around them is docs/post-market.md.
#
# A published release is immutable and its `v*` tag protected, so nothing
# published is edited or deleted. The script:
#
#   1. refuses a version with no row naming it in the `versions` column of
#      docs/registers/non-conforming-versions.tsv, a correcting release that
#      is not later, is not a published release, or is itself in a row;
#   2. moves the floating image tags the release lane publishes
#      (build-image.yml: `<major>.<minor>` and, for a non-prerelease tag,
#      `latest`) of the server, viewer and postgres images from the withdrawn
#      digest to the correcting release, where they still point at the
#      withdrawn digest. The `<version>` and `sha-<commit>` tags stay: GHCR
#      drops a tag only by deleting the digest, which would break every
#      deployment pinned by digest and the attestations that name it. A
#      `<major>.<minor>` tag whose line has no correcting release stays and is
#      named in the notice (there are no backports, SECURITY.md);
#   3. names the chart version the release shipped (--chart, or the release
#      notes) and checks its appVersion. An OCI chart registry keeps no index,
#      so a single chart version cannot be marked deprecated: Helm's
#      `deprecated` field sits inside the packaged Chart.yaml, and re-pushing
#      the version would replace its digest. The notice says so;
#   4. prints the notices to post and the register columns to fill.
#
# Usage:
#   scripts/release/withdraw.sh <version> --to <version> [--chart <chart-version>] [--dry-run]
#   scripts/release/withdraw.sh --self-test
#
# Every command is printed to stderr with a `+` before it. --dry-run runs the
# read-only lookups, prints the tag moves without running them, and changes
# nothing. Exit 1 on a refusal, 2 on a usage error. WITHDRAW_ROOT names the
# repository root and WITHDRAW_TODAY the date printed (YYYY-MM-DD); the
# self-test sets both and puts stub docker, gh and helm on PATH.
set -euo pipefail
export LC_ALL=C

readonly REPO=FerroHEALTH/FerroEHR
readonly IMAGES='ghcr.io/ferrohealth/ferroehr ghcr.io/ferrohealth/ferroehr-viewer ghcr.io/ferrohealth/ferroehr-postgres'
readonly CHART_REF=oci://ghcr.io/ferrohealth/charts/ferroehr
readonly REGISTER=docs/registers/non-conforming-versions.tsv
readonly CONTACT=info@cadasto.com

die() {
  echo "withdraw: $*" >&2
  exit 1
}

usage() {
  echo "usage: withdraw.sh <version> --to <version> [--chart <chart-version>] [--dry-run] | --self-test" >&2
  exit 2
}

# trace CMD...: prints CMD the way a shell would read it back, on fd 3 (the
# caller's stderr), so a lookup that silences its own stderr still shows.
trace() {
  printf '+' >&3
  printf ' %q' "$@" >&3
  printf '\n' >&3
}

# look CMD...: runs a read-only command, traced, in a dry run too.
look() {
  trace "$@"
  "$@"
}

# change CMD...: runs a command that changes the registry, traced; a dry run
# only prints it.
change() {
  trace "$@"
  if [ "$DRY_RUN" -eq 1 ]; then
    echo "  (dry run: not run)" >&3
    return 0
  fi
  "$@"
}

# digest IMAGE TAG: the digest IMAGE:TAG names, or nothing when there is none.
digest() {
  look docker buildx imagetools inspect "$1:$2" 2> /dev/null \
    | awk '$1 == "Digest:" { print $2; exit }' || true
}

# published TAG: whether TAG is a published GitHub release, not a draft.
published() {
  local draft
  draft="$(look gh release view "$1" --repo "$REPO" --json isDraft --jq .isDraft 2> /dev/null)" || return 1
  [ "$draft" = false ]
}

# newer A B: whether version A is later than version B.
newer() {
  local a1 a2 a3 b1 b2 b3
  IFS=. read -r a1 a2 a3 <<< "$1"
  IFS=. read -r b1 b2 b3 <<< "$2"
  [ "$a1" -ne "$b1" ] && { [ "$a1" -gt "$b1" ]; return; }
  [ "$a2" -ne "$b2" ] && { [ "$a2" -gt "$b2" ]; return; }
  [ "$a3" -gt "$b3" ]
}

# rows VERSION: the register rows whose `versions` column names VERSION.
rows() {
  awk -F '\t' -v version="$1" '
    /^#/ || $1 == "id" || NF < 3 { next }
    {
      n = split($3, listed, /[ ,;]+/)
      for (i = 1; i <= n; i++) {
        sub(/^v/, "", listed[i])
        if (listed[i] == version) { print; next }
      }
    }' "$ROOT/$REGISTER"
}

# chart_from_notes VERSION: the chart version the release notes of VERSION name.
chart_from_notes() {
  # shellcheck disable=SC2016 # the backticks are Markdown in the notes, never expanded
  look gh release view "v$1" --repo "$REPO" --json body --jq .body 2> /dev/null \
    | sed -n 's/^| chart | `oci:[^`]*` version `\([^`]*\)` |.*/\1/p' | head -n 1 || true
}

run() {
  VERSION='' TO='' CHART='' DRY_RUN=0
  while [ $# -gt 0 ]; do
    case "$1" in
      --to) [ $# -ge 2 ] || usage; TO="${2#v}"; shift 2 ;;
      --chart) [ $# -ge 2 ] || usage; CHART="$2"; shift 2 ;;
      --dry-run) DRY_RUN=1; shift ;;
      -*) usage ;;
      *) [ -z "$VERSION" ] || usage; VERSION="${1#v}"; shift ;;
    esac
  done
  local semver='^[0-9]+\.[0-9]+\.[0-9]+$'
  [[ "$VERSION" =~ $semver ]] || usage
  [[ "$TO" =~ $semver ]] || usage
  [ -z "$CHART" ] || [[ "$CHART" =~ $semver ]] || usage
  ROOT="${WITHDRAW_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}"
  TODAY="${WITHDRAW_TODAY:-$(date -u +%Y-%m-%d)}"
  local line="${VERSION%.*}" to_line="${TO%.*}"

  [ -f "$ROOT/$REGISTER" ] || die "$REGISTER is missing."
  local found ids summaries
  found="$(rows "$VERSION")"
  [ -n "$found" ] || die "no row in $REGISTER names $VERSION in its versions column: enter the finding first (EHDS Art 30(1)(i) and (o))."
  ids="$(printf '%s\n' "$found" | cut -f1 | paste -sd, - | sed 's/,/, /g')"
  summaries="$(printf '%s\n' "$found" | cut -f5)"
  [ -z "$(rows "$TO")" ] || die "the correcting release $TO is itself named in $REGISTER."
  newer "$TO" "$VERSION" || die "the correcting release $TO is not later than $VERSION."
  for command in docker gh; do
    command -v "$command" > /dev/null || die "$command is not installed."
  done
  published "v$VERSION" || die "v$VERSION is not a published release of $REPO."
  published "v$TO" || die "the correcting release v$TO is not a published release of $REPO."

  # Every tag is read before anything moves, so a refusal leaves the registry as it was.
  local image withdrawn target tag current moves='' kept=''
  for image in $IMAGES; do
    withdrawn="$(digest "$image" "$VERSION")"
    if [ -z "$withdrawn" ]; then
      echo "withdraw: $image has no $VERSION tag; nothing to move there." >&2
      continue
    fi
    target="$(digest "$image" "$TO")"
    [ -n "$target" ] || die "$image has $VERSION but no $TO tag to move to."
    for tag in "$line" latest; do
      current="$(digest "$image" "$tag")"
      if [ "$current" != "$withdrawn" ]; then
        echo "withdraw: $image:$tag does not point at $VERSION; it stays." >&2
      elif [ "$tag" = "$line" ] && [ "$to_line" != "$line" ]; then
        kept="$kept$image:$tag"$'\n'
        echo "withdraw: $image:$tag points at $VERSION and no $line release corrects it; it stays." >&2
      else
        moves="$moves$image $tag $target"$'\n'
      fi
    done
  done

  # The chart version is read before anything moves too.
  local chart_line
  if [ -z "$CHART" ]; then
    CHART="$(chart_from_notes "$VERSION")"
  fi
  if [ -n "$CHART" ]; then
    command -v helm > /dev/null || die "helm is not installed."
    local app
    app="$(look helm show chart "$CHART_REF" --version "$CHART" 2> /dev/null \
      | awk '$1 == "appVersion:" { gsub(/"/, "", $2); print $2; exit }')" || true
    [ -n "$app" ] || die "chart $CHART is not published at $CHART_REF."
    [ "$app" = "$VERSION" ] || die "chart $CHART ships appVersion $app, not $VERSION: pass the chart version the release shipped with --chart."
    chart_line="Helm chart version $CHART of $CHART_REF, which ships $VERSION"
  else
    chart_line="the Helm chart version that shipped $VERSION (the release notes name none: pass --chart)"
  fi

  while IFS=' ' read -r image tag target; do
    [ -n "$image" ] || continue
    change docker buildx imagetools create -t "$image:$tag" "$image@$target"
  done <<< "$moves"

  local moved verb=Moved
  [ "$DRY_RUN" -eq 0 ] || verb='To move (dry run)'
  moved="$(printf '%s' "$moves" | awk 'NF { print "  - " $1 ":" $2 }')"
  [ -n "$moved" ] || moved="  - none: every floating tag already pointed elsewhere"

  cat << EOF

== The chart ==
An OCI chart registry cannot mark one chart version deprecated: Helm's
deprecated field is part of the packaged Chart.yaml, there is no index to
carry it, and re-pushing a version replaces its digest. Name $chart_line
in the advisory and in SECURITY.md instead.

== The image tags ==
$verb to $TO:
$moved
EOF
  if [ -n "$kept" ]; then
    printf 'Still pointing at %s, because no %s release corrects it (move these deployments by hand to %s):\n' "$VERSION" "$line" "$TO"
    printf '%s' "$kept" | awk 'NF { print "  - " $0 }'
  fi

  cat << EOF

== Advisory text (a GitHub security advisory where the finding is a vulnerability, otherwise a GitHub Discussions announcement) ==
FerroEHR $VERSION is withdrawn as of $TODAY by its manufacturer, Cadasto B.V.
(Regulation (EU) 2025/327 Art 30(1)(i)). The finding is $ids in the register
of non-conforming versions,
https://github.com/$REPO/blob/main/$REGISTER:
$summaries

Move every deployment of $VERSION to $TO or later. These image tags point at
$TO:
$moved

A published release cannot be changed or deleted, so these stay published:
the $VERSION image tags and their digests, the release and its attestations,
and $chart_line. A deployment pinned to any of them keeps running the
withdrawn version until it is moved. $VERSION is not supported, whatever its
age. Questions go to $CONTACT.

== The line to add to SECURITY.md, section "Supported versions" (replace "No release has been withdrawn.") ==
- \`$VERSION\`, withdrawn $TODAY ($ids): move to \`$TO\` or later.

== Notice to the national authorities (Regulation (EU) 2025/327 Art 30(1)(i)) ==
To each Member State where FerroEHR $VERSION was made available or put into
service:
FerroEHR $VERSION does not conform ($ids): $summaries
Corrective action: $VERSION is withdrawn as of $TODAY; $TO, a published
release, brings FerroEHR into conformity. Timetable: <the dates the
correction, the withdrawal and the notices took or take place>.

== Notice to distributors, importers and users (Art 30(1)(j)) ==
The advisory text above, through the advisory, an [Unreleased] entry in
CHANGELOG.md that the next release's notes carry, and a direct message to
every distributor, importer and user the manufacturer knows.

== Register columns to fill in $ids ==
  action            withdrawn $TODAY; image tags moved to $TO (the list above)
  corrected_in      $TO
  authorities_told  <date>, <the Member States told>, once the notice above is sent
  users_told        <date>, <the route: advisory, Discussions, direct messages>
  closed            <date>, once every column above is filled
EOF
  if [ "$DRY_RUN" -eq 1 ]; then
    echo
    echo "withdraw: dry run, nothing was changed."
  fi
}

self_test() {
  local work failed=0 n=0
  work="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand $work now: the local is gone at EXIT.
  trap "rm -r '$work'" EXIT
  local stub="$work/stub" root="$work/repo"

  # The stub tools read and write $stub and log every call to $stub/calls.
  mkdir -p "$work/bin"
  cat > "$work/bin/docker" << 'SHIM'
#!/usr/bin/env bash
set -eu
echo "docker $*" >> "$STUB/calls"
[ "$1 $2" = "buildx imagetools" ] || exit 64
case "$3" in
  inspect)
    file="$STUB/registry/${4##*/}"; file="${file%:*}/${4##*:}"
    [ -f "$file" ] || { echo "ERROR: not found" >&2; exit 1; }
    printf 'Name:      %s\nMediaType: application/vnd.oci.image.index.v1+json\nDigest:    %s\n' "$4" "$(cat "$file")"
    ;;
  create)
    [ "$4" = -t ] || exit 64
    dest="$STUB/registry/${5##*/}"; dest="${dest%:*}/${5##*:}"
    printf '%s\n' "${6##*@}" > "$dest"
    ;;
  *) exit 64 ;;
esac
SHIM
  cat > "$work/bin/gh" << 'SHIM'
#!/usr/bin/env bash
set -eu
echo "gh $*" >> "$STUB/calls"
[ "$1 $2" = "release view" ] || exit 64
[ -f "$STUB/releases/$3" ] || exit 1
case "$7" in
  isDraft) echo false ;;
  body) cat "$STUB/releases/$3" ;;
  *) exit 64 ;;
esac
SHIM
  cat > "$work/bin/helm" << 'SHIM'
#!/usr/bin/env bash
set -eu
echo "helm $*" >> "$STUB/calls"
[ "$1 $2" = "show chart" ] || exit 64
[ -f "$STUB/charts/$5" ] || exit 1
cat "$STUB/charts/$5"
SHIM
  chmod +x "$work/bin/docker" "$work/bin/gh" "$work/bin/helm"

  # fixture: a fresh register and registry. 4.3.3 and 4.2.9 are in the
  # register, 4.3.4 corrects them; the viewer's latest points elsewhere.
  fixture() {
    if [ -d "$stub" ]; then rm -r "$stub" "$root"; fi
    mkdir -p "$root/docs/registers" "$stub/releases" "$stub/charts"
    {
      printf '# a comment\n'
      printf 'id\tfound\tversions\trequirement\tsummary\tserious_incident\tcra_notified\taction\tcorrected_in\tauthorities_told\tusers_told\tclosed\n'
      printf 'N-2026-1\t2026-10-01\t4.3.2, 4.3.3\tAnnex II 3.1\tA synthetic finding.\tno\tno\t\t\t\t\t\n'
      printf 'N-2026-2\t2026-10-02\tv4.2.9\tAnnex I Part I\tAnother synthetic finding.\tno\tno\t\t\t\t\t\n'
    } > "$root/$REGISTER"
    # shellcheck disable=SC2016 # the backticks are Markdown, never expanded
    printf 'notes\n| chart | `oci://ghcr.io/ferrohealth/charts/ferroehr` version `1.9.3` |\n' > "$stub/releases/v4.3.3"
    printf 'notes\n' > "$stub/releases/v4.2.9"
    printf 'notes\n' > "$stub/releases/v4.3.4"
    printf 'apiVersion: v2\nname: ferroehr\nversion: 1.9.3\nappVersion: "4.3.3"\n' > "$stub/charts/1.9.3"
    printf 'apiVersion: v2\nname: ferroehr\nversion: 1.9.4\nappVersion: "4.3.4"\n' > "$stub/charts/1.9.4"
    local image
    for image in ferroehr ferroehr-viewer ferroehr-postgres; do
      mkdir -p "$stub/registry/$image"
      printf 'sha256:aaa\n' > "$stub/registry/$image/4.3.3"
      printf 'sha256:bbb\n' > "$stub/registry/$image/4.3.4"
      printf 'sha256:aaa\n' > "$stub/registry/$image/4.3"
      printf 'sha256:aaa\n' > "$stub/registry/$image/latest"
      printf 'sha256:ccc\n' > "$stub/registry/$image/4.2.9"
      printf 'sha256:ccc\n' > "$stub/registry/$image/4.2"
    done
    printf 'sha256:bbb\n' > "$stub/registry/ferroehr-viewer/latest"
  }
  # attempt WANT ARGS...: runs the script on the fixture and checks the exit.
  attempt() {
    local want="$1" status=0
    shift
    n=$((n + 1))
    PATH="$work/bin:$PATH" STUB="$stub" WITHDRAW_ROOT="$root" WITHDRAW_TODAY=2026-10-06 \
      "$BASH" "$0" "$@" > "$work/out" 2>&1 || status=$?
    if [ "$status" -ne "$want" ]; then
      echo "withdraw: self-test case $n ($*) exited $status, wanted $want:" >&2
      cat "$work/out" >&2
      failed=1
    fi
  }
  # check WHAT COMMAND...: records a failure when COMMAND fails.
  check() {
    local what="$1"
    shift
    if ! "$@"; then
      echo "withdraw: self-test: $what" >&2
      failed=1
    fi
  }
  # tag IMAGE TAG: the digest the stub registry holds for IMAGE:TAG.
  tag() {
    cat "$stub/registry/$1/$2"
  }
  local snapshot

  fixture
  attempt 1 4.3.1 --to 4.3.4
  check 'a version no row names is refused' grep -q 'no row in .* names 4.3.1' "$work/out"
  attempt 1 4.3.3 --to 4.3.2
  check 'a correcting release in the register is refused' grep -q 'itself named' "$work/out"
  attempt 1 4.3.3 --to 4.3.0
  check 'an earlier correcting release is refused' grep -q 'not later than' "$work/out"
  rm "$stub/releases/v4.3.4"
  attempt 1 4.3.3 --to 4.3.4
  check 'an unpublished correcting release is refused' grep -q 'not a published release' "$work/out"
  fixture
  attempt 1 4.3.3 --to 4.3.4 --chart 1.9.4
  check 'a chart that ships another version is refused' grep -q 'ships appVersion 4.3.4' "$work/out"
  attempt 2 4.3.3
  attempt 2 4.3.3 --to latest
  attempt 2 4.3.3 4.3.4 --to 4.3.5
  attempt 2 4.3.3 --to 4.3.4 --frobnicate
  check 'a refusal moves no tag' test "$(tag ferroehr 4.3)" = sha256:aaa

  fixture
  snapshot="$(cd "$stub/registry" && find . -type f -exec cksum {} + | sort)"
  attempt 0 v4.3.3 --to 4.3.4 --dry-run
  check 'a dry run changes nothing' test "$snapshot" = "$(cd "$stub/registry" && find . -type f -exec cksum {} + | sort)"
  check 'a dry run prints the tag move' grep -q '^+ docker buildx imagetools create -t ghcr.io/ferrohealth/ferroehr:4.3 ghcr.io/ferrohealth/ferroehr@sha256:bbb$' "$work/out"
  check 'a dry run prints the lookups' grep -q '^+ gh release view v4.3.3' "$work/out"
  check 'a dry run says so' grep -q 'dry run, nothing was changed' "$work/out"
  check 'a dry run runs no create' test -z "$(grep 'imagetools create' "$stub/calls" || true)"

  fixture
  attempt 0 4.3.3 --to 4.3.4
  check 'the line tag moves' test "$(tag ferroehr 4.3)" = sha256:bbb
  check 'latest moves' test "$(tag ferroehr-postgres latest)" = sha256:bbb
  check 'the viewer line tag moves' test "$(tag ferroehr-viewer 4.3)" = sha256:bbb
  check 'a latest pointing elsewhere is not touched' test -z "$(grep 'create -t ghcr.io/ferrohealth/ferroehr-viewer:latest' "$stub/calls" || true)"
  check 'the version tag stays' test "$(tag ferroehr 4.3.3)" = sha256:aaa
  check 'the chart version comes from the release notes' grep -q 'chart version 1.9.3' "$work/out"
  check 'the chart notice says it cannot be deprecated' grep -q 'cannot mark one chart version deprecated' "$work/out"
  # shellcheck disable=SC2016 # the backticks are Markdown, never expanded
  check 'the SECURITY.md line is printed' grep -q '^- `4.3.3`, withdrawn 2026-10-06 (N-2026-1): move to `4.3.4` or later.$' "$work/out"
  check 'the register columns are printed' grep -q 'corrected_in      4.3.4' "$work/out"
  check 'the finding summary is in the notice' grep -q 'A synthetic finding.' "$work/out"

  fixture
  attempt 0 4.2.9 --to 4.3.4
  check 'a line tag with no correcting release in its line stays' test "$(tag ferroehr 4.2)" = sha256:ccc
  check 'that tag is named for a manual move' grep -q '^  - ghcr.io/ferrohealth/ferroehr:4.2$' "$work/out"
  check 'the missing chart version is named' grep -q 'pass --chart' "$work/out"
  check 'nothing moved' grep -q 'none: every floating tag already pointed elsewhere' "$work/out"

  if [ "$failed" -ne 0 ]; then
    echo "withdraw: self-test FAILED" >&2
    exit 1
  fi
  echo "withdraw: self-test passed ($n runs)."
}

exec 3>&2
if [ "${1:-}" = --self-test ]; then
  [ $# -eq 1 ] || usage
  self_test
else
  run "$@"
fi
