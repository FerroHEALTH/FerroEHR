#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Every image of another FerroHEALTH product this repository pins, compared
# with that product's latest release. Dependabot does not read a pin by tag
# and digest in a table or a launcher, and scripts/checks/versions.sh, where
# it holds two files to one pin, cannot tell you that the product shipped a
# newer release, which is what this reads. No specification governs this:
# our own design, after FerroBRIDGE's lane of the same name.
#
#   scripts/checks/pin-freshness.sh
#   scripts/checks/pin-freshness.sh --self-test
#
# Reads every `ghcr.io/ferrohealth/<image>:<tag>@sha256:<digest>` reference
# in docs/VERSIONS.md, the compose files, the hosted sandbox and the Helm chart, the product that publishes the image, that
# product's newest stable GitHub release, and the digest of the image index
# GHCR serves for its tag. A release whose image is not published yet is
# reported PENDING. A line that carries a pin and `held: <reason>` records
# why the pin stays behind; such a pin is reported HELD and never counts as
# stale. Needs an authenticated `gh`, awk, curl and jq. `--self-test` runs
# the readers and the verdicts over fixtures and needs no network.
#
# Exit 0 when every pin is current or held, 1 when at least one is behind
# (each such line starts with STALE), 2 when a release or a digest could not
# be read, so a network failure never reads as a fresh pin.
set -euo pipefail
cd "$(dirname "$0")/../.."

# The images the other FerroHEALTH products publish, one
# "image<TAB>repository<TAB>release tag prefix" record per line: every image
# ships with a release of its repository, tagged the prefix and the version.
readonly FAMILY_IMAGES="\
ferrobridge	FerroBRIDGE	v
ferrobridge-console	FerroBRIDGE	v
ferroehr	FerroEHR	v
ferroehr-postgres	FerroEHR	v
ferroehr-viewer	FerroEHR	v
ferrofed	FerroFED	v
ferrofed-console	FerroFED	v
ferropix	FerroPIX	v
ferropix-console	FerroPIX	v
ferrosmart	FerroSMART	ferrosmart-v
ferroterm	FerroTERM	v
ferroterm-console	FerroTERM	v
ferroterm-sync	FerroTERM	v"
readonly GHCR_INDEX='application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json'

# family_pins FILE...: one "image<TAB>tag<TAB>digest<TAB>files<TAB>held reason"
# record per distinct `ghcr.io/ferrohealth/<image>:<tag>@sha256:<digest>`
# reference in FILE, sorted, with the files that carry it joined by ", " and
# the text after a `held:` on a line that carries it, up to a cell boundary.
family_pins() {
  awk '
    {
      line = $0; held = ""
      if (match(line, /held: *[^|]*/)) {
        held = substr(line, RSTART, RLENGTH); sub(/^held: */, "", held); sub(/[ \t]+$/, "", held)
      }
      while (match(line, /ghcr\.io\/ferrohealth\/[a-z0-9-]+:[A-Za-z0-9._-]+@sha256:[0-9a-f]{64}/)) {
        ref = substr(line, RSTART + 20, RLENGTH - 20); line = substr(line, RSTART + RLENGTH)
        name = ref; sub(/:.*$/, "", name)
        tag = ref; sub(/^[^:]*:/, "", tag); sub(/@.*$/, "", tag)
        digest = ref; sub(/^.*@/, "", digest)
        key = name "\t" tag "\t" digest
        if (!(key in files)) { files[key] = FILENAME; keys[++n] = key }
        else if (index(", " files[key] ", ", ", " FILENAME ", ") == 0) { files[key] = files[key] ", " FILENAME }
        if (held != "" && !(key in reason)) reason[key] = held
      }
    }
    END { for (i = 1; i <= n; i++) print keys[i] "\t" files[keys[i]] "\t" reason[keys[i]] }
  ' "$@" | LC_ALL=C sort
}

# family_newest PREFIX: the highest stable version among the release tags on
# stdin that read PREFIX followed by a dotted version, without the prefix.
family_newest() {
  awk -v prefix="$1" '
    index($0, prefix) == 1 {
      version = substr($0, length(prefix) + 1)
      if (version ~ /^[0-9]+(\.[0-9]+)*$/) print version
    }' | sort -V | tail -n 1
}

