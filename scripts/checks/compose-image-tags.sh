#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Compose image-tag guard (the CITATION.cff / Helm appVersion pattern).
#
# On main, the quickstart docker-compose.yml and its overlays
# (docker-compose.terminology.yml mounts the server image as a volume, #3605)
# pull the images main builds, at `:main`, and carry the configuration main
# reads (#5003). This guard scans every compose file at the repository root
# and under docker/, and fails when any FerroEHR image reference
# (`ghcr.io/ferrohealth/ferroehr`, `-postgres`, `-viewer`) carries a
# release-shaped tag (X.Y.Z), and when docker-compose.yml's three
# `${FERROEHR_*_IMAGE:-…}` defaults are not `:main`. A `:local` or `:latest`
# tag names no release and is left alone. The release lane writes the version
# into the copy of docker-compose.yml it attaches to the release
# (scripts/release/quickstart-compose.sh), and `--release <file>…` holds that
# copy to the workspace version: every FerroEHR reference names it, and none is
# `:main`, so a release never ships a `:main` pin. The hosted sandbox's
# compose file (deploy/hosted/docker-compose.yml) deliberately tracks the
# `:latest` release pointer instead (#2974), so its defaults are held to that
# pointer. Every Keycloak `image:` in the same compose files names one
# `quay.io/keycloak/keycloak:X.Y.Z@sha256:<digest>`, so the dev lanes and the
# quickstart overlay run the same identity provider. The hosted database box
# (deploy/hosted/cloud-init-postgres.yaml) pins a published release by tag and
# index digest, which deploy/hosted/README.md names too. Dependency-free: grep
# + sed only.
#
# Usage: scripts/checks/compose-image-tags.sh [--self-test | --release <file>…]
#   --self-test mutates a COPY of the compose files eleven ways (a stale tag in
#   the terminology overlay, a pre-release suffix the workspace version
#   lacks, a stale tag on every postgres reference of docker-compose.yml
#   after the first, a removed quickstart default, a
#   versioned hosted default, a Keycloak tag moved in one file, a Keycloak
#   reference without its digest, a database box pin without its digest, on
#   `:latest` or past the workspace version, and a hosted README naming
#   another digest) and asserts the guard rejects each, then that the
#   unmutated copy passes.
set -Eeuo pipefail

