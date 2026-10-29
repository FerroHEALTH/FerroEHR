#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Quickstart config guard: the inline configurations of docker-compose.yml are
# readable by the images the file pins (#4380).
#
# The quickstart pins the last published images while its inline
# configurations can already use keys only the next server reads; a downloader
# then gets a server that refuses its own configuration and never turns
# healthy. This guard resolves the file with `docker compose config`, writes
# every `ferroehr-config*` block and the `ferropix-config` block to a file, and
# runs the `config check` of the image each service pins over it, with the
# service's own environment, on no network. A refusal fails the guard.
#
# A FerroEHR tag the registry does not hold yet is the release in flight (the
# release cut moves the pin to the version it is about to publish); that one
# is skipped with a notice, since the release lane publishes it from this tree.
#
# Usage: scripts/checks/quickstart-config-image.sh   (no arguments)
# Needs docker with the compose plugin, and jq.
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"
cd "$(dirname "$0")/../.." || exit 1

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

docker compose -f docker-compose.yml config --format json >"$work/compose.json"
version="$(awk -F'"' '/^version = /{print $2; exit}' Cargo.toml)"
fail=0

# Pull an image, or report that it is not published; exit status 0 = pulled.
pulled() {
  docker pull --quiet "$1" >/dev/null 2>&1
}

# Every name=value of a service's environment that carries a value, one per
# line, the usage report switched off whatever the file says.
service_env() {
  jq -r --arg s "$1" '.services[$s].environment // {}
    | to_entries[] | select(.value != null) | "\(.key)=\(.value)"' "$work/compose.json" \
    | grep -v -e '^FERROEHR__USAGE_REPORT__ENABLED=' -e '^FERROPIX__USAGE_REPORT__ENABLED=' || true
}

# ── FerroEHR ─────────────────────────────────────────────────────────────────
ehr_image="$(jq -r '.services.ferroehr.image' "$work/compose.json")"
if pulled "$ehr_image"; then
  service_env ferroehr >"$work/ferroehr.env"
  echo "FERROEHR__USAGE_REPORT__ENABLED=false" >>"$work/ferroehr.env"
  target="$(jq -r '.services.ferroehr.configs[0].target' "$work/compose.json")"
  # Every server configuration block, the one the service selects by default
  # and the ones a variable selects (FERROEHR_EHR_INDEX=local).
  while IFS= read -r block; do
    suffix="${block#ferroehr-config}"
    FERROEHR_EHR_INDEX="${suffix#-}" \
      docker compose -f docker-compose.yml config --format json </dev/null \
      | jq -r --arg b "$block" '.configs[$b].content | gsub("\\$\\$"; "$")' >"$work/$block.toml"
    if docker run --rm --network none --env-file "$work/ferroehr.env" \
      -v "$work/$block.toml:$target:ro" "$ehr_image" config check </dev/null >"$work/$block.out" 2>&1; then
      echo "  ok: $block reads under $ehr_image"
    else
      echo "::error file=docker-compose.yml::$ehr_image refuses the inline configuration $block:" >&2
      cat "$work/$block.out" >&2
      fail=1
    fi
  done < <(grep -o "^  ferroehr-config[a-z-]*:" docker-compose.yml | tr -d " :")
elif [[ "$ehr_image" == *":$version" ]]; then
  echo "::notice::$ehr_image is not published yet: it is the release in flight, which the release lane publishes from this tree."
else
  echo "::error file=docker-compose.yml::$ehr_image cannot be pulled." >&2
  fail=1
fi

# ── FerroPIX ─────────────────────────────────────────────────────────────────
pix_image="$(jq -r '.services.ferropix.image' "$work/compose.json")"
if pulled "$pix_image"; then
  service_env ferropix >"$work/ferropix.env"
  echo "FERROPIX__USAGE_REPORT__ENABLED=false" >>"$work/ferropix.env"
  jq -r '.configs["ferropix-config"].content | gsub("\\$\\$"; "$")' "$work/compose.json" >"$work/ferropix.toml"
  jq -r '.configs["ferropix-db-url"].content | gsub("\\$\\$"; "$")' "$work/compose.json" >"$work/db-url"
  # A throwaway sealing key: the file only has to exist and hold 32 bytes.
  head -c 32 /dev/urandom | base64 >"$work/data-key"
  chmod 0644 "$work"/*
  args=(--rm --network none --env-file "$work/ferropix.env")
  while IFS=$'\t' read -r source target; do
    case "$source" in
      ferropix-config) args+=(-v "$work/ferropix.toml:$target:ro") ;;
      ferropix-db-url) args+=(-v "$work/db-url:$target:ro") ;;
    esac
  done < <(jq -r '.services.ferropix.configs[] | "\(.source)\t\(.target)"' "$work/compose.json")
  key_dir="$(jq -r '.services.ferropix.volumes[] | select(.source == "ferropix-key") | .target' "$work/compose.json")"
  args+=(-v "$work/data-key:$key_dir/data-key:ro")
  if docker run "${args[@]}" "$pix_image" config check >"$work/ferropix.out" 2>&1; then
    echo "  ok: ferropix-config reads under $pix_image"
  else
    echo "::error file=docker-compose.yml::$pix_image refuses the inline configuration ferropix-config:" >&2
    cat "$work/ferropix.out" >&2
    fail=1
  fi
else
  echo "::error file=docker-compose.yml::$pix_image cannot be pulled." >&2
  fail=1
fi

exit "$fail"