# family_verdict IMAGE PINNED_TAG PINNED_DIGEST FILES HELD REPO RELEASE_TAG
# VERSION DIGEST: the line one product image pin gets against the latest
# release of REPO, tagged RELEASE_TAG for VERSION, whose image index GHCR
# serves at DIGEST. A pin at the latest version and digest is current; a pin
# behind it is STALE, or HELD when its line records a reason.
family_verdict() {
  local image="$1" pinned_tag="$2" pinned_digest="$3" files="$4" held="$5"
  local repo="$6" release_tag="$7" version="$8" digest="$9"
  local release="https://github.com/FerroHEALTH/$repo/releases/tag/$release_tag"
  if [[ "$pinned_tag" = "$version" && "$pinned_digest" = "$digest" ]]; then
    printf 'current    ghcr.io/ferrohealth/%s %s (%s %s; %s)\n' "$image" "$pinned_tag" "$repo" "$release_tag" "$files"
  elif [[ -n "$held" ]]; then
    printf 'HELD       ghcr.io/ferrohealth/%s: pinned %s in %s, latest release %s %s, held: %s\n' \
      "$image" "$pinned_tag" "$files" "$repo" "$release_tag" "$held"
  elif [[ "$pinned_tag" = "$version" ]]; then
    printf 'STALE      ghcr.io/ferrohealth/%s: pinned %s@%s in %s, but GHCR serves the index of %s at %s now (%s)\n' \
      "$image" "$pinned_tag" "$pinned_digest" "$files" "$version" "$digest" "$release"
  else
    printf 'STALE      ghcr.io/ferrohealth/%s: pinned %s in %s, latest release %s %s (%s) at %s@%s; check the Upgrade notes, Changed and Removed sections of every %s release after %s for configuration the pin moves with (https://github.com/FerroHEALTH/%s/releases)\n' \
      "$image" "$pinned_tag" "$files" "$repo" "$release_tag" "$release" "$version" "$digest" "$repo" "$pinned_tag" "$repo"
  fi
}

# ghcr_digest IMAGE TAG: the digest of the image index GHCR serves for
# ghcr.io/ferrohealth/IMAGE:TAG, read anonymously, or nothing when the tag is
# not published; fails when the registry could not be read.
ghcr_digest() {
  local image="$1" tag="$2" token headers status
  token="$(curl --proto '=https' --tlsv1.2 -fsS \
    "https://ghcr.io/token?scope=repository:ferrohealth/$image:pull" | jq -er .token)" || return 1
  headers="$(curl --proto '=https' --tlsv1.2 -sS -I -H "Authorization: Bearer $token" \
    -H "Accept: $GHCR_INDEX" "https://ghcr.io/v2/ferrohealth/$image/manifests/$tag" | tr -d '\r')" || return 1
  status="$(awk 'NR == 1 { print $2 }' <<< "$headers")"
  case "$status" in
    200) awk 'tolower($1) == "docker-content-digest:" { print $2 }' <<< "$headers" | grep -E '^sha256:[0-9a-f]{64}$' ;;
    404) ;;
    *) return 1 ;;
  esac
}

# family_check FILE...: compares every product image pin of FILE with the
# latest release of the product that publishes it; images of SELF_REPO are
# this repository's own and are skipped. Sets stale and unreadable.
family_check() {
  local pins image tag digest files held record repo prefix tags version newest line
  pins="$(family_pins "$@")"
  while IFS=$'\t' read -r image tag digest files held; do
    [[ -n "$image" ]] || continue
    record="$(awk -F'\t' -v want="$image" '$1 == want { print $2 "\t" $3; exit }' <<< "$FAMILY_IMAGES")"
    repo="${record%%$'\t'*}"
    prefix="${record#*$'\t'}"
    if [[ -z "$repo" ]]; then
      printf 'UNREADABLE ghcr.io/ferrohealth/%s: pinned in %s, but no product of FAMILY_IMAGES publishes it\n' "$image" "$files"
      unreadable=1
      continue
    fi
    [[ "$repo" != "$SELF_REPO" ]] || continue
    if ! tags="$(gh api --paginate "repos/FerroHEALTH/$repo/releases?per_page=100" \
      --jq '.[] | select(.draft == false and .prerelease == false) | .tag_name' 2>&1)"; then
      printf 'UNREADABLE ghcr.io/ferrohealth/%s: could not read the releases of FerroHEALTH/%s (%s)\n' "$image" "$repo" "$tags"
      unreadable=1
      continue
    fi
    version="$(family_newest "$prefix" <<< "$tags")"
    if [[ -z "$version" ]]; then
      printf 'UNREADABLE ghcr.io/ferrohealth/%s: FerroHEALTH/%s has no release tagged %s<version>\n' "$image" "$repo" "$prefix"
      unreadable=1
      continue
    fi
    if ! newest="$(ghcr_digest "$image" "$version")"; then
      printf 'UNREADABLE ghcr.io/ferrohealth/%s: could not read the digest of %s from GHCR\n' "$image" "$version"
      unreadable=1
      continue
    fi
    if [[ -z "$newest" ]]; then
      printf 'PENDING    ghcr.io/ferrohealth/%s: pinned %s in %s, latest release %s %s%s has no published image yet\n' \
        "$image" "$tag" "$files" "$repo" "$prefix" "$version"
      continue
    fi
    line="$(family_verdict "$image" "$tag" "$digest" "$files" "$held" "$repo" "$prefix$version" "$version" "$newest")"
    printf '%s\n' "$line"
    [[ "$line" != STALE* ]] || stale=1
  done <<< "$pins"
}