if [[ "$#" -eq 1 && "$1" == "--self-test" ]]; then
  cd "$(dirname "$0")/../.."
  self_root="$(mktemp -d)"
  trap 'rm -rf "$self_root"' EXIT
  mkdir -p "$self_root/scripts" "$self_root/deploy/hosted" "$self_root/docker"
  cp -R scripts/checks scripts/lib "$self_root/scripts/"
  cp Cargo.toml docker-compose*.yml "$self_root/"
  cp docker/*.yml "$self_root/docker/"
  cp deploy/hosted/docker-compose.yml deploy/hosted/cloud-init-postgres.yaml deploy/hosted/README.md "$self_root/deploy/hosted/"
  self_fail=0
  # <label>|<file>|<sed program>
  mutations=(
    "a release tag on the server in the terminology overlay|${self_root}/docker-compose.terminology.yml|s|ferrohealth/ferroehr:main}|ferrohealth/ferroehr:0.0.1}|"
    "a server default other than main|${self_root}/docker-compose.yml|s|ferrohealth/ferroehr:main}|ferrohealth/ferroehr:main-rc9}|"
    "a release tag on a later postgres reference of docker-compose.yml|${self_root}/docker-compose.yml|/ferroehr-postgres:main/{x;s/^/x/;/^xx/{x;s|ferroehr-postgres:main}|ferroehr-postgres:0.0.1}|;x;};x;}"
    "docker-compose.yml without its viewer default|${self_root}/docker-compose.yml|s|\${FERROEHR_VIEWER_IMAGE:-[^}]*}|ghcr.io/ferrohealth/ferroehr-viewer:local|"
    "a versioned hosted default|${self_root}/deploy/hosted/docker-compose.yml|s|ferrohealth/ferroehr:latest|ferrohealth/ferroehr:1.2.3|"
    "a Keycloak tag the quickstart overlay alone moves|${self_root}/docker-compose.keycloak.yml|s|quay.io/keycloak/keycloak:[0-9][0-9.]*@|quay.io/keycloak/keycloak:1.0.0@|g"
    "a Keycloak reference without a digest|${self_root}/docker-compose.dev.yml|s|\(quay.io/keycloak/keycloak:[0-9][0-9.]*\)@sha256:[0-9a-f]*|\1|g"
    "a hosted database pin without its digest|${self_root}/deploy/hosted/cloud-init-postgres.yaml|s|\(ferroehr-postgres:[0-9.]*\)@sha256:[0-9a-f]*|\1|"
    "a hosted database pin on the latest tag|${self_root}/deploy/hosted/cloud-init-postgres.yaml|s|ferroehr-postgres:[0-9.]*@sha256:[0-9a-f]*|ferroehr-postgres:latest|"
    "a hosted database pin past the workspace version|${self_root}/deploy/hosted/cloud-init-postgres.yaml|s|ferroehr-postgres:[0-9.]*@|ferroehr-postgres:999.0.0@|"
    "a hosted README naming another database digest|${self_root}/deploy/hosted/README.md|s|\(ferroehr-postgres:[0-9.]*@sha256:\)[0-9a-f]\{8\}|\100000000|"
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
  # The release copy: the version written in place of every `:main` passes,
  # and one `:main` left behind, or another version, fails.
  version="$(sed -nE 's/^version = "(.*)"$/\1/p' Cargo.toml | sed -n 1p)"
  sed -E "s#(ghcr\.io/ferrohealth/ferroehr(-postgres|-viewer)?):main#\1:${version}#g" \
    docker-compose.yml > "$self_root/released.yml"
  if ! bash scripts/checks/compose-image-tags.sh --release "$self_root/released.yml" >/dev/null 2>&1; then
    echo "::error::--self-test: --release REJECTED the released copy." >&2
    self_fail=1
  else
    echo "  self-test: --release accepted the released copy"
  fi
  for label in "a main pin left in the released copy" "another version in the released copy"; do
    case "$label" in
      a\ main*) from="ferroehr-postgres:${version}" to="ferroehr-postgres:main" ;;
      *) from="ferroehr:${version}}" to="ferroehr:0.0.1}" ;;
    esac
    # The first occurrence alone, so the rest of the copy stays released.
    awk -v from="$from" -v to="$to" '
      !done && (i = index($0, from)) { $0 = substr($0, 1, i - 1) to substr($0, i + length(from)); done = 1 }
      { print }' "$self_root/released.yml" > "$self_root/mutated.yml"
    if cmp -s "$self_root/released.yml" "$self_root/mutated.yml"; then
      echo "::error::--self-test could not apply '${label}'." >&2
      self_fail=1
    elif bash scripts/checks/compose-image-tags.sh --release "$self_root/mutated.yml" >/dev/null 2>&1; then
      echo "::error::--self-test: --release ACCEPTED ${label}." >&2
      self_fail=1
    else
      echo "  self-test: --release rejected ${label}"
    fi
  done
  [[ "$self_fail" -eq 0 ]] || exit 1
  echo "compose-image-tags --self-test: every mutation rejected, the unmutated tree and the released copy accepted."
  exit 0
fi

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"

if [[ "${1:-}" == "--release" ]]; then
  shift
  [[ "$#" -ge 1 ]] || guard_usage "[--self-test | --release <file>…]"
  MANIFEST="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/Cargo.toml"
  version="$(sed -nE 's/^version = "(.*)"$/\1/p' "$MANIFEST" | sed -n 1p)"
  rc=0
  checked=0
  for file in "$@"; do
    while IFS=: read -r line ref; do
      [[ -n "$ref" ]] || continue
      tag="${ref##*:}"
      [[ "$tag" == local || "$tag" == latest ]] && continue
      checked=$((checked + 1))
      if [[ "$tag" != "$version" ]]; then
        echo "::error::$file:$line pins '$ref' in the release copy, but the release is $version: every FerroEHR image names it, and none is :main." >&2
        rc=1
      fi
    done < <(grep -noE 'ghcr\.io/ferrohealth/ferroehr(-postgres|-viewer)?:[0-9A-Za-z.-]+' "$file" || true)
  done
  if [[ "$checked" -eq 0 ]]; then
    echo "::error::the release copy pins no FerroEHR image; the check has nothing to check." >&2
    rc=1
  fi
  [[ "$rc" -eq 0 ]] && echo "the release copy pins every FerroEHR image at $version."
  exit "$rc"
fi
[[ "$#" -eq 0 ]] || guard_usage "[--self-test | --release <file>…]"

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

# The quickstart keeps one overridable default per first-party image, each
# the `:main` image main builds.
for var in FERROEHR_IMAGE FERROEHR_POSTGRES_IMAGE FERROEHR_VIEWER_IMAGE; do
  if ! grep -qE "\\\$\{$var:-ghcr\.io/ferrohealth/[^}]+\}" "$COMPOSE_FILE"; then
    echo "::error::$COMPOSE_FILE declares no \${$var:-…} default image reference" >&2
    rc=1
  fi
  while IFS= read -r ref; do
    if [[ "${ref##*:}" != main ]]; then
      echo "::error::$COMPOSE_FILE defaults $var to '$ref'; on main the quickstart pulls the :main image, and the release lane writes the version into the copy it attaches." >&2
      rc=1
    fi
  done < <(grep -oE "\\\$\{$var:-ghcr\.io/ferrohealth/[^}]+\}" "$COMPOSE_FILE" | sed -E "s/^\\\$\{$var:-//; s/\}$//")
done

# No root or docker/ compose file pins a FerroEHR release: on main every
# FerroEHR image is `:main` (or a `:local` build), and only the release copy
# names a version.
checked=0
shopt -s nullglob
for file in "$ROOT_DIR"/docker-compose*.yml "$ROOT_DIR"/docker/*.yml; do
  while IFS=: read -r line ref; do
    [[ -n "$ref" ]] || continue
    tag="${ref##*:}"
    rel="${file#"$ROOT_DIR"/}"
    case "$tag" in
      local | latest) continue ;;
      main) checked=$((checked + 1)); echo "$rel:$line $ref is the :main image." ;;
      *)
        echo "::error::$rel:$line pins '$ref'; on main a FerroEHR image is :main, and the release lane writes the version into the copy it attaches (scripts/release/quickstart-compose.sh)." >&2
        rc=1
        ;;
    esac
  done < <(grep -noE 'ghcr\.io/ferrohealth/ferroehr(-postgres|-viewer)?:[0-9A-Za-z.-]+' "$file" || true)
done
shopt -u nullglob
if [[ "$checked" -eq 0 ]]; then
  echo "::error::no compose file pins a FerroEHR :main image; the guard has nothing to check, so its scan is broken." >&2
  rc=1
fi

# Every Keycloak reference names one release, pinned by digest: the dev lanes
# and the quickstart overlay run the same identity provider, so a bump that
# misses a file splits them, and the base-image watcher reads the pin from
# docker-compose.dev.yml alone. No specification governs this: our own design.
keycloak_refs=()
shopt -s nullglob
for file in "$ROOT_DIR"/docker-compose*.yml "$ROOT_DIR"/docker/*.yml; do
  while IFS=: read -r line ref; do
    [[ -n "$ref" ]] || continue
    rel="${file#"$ROOT_DIR"/}"
    if [[ ! "$ref" =~ ^quay\.io/keycloak/keycloak:[0-9]+\.[0-9]+\.[0-9]+@sha256:[0-9a-f]{64}$ ]]; then
      echo "::error::$rel:$line names '$ref'; a Keycloak image is pinned as quay.io/keycloak/keycloak:X.Y.Z@sha256:<digest>." >&2
      rc=1
    fi
    keycloak_refs+=("$ref")
  done < <(grep -nE '^[[:space:]]*image:[[:space:]]*"?quay\.io/keycloak/keycloak' "$file" \
    | sed -E 's/^([0-9]+):[[:space:]]*image:[[:space:]]*"?([^[:space:]"]+).*/\1:\2/' || true)
