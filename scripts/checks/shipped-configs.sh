#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Shipped-configuration guard: every server and viewer configuration this
# repository ships is read by the program built from the change under test
# (#5396).
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
# The viewer reads its configuration through its own loader, so a viewer
# binary checks, with `ferroehr-viewer config check`:
#
#   * the hosted sandbox's viewer file with the environment
#     deploy/hosted/docker-compose.yml gives the viewer;
#   * the viewer environment (and file, where one is mounted) of
#     docker-compose.yml alone and with each overlay;
#   * the viewer environment of every chart render under deploy/helm/golden/
#     that deploys the viewer.
#
# No specification governs the deployment files: our own design.
#
# Usage: scripts/checks/shipped-configs.sh   (no arguments)
#   FERROEHR_CONFIG_IMAGE  the FerroEHR image built from this tree, run with
#                          no network; or
#   FERROEHR_BIN           a FerroEHR binary built from this tree, run directly
#                          (a developer machine);
#   FERROEHR_VIEWER_BIN    a ferroehr-viewer binary built from this tree.
# At least one is set; each part runs for the program it is given.
# Needs docker with the compose plugin (or `docker-compose`), jq and yq.
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"
cd "$(dirname "$0")/../.." || exit 1

if [[ -z "${FERROEHR_CONFIG_IMAGE:-}" && -z "${FERROEHR_BIN:-}" && -z "${FERROEHR_VIEWER_BIN:-}" ]]; then
  echo "::error::shipped-configs: set FERROEHR_CONFIG_IMAGE, FERROEHR_BIN or FERROEHR_VIEWER_BIN to a program built from this tree" >&2
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

if [[ -n "${FERROEHR_CONFIG_IMAGE:-}" || -n "${FERROEHR_BIN:-}" ]]; then
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

fi

# viewer_check <label> <env-file> [<file>]: `ferroehr-viewer config check`
# with the environment in the env file and, when given, the file the viewer
# reads in place of the path the deployment mounts it at.
viewer_check() {
  local label="$1" envfile="$2" file="${3:-}" status=0
  local -a vars=()
  while IFS= read -r line; do
    case "$line" in
      FERROEHR_VIEWER_CONFIG=*) ;;
      *) vars+=("$line") ;;
    esac
  done <"$envfile"
  [[ -z "$file" ]] || vars+=("FERROEHR_VIEWER_CONFIG=$file")
  env -i PATH="$PATH" "${vars[@]}" "$FERROEHR_VIEWER_BIN" config check </dev/null >"$work/out" 2>&1 || status=$?
  if [[ "$status" -eq 0 ]]; then
    echo "  ok: $label reads under the viewer built from this tree"
  else
    echo "::error::the viewer built from this tree refuses $label:" >&2
    cat "$work/out" >&2
    fail=1
  fi
}

