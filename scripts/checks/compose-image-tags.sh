#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Compose image-tag guard (the CITATION.cff / Helm appVersion pattern).
#
# The quickstart docker-compose.yml and its overlays (docker-compose.terminology.yml
# mounts the server image as a volume, #3605) pull the PUBLISHED images by an
# explicit tag, so a release cut that forgets one of those defaults ships a
# compose stack pinned to the previous release. This guard scans every compose
# file at the repository root and under docker/, and fails when any FerroEHR
# image reference (`ghcr.io/ferrohealth/ferroehr`, `-postgres`, `-viewer`) with
# a release-shaped tag (X.Y.Z) disagrees with the workspace version in
# Cargo.toml (.claude/rules/changelog.md §Cutting a release). A `:local` or
# `:latest` tag names no release and is left alone. docker-compose.yml must
# also keep its three `${FERROEHR_*_IMAGE:-…}` defaults. The hosted sandbox's
# compose file (deploy/hosted/docker-compose.yml) deliberately tracks the
# `:latest` release pointer instead (#2974), so its defaults are held to that
# pointer. Dependency-free: grep + sed only.
#
# Usage: scripts/checks/compose-image-tags.sh [--self-test]
#   --self-test mutates a COPY of the compose files four ways (a stale tag in
#   the terminology overlay, a stale tag on every postgres reference of
#   docker-compose.yml after the first, a removed quickstart default, a
#   versioned hosted default) and asserts the guard rejects each, then that
#   the unmutated copy passes.
set -Eeuo pipefail

