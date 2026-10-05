#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The pinned conformance instrument: Veredictum.
#
# The acceptance instrument is no longer a sibling crate in this workspace. It
# is an independent project with its own release line, consumed here at a PIN
# recorded in exactly one place — VEREDICTUM_VERSION below. Everything that
# runs the catalogue sources this file and calls the two resolvers.
#
# Two halves, obtained separately because the publisher separates them:
#
#   the BINARY  `cargo install veredictum --version <pin> --locked`, from
#               crates.io. The published package carries source only; its
#               manifest `include` list is an allowlist of code and the legal
#               set, because the catalogue and the vendored spec oracle are
#               hundreds of megabytes of data no registry accepts.
#   the DATA    the catalogue (`artifacts/`) and the spec oracle
#               (`specs/openehr`, with the schema bundles beside it) come from
#               the repository at the tag matching the pin — a shallow, cached
#               checkout. There is no data tarball on the release, and the
#               release binaries are Linux-only, so this is the one path that
#               works on every machine that runs the pipeline.
#
# Both halves are cached under one version-keyed root, and both resolvers are
# idempotent: a warm machine re-runs them for the cost of a version check.
#
# Overrides, for an offline machine or a contributor working on the instrument
# itself (each is used verbatim and never fetched over):
#   VEREDICTUM_BIN   path to an existing `veredictum` binary
#   VEREDICTUM_ROOT  path to an existing Veredictum checkout (its `artifacts/`
#                    and `specs/` are read from there)
#   VEREDICTUM_CACHE cache root (default: $XDG_CACHE_HOME/ferroehr-veredictum,
#                    falling back to ~/.cache/ferroehr-veredictum)

# The pin. Bumping it is a deliberate change: the catalogue, the oracle and the
# verdict pipeline all move together, so a bump is re-proven by a full
# `scripts/conformance.sh` run against the committed baseline, unless the
# catalogue (`artifacts/`), `specs/`, `schemas/` and runner
# (`app/veredictum/src/`) trees are all unchanged between the old and the new
# tag. Machine-enforced by the `veredictum-pin` CI job
# (scripts/checks/veredictum-pin.sh, #2867, #3593): outside that exception the
# bumping PR refreshes docs/conformance/ferroehr/ or carries the
# `no-conformance-run` deferral label.
VEREDICTUM_VERSION="0.1.5"
VEREDICTUM_REPO="https://github.com/rubentalstra/Veredictum"

veredictum_cache_root() {
  local base="${VEREDICTUM_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/ferroehr-veredictum}"
  printf '%s/%s' "$base" "$VEREDICTUM_VERSION"
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
  local root bin
  root="$(veredictum_cache_root)/install"
  bin="$root/bin/veredictum"
  if [[ ! -x "$bin" ]] || ! "$bin" --version 2>/dev/null | grep -qF "$VEREDICTUM_VERSION"; then
    echo "==> Installing veredictum $VEREDICTUM_VERSION (cargo install, cached in $root)" >&2
    cargo install veredictum --version "$VEREDICTUM_VERSION" --locked --root "$root" >&2
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
  local root tag stamp
  root="$(veredictum_cache_root)/repo"
  tag="v$VEREDICTUM_VERSION"
  stamp="$root/.ferroehr-pin"
  if [[ "$(cat "$stamp" 2>/dev/null || true)" != "$tag" ]]; then
    echo "==> Fetching the Veredictum catalogue at $tag (shallow clone, cached in $root)" >&2
    rm -rf "${root:?}"
    mkdir -p "$(dirname "$root")"
    git -c advice.detachedHead=false clone --quiet --depth 1 --branch "$tag" \
      "$VEREDICTUM_REPO" "$root" >&2
    printf '%s\n' "$tag" > "$stamp"
  fi
  printf '%s' "$root"
}

# The catalogue root the `--root` flag takes.
veredictum_artifacts() { printf '%s/artifacts' "$(veredictum_root)"; }

# The vendored spec oracle the `--specs` flag takes. The instrument resolves
# the ITS schema bundles from this path's parent, so it is passed as-is.
veredictum_specs() { printf '%s/specs/openehr' "$(veredictum_root)"; }
