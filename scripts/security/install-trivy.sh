#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# Installs the pinned Trivy from its GitHub release tarball, verified against
# the sha256 below, on a Linux x86_64 or arm64 host or a macOS arm64 one, and
# puts it on GITHUB_PATH when a workflow runs it.
#
#   scripts/security/install-trivy.sh <version> <bindir>
#   scripts/security/install-trivy.sh --sha256 <version> [<platform>]   # print the pin, or fail
#   scripts/security/install-trivy.sh --check   # the workflows pin one version, pinned for every platform
#
# <platform> is the release asset's platform: Linux-64bit, Linux-ARM64 or
# macOS-ARM64 (default: this host's). Trivy's own install script fetches the
# tarball from get.trivy.dev, which stopped resolving on 2026-11-03 (#1548), so
# the lanes take the release asset directly. A version or platform with no row
# below is refused: a bump adds one row per platform, read from the release's
# trivy_<version>_checksums.txt. The same pinned version serves CI and the
# local rerun of the image scan (.claude/rules/image-security.md, #6743). No
# specification governs this: our own design.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
readonly ROOT

# One "version<TAB>platform<TAB>sha256 of trivy_<version>_<platform>.tar.gz" row per line.
readonly CHECKSUMS="\
0.73.0	Linux-64bit	2edd39da482bb4e9831962487b68f68e3928ec3137794757f54d00383d79547b
0.73.0	Linux-ARM64	13833d97e8a1a5367471c372a173180157f593bece570e20d5d925fef552f5dd
0.73.0	macOS-ARM64	80cc25faaf6378e37701202d0b4f9f43d9e413d198d594ba60fdf559fe44a683"
readonly PLATFORMS="Linux-64bit Linux-ARM64 macOS-ARM64"

die() {
  printf 'install-trivy: %s\n' "$1" >&2
  exit 1
}

# host_platform: the release asset platform of this host, or fails.
host_platform() {
  case "$(uname -s) $(uname -m)" in
    "Linux x86_64") echo Linux-64bit ;;
    "Linux aarch64" | "Linux arm64") echo Linux-ARM64 ;;
    "Darwin arm64") echo macOS-ARM64 ;;
    *) die "no Trivy tarball is pinned for $(uname -s) $(uname -m); the pinned platforms are $PLATFORMS" ;;
  esac
}

# pinned_sha256 <version> <platform>
pinned_sha256() {
  awk -F'\t' -v version="$1" -v platform="$2" \
    '$1 == version && $2 == platform { print $3; found = 1 } END { exit !found }' <<<"$CHECKSUMS"
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# check: every TRIVY_VERSION a workflow sets is one version, pinned here for
# every platform.
check() {
  local versions version platform bad=0
  versions="$(grep -hoE 'TRIVY_VERSION: "[^"]+"' "$ROOT"/.github/workflows/*.yml \
    | sed -E 's/.*"([^"]+)"/\1/' | sort -u)"
  [[ -n "$versions" ]] || die "no workflow sets TRIVY_VERSION"
  if [[ "$(grep -c . <<<"$versions")" -ne 1 ]]; then
    die "the workflows pin more than one Trivy: $(paste -sd' ' - <<<"$versions")"
  fi
  version="$versions"
  for platform in $PLATFORMS; do
    if ! pinned_sha256 "$version" "$platform" >/dev/null; then
      printf 'install-trivy: no sha256 is pinned for Trivy %s on %s\n' "$version" "$platform" >&2
      bad=1
    fi
  done
  [[ "$bad" -eq 0 ]] || exit 1
  echo "install-trivy: the workflows pin Trivy $version, and its tarball is pinned for $PLATFORMS"
}

case "${1:-}" in
  --check)
    [[ $# -eq 1 ]] || die "usage: $0 --check"
    check
    exit 0
    ;;
  --sha256)
    [[ $# -eq 2 || $# -eq 3 ]] || die "usage: $0 --sha256 <version> [<platform>]"
    platform="${3:-$(host_platform)}"
    pinned_sha256 "$2" "$platform" || die "no sha256 is pinned for Trivy $2 on $platform"
    exit 0
    ;;
  *) ;;
esac

[[ $# -eq 2 ]] || die "usage: $0 <version> <bindir>"
version="$1"
bindir="$2"

platform="$(host_platform)"
want="$(pinned_sha256 "$version" "$platform")" || die "no sha256 is pinned for Trivy $version on $platform"

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