if [[ "$#" -eq 1 && "$1" == "--self-test" ]]; then
  cd "$(dirname "$0")/../.."
  self_root="$(mktemp -d)"
  trap 'rm -rf "$self_root"' EXIT
  mkdir -p "$self_root/scripts" "$self_root/deploy/hosted" "$self_root/docker"
  cp -R scripts/checks scripts/lib "$self_root/scripts/"
  cp Cargo.toml docker-compose*.yml "$self_root/"
  cp docker/*.yml "$self_root/docker/"
  cp deploy/hosted/docker-compose.yml "$self_root/deploy/hosted/"
  self_fail=0
  # <label>|<file>|<sed program>
  mutations=(
    "a stale server tag in the terminology overlay|${self_root}/docker-compose.terminology.yml|s|ferrohealth/ferroehr:[0-9][0-9.]*}|ferrohealth/ferroehr:0.0.1}|"
    "a stale tag on a later postgres reference of docker-compose.yml|${self_root}/docker-compose.yml|/ferroehr-postgres:[0-9]/{x;s/^/x/;/^xx/{x;s|ferroehr-postgres:[0-9][0-9.]*}|ferroehr-postgres:0.0.1}|;x;};x;}"
    "docker-compose.yml without its viewer default|${self_root}/docker-compose.yml|s|\${FERROEHR_VIEWER_IMAGE:-[^}]*}|ghcr.io/ferrohealth/ferroehr-viewer:local|"
    "a versioned hosted default|${self_root}/deploy/hosted/docker-compose.yml|s|ferrohealth/ferroehr:latest|ferrohealth/ferroehr:1.2.3|"
  )
  for mutation in "${mutations[@]}"; do
    label="${mutation%%|*}"
    rest="${mutation#*|}"
    file="${rest%%|*}"
    program="${rest#*|}"
    cp "$file" "${file}.orig"
    sed "$program" "${file}.orig" > "$file"
    if cmp -s "${file}.orig" "$file"; then
      echo "::error::--self-test could not apply the mutation '${label}': its anchor no longer matches, so this case proves nothing." >&2
      self_fail=1
    elif (cd "$self_root" && bash scripts/checks/compose-image-tags.sh >/dev/null 2>&1); then
      echo "::error::--self-test: the guard ACCEPTED ${label}." >&2
      self_fail=1
    else
      echo "  self-test: rejected ${label}"
    fi
    mv "${file}.orig" "$file"
  done
  # The unmutated copy must pass, or every rejection above is a broken fixture.
  if ! (cd "$self_root" && bash scripts/checks/compose-image-tags.sh >/dev/null 2>&1); then
    echo "::error::--self-test: the guard REJECTED the unmutated tree." >&2
    self_fail=1
  fi
  [[ "$self_fail" -eq 0 ]] || exit 1
  echo "compose-image-tags --self-test: every mutation rejected, the unmutated tree accepted."
  exit 0
fi

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
[[ "$#" -eq 0 ]] || guard_usage "[--self-test]"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
COMPOSE_FILE="$ROOT_DIR/docker-compose.yml"
HOSTED_COMPOSE="$ROOT_DIR/deploy/hosted/docker-compose.yml"
MANIFEST="$ROOT_DIR/Cargo.toml"

version="$(sed -nE 's/^version = "(.*)"$/\1/p' "$MANIFEST" | sed -n 1p)"
if [[ -z "$version" ]]; then
  echo "::error::could not read the workspace version from $MANIFEST" >&2
  exit 1
fi

rc=0

# The quickstart keeps one overridable default per first-party image.
for var in FERROEHR_IMAGE FERROEHR_POSTGRES_IMAGE FERROEHR_VIEWER_IMAGE; do
  if ! grep -qE "\\\$\{$var:-ghcr\.io/ferrohealth/[^}]+\}" "$COMPOSE_FILE"; then
    echo "::error::$COMPOSE_FILE declares no \${$var:-…} default image reference" >&2
    rc=1
  fi
done

# Every release-shaped FerroEHR image tag in every root and docker/ compose
# file, each occurrence on its own, equals the workspace version.
checked=0
shopt -s nullglob
for file in "$ROOT_DIR"/docker-compose*.yml "$ROOT_DIR"/docker/*.yml; do
  while IFS=: read -r line ref; do
    [[ -n "$ref" ]] || continue
    checked=$((checked + 1))
    tag="${ref##*:}"
    rel="${file#"$ROOT_DIR"/}"
    if [[ "$tag" != "$version" ]]; then
      echo "::error::$rel:$line pins '$ref' (tag $tag) but the workspace version is $version; the release cut bumps every compose file's FerroEHR image tags (.claude/rules/changelog.md)." >&2
      rc=1
    else
      echo "$rel:$line $ref matches the workspace version."
    fi
  done < <(grep -noE 'ghcr\.io/ferrohealth/ferroehr(-postgres|-viewer)?:[0-9]+\.[0-9]+\.[0-9]+' "$file" || true)
done
shopt -u nullglob
if [[ "$checked" -eq 0 ]]; then
  echo "::error::no compose file pins a FerroEHR release tag; the guard has nothing to check, so its scan is broken." >&2
  rc=1
fi

# The hosted sandbox's compose defaults deliberately carry NO version: the box
# tracks the `:latest` release pointer the image lane moves (#2974), so this
# guard only pins them to that pointer. A versioned default reappearing would
# resurrect the release-cut bump this guard used to police.
for pair in "FERROEHR_IMAGE ghcr.io/ferrohealth/ferroehr:latest" "FERROEHR_VIEWER_IMAGE ghcr.io/ferrohealth/ferroehr-viewer:latest"; do
  var="${pair%% *}"
  want="${pair#* }"
  while IFS= read -r ref; do
    if [[ "$ref" != "$want" ]]; then
      echo "::error::$HOSTED_COMPOSE must default $var to $want (the release pointer, #2974); found '$ref'." >&2
      rc=1
    fi
  done < <(grep -oE "\\\$\{$var:-[^}]+\}" "$HOSTED_COMPOSE" | sed -E "s/^\\\$\{$var:-//; s/\}$//")
  if ! grep -qE "\\\$\{$var:-[^}]+\}" "$HOSTED_COMPOSE"; then
    echo "::error::$HOSTED_COMPOSE must default $var to $want (the release pointer, #2974); found nothing." >&2
    rc=1
  else
    echo "hosted sandbox $var tracks the :latest release pointer."
  fi
done

exit "$rc"