# The files whose product image pins this reads, and the repository whose
# own images it skips.
readonly FAMILY_SOURCES=(docs/VERSIONS.md docker-compose.yml docker-compose.ferrosmart.yml docker-compose.terminology.yml docker/sut-ferroehr.yml deploy/hosted/docker-compose.yml deploy/helm/ferroehr/Chart.yaml)
readonly SELF_REPO=FerroEHR

# Runs the readers and the verdicts over fixtures, with no network, and
# checks that every source file names at least one product image pin.
self_test() {
  local failures=0 got source
  expect() {
    local expected="$1" actual="$2" case_label="$3"
    if [[ "$actual" != "$expected" ]]; then
      printf 'pin-freshness: self-test: %s gave "%s", wanted "%s"\n' "$case_label" "$actual" "$expected" >&2
      failures=$((failures + 1))
    fi
  }
  local fixture d1 d2
  fixture="$(mktemp)"
  d1="sha256:$(printf '1%.0s' {1..64})"
  d2="sha256:$(printf '2%.0s' {1..64})"
  printf '%s\n' \
    "| A | \`ghcr.io/ferrohealth/ferroterm:0.4.2@$d1\` | x |" \
    "    image: \${FERROPIX_IMAGE:-ghcr.io/ferrohealth/ferropix:0.0.26@$d2}  # held: the PIX configuration" \
    "| B | \`ghcr.io/ferrohealth/ferroterm:0.4.2@$d1\` | y |" \
    "    image: ghcr.io/ferrohealth/ferrobridge:\${FERROBRIDGE_VERSION:-0.0.27}" > "$fixture"
  got="$(family_pins "$fixture" | cut -f1,2,5 | tr '\t' ' ' | paste -sd ';' -)"
  expect "ferropix 0.0.26 the PIX configuration;ferroterm 0.4.2 " "$got" "the product image pins of a file"
  got="$(family_pins "$fixture" "$fixture" | awk -F'\t' '$1 == "ferroterm" { print $4 }')"
  expect "$fixture" "$got" "a file named once for a pin it carries twice"
  rm -f "$fixture"
  got="$(printf '%s\n' v0.0.9 v0.0.10 v0.0.11-rc1 ferrohealth-smart-v0.33.1 | family_newest v)"
  expect "0.0.10" "$got" "the newest stable release of a v-tagged product"
  got="$(printf '%s\n' ferrohealth-smart-v0.33.1 ferrosmart-v0.3.0 ferrosmart-v0.2.0 | family_newest ferrosmart-v)"
  expect "0.3.0" "$got" "the newest server release among crate releases"
  got="$(family_verdict ferroterm 0.4.7 "$d1" f "" FerroTERM v0.4.7 0.4.7 "$d1" | cut -c1-10)"
  expect "current   " "$got" "a pin at the latest release and digest"
  got="$(family_verdict ferroterm 0.4.7 "$d1" f "" FerroTERM v0.4.7 0.4.7 "$d2" | cut -c1-10)"
  expect "STALE     " "$got" "a pin whose tag now serves another digest"
  got="$(family_verdict ferroterm 0.4.2 "$d1" f "" FerroTERM v0.4.7 0.4.7 "$d2")"
  case "$got" in
    "STALE      ghcr.io/ferrohealth/ferroterm: pinned 0.4.2 in f, latest release FerroTERM v0.4.7 "*"Upgrade notes"*) ;;
    *) expect "a STALE line naming the release and the upgrade notes" "$got" "a pin behind the latest release" ;;
  esac
  got="$(family_verdict ferroterm 0.4.2 "$d1" f "a reason" FerroTERM v0.4.7 0.4.7 "$d2" | cut -c1-10)"
  expect "HELD      " "$got" "a held pin behind the latest release"
  for source in "${FAMILY_SOURCES[@]}"; do
    got="$(family_pins "$source" | wc -l | tr -d ' ')"
    [[ "$got" -gt 0 ]] || expect "at least one pin" "none" "the product image pins of $source"
  done
  if [[ "$failures" -gt 0 ]]; then
    echo "pin-freshness: self-test: $failures check(s) failed" >&2
    exit 1
  fi
  echo "pin-freshness: self-test passed"
}

case "${1:-}" in
  "") ;;
  --self-test)
    self_test
    exit 0
    ;;
  *)
    echo "usage: scripts/checks/pin-freshness.sh [--self-test]" >&2
    exit 2
    ;;
esac

stale=0
unreadable=0
family_check "${FAMILY_SOURCES[@]}"
[[ "$unreadable" -eq 0 ]] || exit 2
exit "$stale"