# stage <env-file> <mount-target>...: each value in the env file under a
# mount target names a file a volume holds on a running deployment; a
# throwaway CA certificate is staged for it and the value pointed there.
stage() {
  local envfile="$1" mount line key value
  shift
  : >"$envfile.staged"
  while IFS= read -r line; do
    key="${line%%=*}"
    value="${line#*=}"
    for mount in "$@"; do
      if [[ "$value" == "$mount"/* ]]; then
        mkdir -p "$(dirname "$work/stage$value")"
        cp "$work/ca.crt" "$work/stage$value"
        value="$work/stage$value"
      fi
    done
    printf '%s=%s\n' "$key" "$value" >>"$envfile.staged"
  done <"$envfile"
  mv "$envfile.staged" "$envfile"
}

# viewer_compose <label> <compose.json> <dir>: the viewer service of a
# resolved compose file, its environment and the file it mounts at the path
# FERROEHR_VIEWER_CONFIG names, a relative bind source read from <dir>.
viewer_compose() {
  local label="$1" json="$2" dir="$3" config source file="" service
  service="$(jq -r '[.services | to_entries[] | select(.value.image // "" | test("/ferroehr-viewer[:@]")) | .key][0] // empty' "$json")"
  [[ -n "$service" ]] || return 0
  jq -r --arg s "$service" '.services[$s].environment // {} | to_entries[]
    | select(.value != null) | "\(.key)=\(.value)"' "$json" >"$work/viewer.env"
  config="$(jq -r --arg s "$service" '.services[$s].environment.FERROEHR_VIEWER_CONFIG // empty' "$json")"
  if [[ -n "$config" ]]; then
    source="$(jq -r --arg s "$service" --arg t "$config" '.services[$s].volumes[]? | select(.target == $t) | .source' "$json")"
    if [[ -n "$source" ]]; then
      # A bind mount: the file itself.
      case "$source" in
        /*) file="$source" ;;
        *) file="$dir/$source" ;;
      esac
    else
      # A compose config: its file, or its inline content.
      source="$(jq -r --arg s "$service" --arg t "$config" '.services[$s].configs[]? | select(.target == $t) | .source' "$json")"
      if [[ -n "$source" ]]; then
        file="$(jq -r --arg c "$source" '.configs[$c].file // empty' "$json")"
        if [[ -z "$file" ]]; then
          file="$work/viewer-config.toml"
          jq -r --arg c "$source" '.configs[$c].content | gsub("\\$\\$"; "$")' "$json" >"$file"
        fi
      fi
    fi
    if [[ -z "$file" ]]; then
      echo "::error::$label names FERROEHR_VIEWER_CONFIG=$config, which no volume or config of the viewer provides" >&2
      fail=1
      return 0
    fi
  fi
  mounts_in=()
  while IFS= read -r mount; do mounts_in+=("$mount"); done < <(jq -r --arg s "$service" '.services[$s].volumes[]? | select(.type == "volume") | .target' "$json")
  stage "$work/viewer.env" ${mounts_in[@]+"${mounts_in[@]}"}
  viewer_check "$label" "$work/viewer.env" "$file"
}

if [[ -n "${FERROEHR_VIEWER_BIN:-}" ]]; then
  # The viewer sits behind a compose profile; every profile is resolved.
  mounts_in=()
  export COMPOSE_PROFILES="*"
  # ── The hosted sandbox's viewer ────────────────────────────────────────────
  # The box's .env supplies these; a placeholder stands in for each.
  DATABASE_URL="$dsn" FERROPIX_DATABASE_URL="$dsn" FERROPIX_DATA_KEY=placeholder \
    "${compose[@]}" -f deploy/hosted/docker-compose.yml config --format json </dev/null >"$work/hosted.json"
  viewer_compose deploy/hosted/docker-compose.yml "$work/hosted.json" "$PWD/deploy/hosted"

  # ── The quickstart's viewer, alone and with each overlay ───────────────────
  "${compose[@]}" -f docker-compose.yml config --format json </dev/null >"$work/quickstart.json"
  viewer_compose docker-compose.yml "$work/quickstart.json" "$PWD"
  for overlay in docker-compose.*.yml; do
    "${compose[@]}" -f docker-compose.yml -f "$overlay" config --format json </dev/null >"$work/overlay.json" 2>/dev/null || continue
    viewer_compose "docker-compose.yml with $overlay" "$work/overlay.json" "$PWD"
  done

  # ── Every chart render that deploys the viewer ─────────────────────────────
  for golden in deploy/helm/golden/*.yaml; do
    yq -o=json 'select(.kind == "Deployment" and .metadata.name == "ferroehr-viewer") | .spec.template.spec.containers[0].env // []' "$golden" >"$work/chart-env.json"
    [[ -s "$work/chart-env.json" ]] || continue
    if jq -e 'map(select(.valueFrom)) | length > 0' "$work/chart-env.json" >/dev/null; then
      echo "::error file=$golden::the viewer Deployment reads a variable through valueFrom, which this check does not resolve" >&2
      fail=1
      continue
    fi
    jq -r '.[] | "\(.name)=\(.value)"' "$work/chart-env.json" >"$work/viewer.env"
    mounts_in=()
    while IFS= read -r mount; do mounts_in+=("$mount"); done < <(yq 'select(.kind == "Deployment" and .metadata.name == "ferroehr-viewer") | .spec.template.spec.containers[0].volumeMounts[].mountPath' "$golden")
    stage "$work/viewer.env" ${mounts_in[@]+"${mounts_in[@]}"}
    viewer_check "$golden (the viewer Deployment)" "$work/viewer.env"
  done
fi

exit "$fail"
