#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
# Compose image-tag guard (the CITATION.cff / Helm appVersion pattern).
#
# The standalone quickstart docker-compose.yml pulls the PUBLISHED images by an
# explicit tag, so a release cut that forgets to bump those defaults ships a
# quickstart pinned to the previous release. This guard fails when any default
# tag in the `${FERROEHR_*_IMAGE:-ghcr.io/...:X.Y.Z}` fallbacks disagrees with
# the workspace version in Cargo.toml (.claude/rules/changelog.md §Cutting a
# release). The hosted sandbox's compose file (deploy/hosted/docker-compose.yml)
# deliberately tracks the `:latest` release pointer instead (#2974), so its
# defaults are held to that pointer. Dependency-free: grep + sed only.
#
# Usage: scripts/checks/compose-image-tags.sh   (no arguments)
set -Eeuo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
COMPOSE_FILE="$ROOT_DIR/docker-compose.yml"
HOSTED_COMPOSE="$ROOT_DIR/deploy/hosted/docker-compose.yml"
MANIFEST="$ROOT_DIR/Cargo.toml"

version="$(sed -nE 's/^version = "(.*)"$/\1/p' "$MANIFEST" | head -1)"
if [[ -z "$version" ]]; then
  echo "::error::could not read the workspace version from $MANIFEST" >&2
  exit 1
fi

rc=0
for var in FERROEHR_IMAGE FERROEHR_POSTGRES_IMAGE FERROEHR_VIEWER_IMAGE; do
  # The default is the `:-` fallback inside `${VAR:-ghcr.io/owner/name:tag}`.
  ref="$(grep -oE "\\\$\{$var:-[^}]+\}" "$COMPOSE_FILE" | head -1 \
    | sed -E "s/^\\\$\{$var:-//; s/\}$//")"
  if [[ -z "$ref" ]]; then
    echo "::error::$COMPOSE_FILE declares no \${$var:-…} default image reference" >&2
    rc=1
    continue
  fi
  tag="${ref##*:}"
  if [[ "$tag" != "$version" ]]; then
    echo "::error::$var default is '$ref' (tag $tag) but the workspace version is $version — the release cut bumps the docker-compose.yml image tags (.claude/rules/changelog.md)." >&2
    rc=1
  else
    echo "$var default $ref matches the workspace version."
  fi
done

# The hosted sandbox's compose defaults deliberately carry NO version: the box
# tracks the `:latest` release pointer the image lane moves (#2974), so this
# guard only pins them to that pointer — a versioned default reappearing would
# resurrect the release-cut bump this guard used to police.
for pair in "FERROEHR_IMAGE ghcr.io/ferrohealth/ferroehr:latest"   "FERROEHR_VIEWER_IMAGE ghcr.io/ferrohealth/ferroehr-viewer:latest"; do
  var="${pair%% *}"
  want="${pair#* }"
  ref="$(grep -oE "\\$\{$var:-[^}]+\}" "$HOSTED_COMPOSE" | head -1     | sed -E "s/^\\$\{$var:-//; s/\}$//")"
  if [[ "$ref" != "$want" ]]; then
    echo "::error::$HOSTED_COMPOSE must default $var to $want (the release pointer, #2974); found '${ref:-nothing}'." >&2
    rc=1
  else
    echo "hosted sandbox $var tracks the :latest release pointer."
  fi
done

exit "$rc"
