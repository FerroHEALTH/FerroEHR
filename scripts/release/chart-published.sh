#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# Compares a chart package already published under a version with the
# package a run built for the same version, by their unpacked contents (#4331).
# `helm package` stamps each tar entry with the packaging time, so two packages
# of the same chart differ in bytes and a digest comparison would refuse every
# re-run. build-chart.yml refuses a publish only when the contents differ, and
# signs and attests the published digest when they match, so a chart leg that
# pushed and then failed its signature, an attestation or the ownership tag is
# finished by a re-run. No openEHR spec governs this: our own design.
#
#   scripts/release/chart-published.sh compare <published.tgz> <packaged.tgz>
#       Exit 0 when both packages unpack to the same files with the same
#       bytes. Exit 1 naming each file that is missing on one side or
#       differs, without printing the files themselves.
#   scripts/release/chart-published.sh --self-test
#       Proves a repackaged chart matches and a changed, added or removed
#       file is refused, offline, with fixture packages.
#
# Needs tar, gzip, diff and cmp. Exit 2 on a usage error.
set -euo pipefail

usage() {
  echo "usage: scripts/release/chart-published.sh compare <published.tgz> <packaged.tgz> | --self-test" >&2
  exit 2
}

die() {
  echo "::error::chart-published: $*" >&2
  exit 1
}

# unpack PACKAGE DIR: the package's files under DIR, or a refusal naming it.
unpack() {
  mkdir -p "$2"
  tar -xzf "$1" -C "$2" --no-same-owner --no-same-permissions 2>/dev/null \
    || die "$1 is not a readable chart package"
}

# files DIR: every regular file and symbolic link under DIR, relative, sorted.
files() {
  (cd "$1" && find . \( -type f -o -type l \) -print | LC_ALL=C sort)
}

# compare PUBLISHED PACKAGED, in a subshell of its own, so a refusal ends the
# comparison alone and the trap removes its directory.
compare() (
  local published="$1" packaged="$2" work faults=0 path
  [[ -f "$published" ]] || die "$published does not exist"
  [[ -f "$packaged" ]] || die "$packaged does not exist"
  work="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand work now, so the trap removes this directory
  trap "rm -r '$work'" EXIT
  unpack "$published" "$work/published"
  unpack "$packaged" "$work/packaged"
  files "$work/published" > "$work/published.list"
  files "$work/packaged" > "$work/packaged.list"
  while IFS= read -r path; do
    echo "::error::chart-published: ${path#./} is only in the published chart" >&2
    faults=1
  done < <(LC_ALL=C comm -23 "$work/published.list" "$work/packaged.list")
  while IFS= read -r path; do
    echo "::error::chart-published: ${path#./} is only in the chart this run packaged" >&2
    faults=1
  done < <(LC_ALL=C comm -13 "$work/published.list" "$work/packaged.list")
  while IFS= read -r path; do
    if ! cmp -s "$work/published/$path" "$work/packaged/$path"; then
      echo "::error::chart-published: ${path#./} differs from the published chart" >&2
      faults=1
    fi
  done < <(LC_ALL=C comm -12 "$work/published.list" "$work/packaged.list")
  if [[ "$faults" -ne 0 ]]; then
    die "$(basename "$packaged") differs from the chart published under the same version"
  fi
  echo "chart-published: $(basename "$packaged") holds the same files as the published chart."
)

self_test() {
  local work refusal
  work="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand work now, so the trap removes this directory
  trap "rm -r '$work'" EXIT
  mkdir -p "$work/chart/demo/templates"
  printf 'apiVersion: v2\nname: demo\nversion: 0.1.0\n' > "$work/chart/demo/Chart.yaml"
  printf 'image:\n  digest: sha256:aa\n' > "$work/chart/demo/values.yaml"
  printf 'kind: ConfigMap\n' > "$work/chart/demo/templates/cm.yaml"
  tar -czf "$work/published.tgz" -C "$work/chart" demo
  # The same files packaged a second later: other mtimes, other bytes.
  touch -t 202001010000 "$work/chart/demo/Chart.yaml"
  tar -czf "$work/repackaged.tgz" -C "$work/chart" demo
  if cmp -s "$work/published.tgz" "$work/repackaged.tgz"; then
    echo "self-test: the fixture packages should differ in bytes" >&2
    exit 1
  fi
  if ! compare "$work/published.tgz" "$work/repackaged.tgz" > /dev/null 2>&1; then
    echo "self-test: a repackaged chart with the same files was refused" >&2
    exit 1
  fi
  printf 'image:\n  digest: sha256:bb\n' > "$work/chart/demo/values.yaml"
  tar -czf "$work/changed.tgz" -C "$work/chart" demo
  if compare "$work/published.tgz" "$work/changed.tgz" > /dev/null 2>&1; then
    echo "self-test: a chart with a changed file was accepted" >&2
    exit 1
  fi
  refusal="$(compare "$work/published.tgz" "$work/changed.tgz" 2>&1 || true)"
  if ! grep -q 'demo/values.yaml differs' <<<"$refusal"; then
    echo "self-test: the refusal of a changed file does not name it" >&2
    exit 1
  fi
  printf 'image:\n  digest: sha256:aa\n' > "$work/chart/demo/values.yaml"
  printf 'kind: Secret\n' > "$work/chart/demo/templates/extra.yaml"
  tar -czf "$work/added.tgz" -C "$work/chart" demo
  if compare "$work/published.tgz" "$work/added.tgz" > /dev/null 2>&1; then
    echo "self-test: a chart with an added file was accepted" >&2
    exit 1
  fi
  if compare "$work/added.tgz" "$work/published.tgz" > /dev/null 2>&1; then
    echo "self-test: a chart with a removed file was accepted" >&2
    exit 1
  fi
  printf 'not a package\n' > "$work/broken.tgz"
  if compare "$work/broken.tgz" "$work/published.tgz" > /dev/null 2>&1; then
    echo "self-test: an unreadable package was accepted" >&2
    exit 1
  fi
  echo "chart-published: self-test OK (a repackaged chart matches; a changed, added or removed file and an unreadable package are refused)."
}

case "${1:-}" in
  compare)
    [[ $# -eq 3 ]] || usage
    compare "$2" "$3"
    ;;
  --self-test)
    [[ $# -eq 1 ]] || usage
    self_test
    ;;
  *) usage ;;
esac