done
shopt -u nullglob
if [[ "${#keycloak_refs[@]}" -eq 0 ]]; then
  echo "::error::no compose file names a Keycloak image; the guard has nothing to check, so its scan is broken." >&2
  rc=1
elif [[ "$(printf '%s\n' "${keycloak_refs[@]}" | sort -u | wc -l | tr -d ' ')" -ne 1 ]]; then
  echo "::error::the compose files name more than one Keycloak image; every reference carries the same tag and digest:" >&2
  printf '%s\n' "${keycloak_refs[@]}" | sort | uniq -c >&2
  rc=1
else
  echo "every Keycloak reference (${#keycloak_refs[@]}) is ${keycloak_refs[0]}."
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

# The hosted database box runs a published release pinned by its index digest
# (deploy/hosted/cloud-init-postgres.yaml, #4252). The pin moves after a
# release publishes, since its digest exists only then, so its tag is at or
# before the workspace version, and the hosted README names the same reference.
DB_CLOUD_INIT="$ROOT_DIR/deploy/hosted/cloud-init-postgres.yaml"
HOSTED_README="$ROOT_DIR/deploy/hosted/README.md"
db_pin="$(grep -oE '\$\{FERROEHR_POSTGRES_IMAGE:-[^}]+\}' "$DB_CLOUD_INIT" | sed -E 's/^\$\{FERROEHR_POSTGRES_IMAGE:-//; s/\}$//' | sed -n 1p)" || true
db_pin_re='^ghcr\.io/ferrohealth/ferroehr-postgres:([0-9]+\.[0-9]+\.[0-9]+)@sha256:[0-9a-f]{64}$'
if [[ ! "$db_pin" =~ $db_pin_re ]]; then
  echo "::error::$DB_CLOUD_INIT must default FERROEHR_POSTGRES_IMAGE to ghcr.io/ferrohealth/ferroehr-postgres:X.Y.Z@sha256:<index digest>; found '${db_pin}'." >&2
  rc=1
elif [[ "$(printf '%s\n%s\n' "${BASH_REMATCH[1]}" "$version" | sort -t. -k1,1n -k2,2n -k3,3n | sed -n 2p)" != "$version" ]]; then
  echo "::error::$DB_CLOUD_INIT pins ferroehr-postgres ${BASH_REMATCH[1]}, past the workspace version $version: the pin names a published release." >&2
  rc=1
elif ! grep -qF "$db_pin" "$HOSTED_README"; then
  echo "::error::$HOSTED_README does not name the database box's pin $db_pin from $DB_CLOUD_INIT." >&2
  rc=1
else
  echo "hosted database box pins $db_pin, and the hosted README names it."
fi

exit "$rc"
