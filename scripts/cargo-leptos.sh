#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# `cargo leptos` for the viewer, with the workspace lockfile frozen.
#
# Every other build in this repository runs `--locked`; the viewer's did not,
# and a viewer build re-resolved and rewrote Cargo.lock in the working tree
# (#2877). `cargo leptos` cannot simply be handed the flag: before it compiles
# anything it resolves the workspace through its own `cargo metadata` call
# (cargo_metadata's MetadataCommand, which cargo-leptos passes no extra flags
# to), and cargo exposes no environment variable for `--locked` — the
# environment-variables reference lists one only for `--offline`
# (CARGO_NET_OFFLINE), and an offline cargo still re-resolves from the local
# registry cache. Measured on the pinned toolchain: `cargo metadata` against a
# lockfile with one entry removed silently re-resolved it to a NEWER version.
#
# So the lock is frozen in two moves:
#   1. a `cargo metadata --locked` PRECHECK — it fails loud if the committed
#      lockfile does not already satisfy every manifest, and it runs before
#      cargo-leptos exists, so cargo-leptos's own unlocked metadata call then
#      finds nothing to change;
#   2. `--locked` passed through to both compile legs (`--lib-cargo-args` /
#      `--bin-cargo-args`), so neither can re-resolve either.
#
# Usage: scripts/cargo-leptos.sh build [--release] [...]
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VIEWER_DIR="$ROOT/app/ferroehr-viewer"

if ! cargo metadata --locked --format-version 1 \
  --manifest-path "$VIEWER_DIR/Cargo.toml" >/dev/null; then
  echo "FATAL: Cargo.lock does not satisfy the workspace manifests." >&2
  echo "       Re-resolve deliberately (cargo check --workspace) and commit the" >&2
  echo "       lockfile change; a viewer build must never do it silently." >&2
  exit 1
fi

# `cargo leptos` reads its configuration from the viewer crate's own manifest
# directory, so the build runs from there.
cd "$VIEWER_DIR"
exec cargo leptos "$@" \
  --lib-cargo-args=--locked \
  --bin-cargo-args=--locked
