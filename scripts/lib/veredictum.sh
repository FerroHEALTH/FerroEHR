#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The pinned conformance instrument: Veredictum.
#
# The acceptance instrument is an independent project with its own release
# line, consumed here at the pin VEREDICTUM_VERSION records. Everything that
# runs it sources this file.
#
# Three forms of one release, each obtained the way the publisher ships it:
#
#   the IMAGE   ghcr.io/vernum-projecten/veredictum, pinned below by the digest
#               of its multi-platform index. It carries the engine, the
#               catalogue (/work/catalogue) and the vendored spec oracle
#               (/work/specs/openehr), so `validate`, `test` and `export` run
#               in it with nothing else fetched. scripts/conformance.sh uses it.
#   the DATA    the repository at the tag of the pin, a shallow cached clone,
#               for what the image does not carry: the performance schedule
#               (`perf/`) the measured lanes read.
#   the BINARY  `cargo install veredictum`, from crates.io when the release is
#               published there and from the repository tag otherwise, for
#               the lanes that run on the host beside the clone.
#
# The clone and the binary are cached under one version-keyed root, and both
# resolvers are idempotent: a warm machine re-runs them for the cost of a check.
#
# Overrides, for an offline machine or a contributor working on the instrument
# itself (each is used verbatim and never fetched over):
#   VEREDICTUM_BIN    path to an existing `veredictum` binary
#   VEREDICTUM_ROOT   path to an existing Veredictum checkout
#   VEREDICTUM_CACHE  cache root (default: $XDG_CACHE_HOME/ferroehr-veredictum,
#                     falling back to ~/.cache/ferroehr-veredictum)
#   CONTAINER_ENGINE  `docker` or `podman` (default: docker when it is on PATH,
#                     else podman)

# The pin. A bump moves the version, the image digest and the Veredictum Action
# step of .github/workflows/conformance.yml (its commit SHA, its version
# comment and its `image:`) together: the digest is the one `docker buildx
# imagetools inspect` reports for the tag's index, and
# scripts/checks/veredictum-action.sh fails while the three disagree (#4449).
VEREDICTUM_VERSION="0.3.3"
VEREDICTUM_REPO="https://github.com/Vernum-Projecten/Veredictum"
# shellcheck disable=SC2034 # read by the scripts that source this file
VEREDICTUM_IMAGE="ghcr.io/vernum-projecten/veredictum@sha256:df50feb4df9de262be64502ff999d0bbe784ba623ffbfa32313b52769a594efd"

# The release the committed record under docs/conformance/ferroehr/ was
# produced with. Its renderers (scripts/render/perf-assets.sh and
# conformance-assets.sh) read that record's shape, which no later release
# reads, so they run this release until the record of the current pin replaces
# it (#4451, #4452).
VEREDICTUM_RECORD_VERSION="0.1.6"

# Echoes the container CLI the image runs under.
veredictum_engine() {
  if [[ -n "${CONTAINER_ENGINE:-}" ]]; then
    printf '%s' "$CONTAINER_ENGINE"
  elif command -v docker >/dev/null 2>&1; then
    printf 'docker'
  elif command -v podman >/dev/null 2>&1; then
    printf 'podman'
  else
    echo "veredictum: neither docker nor podman is on PATH" >&2
    return 1
  fi
}

# Each resolver below takes an optional release (default: the pin).
veredictum_cache_root() {
  local base="${VEREDICTUM_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/ferroehr-veredictum}"
  printf '%s/%s' "$base" "${1:-$VEREDICTUM_VERSION}"
}

# Echoes the path to the pinned binary, installing it on a cache miss.
#
# `cargo install --root` keeps the install out of ~/.cargo/bin, so a machine
# working on several pins never has one shadow another, and the version check
# below is a fact about THIS pin rather than about whatever is on PATH.
veredictum_bin() {
  if [[ -n "${VEREDICTUM_BIN:-}" ]]; then
    printf '%s' "$VEREDICTUM_BIN"
    return 0
  fi
  local version="${1:-$VEREDICTUM_VERSION}" root bin
  root="$(veredictum_cache_root "$version")/install"
  bin="$root/bin/veredictum"
  local reported=""
  [[ ! -x "$bin" ]] || reported="$("$bin" --version 2>/dev/null || true)"
  if [[ "$reported" != "veredictum $version" ]] && [[ "$reported" != "veredictum $version "* ]]; then
    echo "==> Installing veredictum $version (cargo install, cached in $root)" >&2
    # Both sources build the same tagged release; the tag serves a release the
    # registry does not carry.
    cargo install veredictum --version "$version" --locked --root "$root" >&2 ||
      cargo install veredictum --git "$VEREDICTUM_REPO" --tag "v$version" \
        --locked --root "$root" >&2
  fi
  printf '%s' "$bin"
}

# Echoes the path to the pinned checkout, cloning it on a cache miss.
#
# `--depth 1 --branch <tag>` fetches the one commit the pin names. A stamp file
# written after the clone succeeds is what marks the cache warm, so a directory
# left half-written by an interrupted clone is replaced rather than trusted.
veredictum_root() {
  if [[ -n "${VEREDICTUM_ROOT:-}" ]]; then
    printf '%s' "$VEREDICTUM_ROOT"
    return 0
  fi
  local version="${1:-$VEREDICTUM_VERSION}" root tag stamp
  root="$(veredictum_cache_root "$version")/repo"
  tag="v$version"
  stamp="$root/.ferroehr-pin"
  if [[ "$(cat "$stamp" 2>/dev/null || true)" != "$tag" ]]; then
    echo "==> Fetching Veredictum at $tag (shallow clone, cached in $root)" >&2
    rm -rf "${root:?}"
    mkdir -p "$(dirname "$root")"
    git -c advice.detachedHead=false clone --quiet --depth 1 --branch "$tag" \
      "$VEREDICTUM_REPO" "$root" >&2
    printf '%s\n' "$tag" > "$stamp"
  fi
  printf '%s' "$root"
}

# The conformance catalogue the `--root` flag of `validate` and `test` takes.
veredictum_catalogue() { printf '%s/catalogue' "$(veredictum_root)"; }

# The vendored spec oracle the `--specs` flag takes.
veredictum_specs() { printf '%s/specs/openehr' "$(veredictum_root)"; }

# The performance schedule the `--root` flag of the measured commands takes.
veredictum_perf() { printf '%s/perf' "$(veredictum_root)"; }

# The case-core catalogue of the record's release, which its renderers take as
# `--root`.
veredictum_record_artifacts() { printf '%s/artifacts' "$(veredictum_root "$VEREDICTUM_RECORD_VERSION")"; }
