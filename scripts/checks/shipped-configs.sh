#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Shipped-configuration guard: every server configuration this repository
# ships is read by the server built from the change under test (#5396).
#
# The 5.7.0-rc3 sandbox did not start because deploy/hosted/ferroehr.sandbox.toml
# still spelled `[scim]`, which 5.7.0 moved under `[auth]`, and nothing read
# that file before the box did. The quickstart's inline blocks are checked by
# quickstart-config-image.sh and every chart overlay by
# deploy/helm/ci/boot-check.sh; this guard reads the rest with `config check`,
# with the environment each deployment gives the server:
#
#   * deploy/hosted/ferroehr.sandbox.toml, with the DATABASE_URL the box's
#     `.env` supplies (deploy/hosted/docker-compose.yml);
#   * the server configuration and the `ferroehr` environment of every
#     overlay on docker-compose.yml (docker-compose.<overlay>.yml), resolved
#     with `docker compose config` over both files. A file the server reads
#     from a named volume (the Keycloak overlay's development CA) is staged as
#     a throwaway CA certificate, since the volume exists only on a running
#     stack.
#
# No specification governs the deployment files: our own design.
#
# Usage: scripts/checks/shipped-configs.sh   (no arguments)
#   FERROEHR_CONFIG_IMAGE  the FerroEHR image built from this tree, run with
#                          no network; or
#   FERROEHR_BIN           a FerroEHR binary built from this tree, run directly
#                          (a developer machine).
# Needs docker with the compose plugin (or `docker-compose`), and jq.
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"
cd "$(dirname "$0")/../.." || exit 1

if [[ -z "${FERROEHR_CONFIG_IMAGE:-}" && -z "${FERROEHR_BIN:-}" ]]; then
  echo "::error::shipped-configs: set FERROEHR_CONFIG_IMAGE or FERROEHR_BIN to the server built from this tree" >&2
  exit 2
fi
if docker compose version >/dev/null 2>&1; then
  compose=(docker compose)
else
  compose=(docker-compose)
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
readonly target=/etc/ferroehr/ferroehr.toml
readonly dsn="postgres://ferroehr:config-check@ferroehr-postgres:5432/ferroehr"
fail=0

# A throwaway CA certificate for a trust-roots file a volume would hold.
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 1 \
  -subj "/CN=shipped-configs" -keyout "$work/ca.key" -out "$work/ca.crt" >/dev/null 2>&1

# The docker run mounts of the named volumes the service reads, and the
# environment a local binary reads them through instead.
mounts=()

# check <label> <toml> <env-file>: `config check` over the file, with the
# environment in the env file and the usage report off.
check() {
  local label="$1" toml="$2" envfile="$3" status=0
  echo "FERROEHR__USAGE_REPORT__ENABLED=false" >>"$envfile"
  if [[ -n "${FERROEHR_CONFIG_IMAGE:-}" ]]; then
    chmod 0644 "$toml" "$envfile"
    docker run --rm --network none --env-file "$envfile" ${mounts[@]+"${mounts[@]}"} -v "$(cd "$(dirname "$toml")" && pwd)/$(basename "$toml"):$target:ro" \
      "$FERROEHR_CONFIG_IMAGE" config check --config "$target" </dev/null >"$work/out" 2>&1 || status=$?
  else
    local -a vars=()
    while IFS= read -r line; do vars+=("$line"); done <"$envfile"
    env -i PATH="$PATH" ${vars[@]+"${vars[@]}"} "$FERROEHR_BIN" config check --config "$toml" </dev/null >"$work/out" 2>&1 || status=$?
  fi
  if [[ "$status" -eq 0 ]]; then
    echo "  ok: $label reads under the server built from this tree"
  else
    echo "::error::the server built from this tree refuses $label:" >&2
    cat "$work/out" >&2
    fail=1
  fi
}

# ── The hosted sandbox ───────────────────────────────────────────────────────
printf 'DATABASE_URL=%s\n' "$dsn" >"$work/hosted.env"
check deploy/hosted/ferroehr.sandbox.toml deploy/hosted/ferroehr.sandbox.toml "$work/hosted.env"

# ── Every overlay on the quickstart ──────────────────────────────────────────
for overlay in docker-compose.*.yml; do
  "${compose[@]}" -f docker-compose.yml -f "$overlay" config --format json </dev/null >"$work/compose.json" 2>"$work/compose.err" || {
    echo "::error file=$overlay::docker compose cannot resolve docker-compose.yml with $overlay:" >&2
    cat "$work/compose.err" >&2
    fail=1
    continue
  }
  jq -e '.services.ferroehr' "$work/compose.json" >/dev/null || continue
  source="$(jq -r --arg t "$target" '.services.ferroehr.configs[]? | select(.target == $t) | .source' "$work/compose.json")"
  [[ -n "$source" ]] || continue
  name="${overlay#docker-compose.}"
  name="${name%.yml}"
  file="$(jq -r --arg s "$source" '.configs[$s].file // empty' "$work/compose.json")"
  if [[ -n "$file" ]]; then
    cp "$file" "$work/$name.toml"
  else
    jq -r --arg s "$source" '.configs[$s].content | gsub("\\$\\$"; "$")' "$work/compose.json" >"$work/$name.toml"
  fi
  # Each value under a named volume the service mounts names a file there:
  # stage the CA certificate at it.
  mounts=()
  local_paths=()
  while IFS=$'\t' read -r volume vtarget; do
    mkdir -p "$work/vol/$volume"
    while IFS= read -r path; do
      mkdir -p "$(dirname "$work/vol/$volume/${path#"$vtarget"/}")"
      cp "$work/ca.crt" "$work/vol/$volume/${path#"$vtarget"/}"
    done < <(jq -r --arg t "$vtarget/" '.services.ferroehr.environment // {} | to_entries[] | .value // "" | tostring | select(startswith($t))' "$work/compose.json")
    chmod -R a+rX "$work/vol"
    mounts+=(-v "$work/vol/$volume:$vtarget:ro")
    local_paths+=("$vtarget|$work/vol/$volume")
  done < <(jq -r '.services.ferroehr.volumes[]? | select(.type == "volume") | "\(.source)\t\(.target)"' "$work/compose.json")
  jq -r '.services.ferroehr.environment // {} | to_entries[]
    | select(.value != null) | "\(.key)=\(.value)"' "$work/compose.json" \
    | grep -v -e '^FERROEHR__USAGE_REPORT__ENABLED=' -e '^FERROEHR_CONFIG=' >"$work/$name.env" || true
  # A local binary reads the staged files where they are.
  if [[ -z "${FERROEHR_CONFIG_IMAGE:-}" ]]; then
    for pair in ${local_paths[@]+"${local_paths[@]}"}; do
      sed -i.bak "s|=${pair%%|*}/|=${pair#*|}/|" "$work/$name.env"
    done
  fi
  check "docker-compose.yml with $overlay" "$work/$name.toml" "$work/$name.env"
done

exit "$fail"
