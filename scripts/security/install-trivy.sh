#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# Installs the pinned Trivy from its GitHub release tarball for this machine
# (Linux or macOS, x86_64 or arm64), verified against the sha256 below, and
# puts it on GITHUB_PATH when that is set.
#
#   scripts/security/install-trivy.sh <version> <bindir>
#   scripts/security/install-trivy.sh --sha256 <version> [<platform>]  # print the pin, or fail
#   scripts/security/install-trivy.sh --pinned   # the version every workflow pins
#   scripts/security/install-trivy.sh --check    # the workflows agree, every platform pinned
#
# Trivy's own install script fetches the tarball from get.trivy.dev, which
# stopped resolving on 2026-11-03 (#1548), so the lanes take the release asset
# directly. A version or platform with no row below is refused: a bump adds one
# row per platform, read from the release's trivy_<version>_checksums.txt. No
# specification governs this: our own design.
set -euo pipefail

# One "version<TAB>platform<TAB>sha256 of trivy_<version>_<platform>.tar.gz" row per line.
readonly CHECKSUMS="\
0.73.0	Linux-64bit	2edd39da482bb4e9831962487b68f68e3928ec3137794757f54d00383d79547b
0.73.0	Linux-ARM64	13833d97e8a1a5367471c372a173180157f593bece570e20d5d925fef552f5dd
0.73.0	macOS-64bit	d39d1374dd3e35d48621b82df9b6625fe69f9920cc67d2739ed81bb679f16f51
0.73.0	macOS-ARM64	80cc25faaf6378e37701202d0b4f9f43d9e413d198d594ba60fdf559fe44a683"
readonly PLATFORMS="Linux-64bit Linux-ARM64 macOS-64bit macOS-ARM64"
WORKFLOWS="$(dirname "$0")/../../.github/workflows"
readonly WORKFLOWS

die() {
  printf 'install-trivy: %s\n' "$1" >&2
  exit 1
}

pinned_sha256() {
  local version="$1" platform="$2"
  awk -F'\t' -v v="$version" -v p="$platform" \
    '$1 == v && $2 == p { print $3; found = 1 } END { exit !found }' <<<"$CHECKSUMS"
}

# The release asset's platform name for this machine.
this_platform() {
  local os arch
  case "$(uname -s)" in
    Linux) os=Linux ;;
    Darwin) os=macOS ;;
    *) die "no Trivy tarball is pinned for $(uname -s)" ;;
  esac
  case "$(uname -m)" in
    x86_64 | amd64) arch=64bit ;;
    aarch64 | arm64) arch=ARM64 ;;
    *) die "no Trivy tarball is pinned for $(uname -m)" ;;
  esac
  printf '%s-%s\n' "$os" "$arch"
}

sha256_of() {
  if command -v sha256sum >/dev/null; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# The one TRIVY_VERSION the workflows pin, or a failure naming the disagreement.
workflow_version() {
  local found
  found="$(grep -rhoE 'TRIVY_VERSION: "[^"]+"' "$WORKFLOWS" | sort -u)" \
    || die "no workflow under .github/workflows pins TRIVY_VERSION"
  [[ "$(wc -l <<<"$found")" -eq 1 ]] \
    || die "the workflows pin more than one TRIVY_VERSION: $(tr '\n' ' ' <<<"$found")"
  sed -E 's/.*"([^"]+)"/\1/' <<<"$found"
}

case "${1:-}" in
  --sha256)
    [[ $# -eq 2 || $# -eq 3 ]] || die "usage: $0 --sha256 <version> [<platform>]"
    platform="${3:-$(this_platform)}"
    pinned_sha256 "$2" "$platform" || die "no sha256 is pinned for Trivy $2 on $platform"
    exit 0
    ;;
  --pinned)
    [[ $# -eq 1 ]] || die "usage: $0 --pinned"
    workflow_version
    exit 0
    ;;
  --check)
    [[ $# -eq 1 ]] || die "usage: $0 --check"
    version="$(workflow_version)"
    for platform in $PLATFORMS; do
      pinned_sha256 "$version" "$platform" >/dev/null \
        || die "the workflows pin Trivy $version, and no sha256 is pinned for $platform"
    done
    echo "install-trivy: every workflow pins Trivy $version, pinned for $PLATFORMS"
    exit 0
    ;;
  *) ;;
esac

[[ $# -eq 2 ]] || die "usage: $0 <version> <bindir>"
version="$1"
bindir="$2"

platform="$(this_platform)"
want="$(pinned_sha256 "$version" "$platform")" \
  || die "no sha256 is pinned for Trivy $version on $platform"

tarball="trivy_${version}_${platform}.tar.gz"
url="https://github.com/aquasecurity/trivy/releases/download/v${version}/${tarball}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
  --retry 3 --output "$work/$tarball" "$url"
got="$(sha256_of "$work/$tarball")"
[[ "$got" == "$want" ]] || die "$tarball has sha256 $got, and $want is pinned"

mkdir -p "$bindir"
tar -xzf "$work/$tarball" -C "$bindir" trivy
"$bindir/trivy" --version
if [[ -n "${GITHUB_PATH:-}" ]]; then
  printf '%s\n' "$bindir" >>"$GITHUB_PATH"
fi
