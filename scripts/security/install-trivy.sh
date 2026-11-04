#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# Installs the pinned Trivy on a Linux x86_64 runner from its GitHub release
# tarball, verified against the sha256 below, and puts it on GITHUB_PATH.
#
#   scripts/security/install-trivy.sh <version> <bindir>
#   scripts/security/install-trivy.sh --sha256 <version>   # print the pin, or fail
#
# Trivy's own install script fetches the tarball from get.trivy.dev, which
# stopped resolving on 2026-11-03 (#1548), so the lanes take the release asset
# directly. A version with no row below is refused: a bump adds its row, read
# from the release's trivy_<version>_checksums.txt. No specification governs
# this: our own design.
set -euo pipefail

# One "version<TAB>sha256 of trivy_<version>_Linux-64bit.tar.gz" row per line.
readonly CHECKSUMS="\
0.73.0	2edd39da482bb4e9831962487b68f68e3928ec3137794757f54d00383d79547b"

die() {
  printf 'install-trivy: %s\n' "$1" >&2
  exit 1
}

pinned_sha256() {
  local asset="$1"
  awk -F'\t' -v want="$asset" '$1 == want { print $2; found = 1 } END { exit !found }' \
    <<<"$CHECKSUMS"
}

if [[ "${1:-}" == "--sha256" ]]; then
  [[ $# -eq 2 ]] || die "usage: $0 --sha256 <version>"
  pinned_sha256 "$2" || die "no sha256 is pinned for Trivy $2"
  exit 0
fi

[[ $# -eq 2 ]] || die "usage: $0 <version> <bindir>"
version="$1"
bindir="$2"

want="$(pinned_sha256 "$version")" || die "no sha256 is pinned for Trivy $version"
[[ "$(uname -s)" == "Linux" && "$(uname -m)" == "x86_64" ]] \
  || die "only the Linux x86_64 tarball is pinned, and this is $(uname -s) $(uname -m)"

tarball="trivy_${version}_Linux-64bit.tar.gz"
url="https://github.com/aquasecurity/trivy/releases/download/v${version}/${tarball}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
  --retry 3 --output "$work/$tarball" "$url"
got="$(sha256sum "$work/$tarball" | cut -d' ' -f1)"
[[ "$got" == "$want" ]] || die "$tarball has sha256 $got, and $want is pinned"

mkdir -p "$bindir"
tar -xzf "$work/$tarball" -C "$bindir" trivy
"$bindir/trivy" --version
if [[ -n "${GITHUB_PATH:-}" ]]; then
  printf '%s\n' "$bindir" >>"$GITHUB_PATH"
fi
