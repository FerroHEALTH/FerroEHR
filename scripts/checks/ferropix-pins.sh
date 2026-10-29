#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# FerroPIX pin guard: every FerroPIX the repository deploys is one release,
# and every FerroPIX configuration it ships reads under that release (#4392).
#
# Three places pin FerroPIX: the compose quickstart (docker-compose.yml), the
# Helm chart (values.yaml `ferropix.image` and the Chart.yaml
# `artifacthub.io/images` entry) and the hosted sandbox
# (deploy/hosted/docker-compose.yml, the default of FERROPIX_IMAGE). Dependabot
# watches the quickstart pin alone, so the chart sat at 0.0.7 and the sandbox
# at 0.0.10 while the quickstart moved to 0.0.12, each with a configuration
# the newer release refuses. This guard holds the three to one tag and digest,
# so a Dependabot proposal for the quickstart carries the other two with it,
# and runs the pinned image's `config check`, on no network, over the
# sandbox's ferropix.sandbox.toml and over the ferropix.toml each chart golden
# render carries (deploy/helm/validate.sh keeps those renders current).
#
# Usage: scripts/checks/ferropix-pins.sh                  pins and configurations
#        scripts/checks/ferropix-pins.sh --pins-only      pins alone, no docker
#        scripts/checks/ferropix-pins.sh --self-test      the pin reader on fixtures
# Needs docker for the configuration check, and yq (mikefarah) and jq.
# No specification governs this guard: our own design.
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
GRAMMAR="[--pins-only | --self-test]"
mode=all
case "${1:-}" in
  "") ;;
  --pins-only) mode=pins ;;
  --self-test) mode=self-test ;;
  *) guard_usage "$GRAMMAR" ;;
esac
[[ "$#" -le 1 ]] || guard_usage "$GRAMMAR"

# Prints `<place> <tag>@sha256:<digest>` for each pin under the tree $1; a pin
# that cannot be read prints `<place> missing`.
read_pins() {
  local root="$1" pin tag digest
  pin="$(sed -nE 's#^ +image: ghcr\.io/ferrohealth/ferropix:([^@[:space:]]+@sha256:[0-9a-f]{64}).*#\1#p' \
    "$root/docker-compose.yml" | sed -n 1p)"
  echo "docker-compose.yml ${pin:-missing}"
  tag="$(yq -r '.ferropix.image.tag // ""' "$root/deploy/helm/ferroehr/values.yaml")"
  digest="$(yq -r '.ferropix.image.digest // ""' "$root/deploy/helm/ferroehr/values.yaml")"
  if [[ -n "$tag" && "$digest" =~ ^sha256:[0-9a-f]{64}$ ]]; then
    echo "deploy/helm/ferroehr/values.yaml $tag@$digest"
  else
    echo "deploy/helm/ferroehr/values.yaml missing"
  fi
  pin="$(sed -nE 's#.*image: ghcr\.io/ferrohealth/ferropix:([^@[:space:]]+@sha256:[0-9a-f]{64}).*#\1#p' \
    "$root/deploy/helm/ferroehr/Chart.yaml" | sed -n 1p)"
  echo "deploy/helm/ferroehr/Chart.yaml ${pin:-missing}"
  pin="$(sed -nE 's#.*\$\{FERROPIX_IMAGE:-ghcr\.io/ferrohealth/ferropix:([^@}]+@sha256:[0-9a-f]{64})\}.*#\1#p' \
    "$root/deploy/hosted/docker-compose.yml" | sed -n 1p)"
  echo "deploy/hosted/docker-compose.yml ${pin:-missing}"
}

# Fails unless every pin under $1 is read and names the quickstart's release.
# Prints the pin on success.
check_pins() {
  local root="$1" pins want bad=0 place pin
  pins="$(read_pins "$root")"
  want="$(awk 'NR == 1 { print $2 }' <<<"$pins")"
  while read -r place pin; do
    if [[ "$pin" == missing ]]; then
      echo "::error file=$place::no FerroPIX pin by tag and digest could be read here" >&2
      bad=1
    elif [[ "$pin" != "$want" ]]; then
      echo "::error file=$place::FerroPIX is pinned at $pin here and at $want in docker-compose.yml; move every FerroPIX pin together, with the configuration that release reads" >&2
      bad=1
    fi
  done <<<"$pins"
  [[ "$bad" -eq 0 ]] || return 1
  echo "$want"
}

self_test() {
  local here work fixture
  here="$(cd "$(dirname "$0")/../.." && pwd)"
  work="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand now: $work is local to this function
  trap "rm -rf '$work'" EXIT
  fixture="$work/tree"
  mkdir -p "$fixture/deploy/helm/ferroehr" "$fixture/deploy/hosted"
  local d1 d2
  d1="$(printf 'a%.0s' {1..64})"
  d2="$(printf 'b%.0s' {1..64})"
  write() {
    cat > "$fixture/docker-compose.yml" <<EOF
services:
  ferropix:
    image: ghcr.io/ferrohealth/ferropix:$1@sha256:$2
EOF
    cat > "$fixture/deploy/helm/ferroehr/values.yaml" <<EOF
ferropix:
  image:
    tag: "$3"
    digest: "sha256:$4"
EOF
    cat > "$fixture/deploy/helm/ferroehr/Chart.yaml" <<EOF
annotations:
  artifacthub.io/images: |
    - name: ferropix
      image: ghcr.io/ferrohealth/ferropix:$5@sha256:$6  # party: third
EOF
    cat > "$fixture/deploy/hosted/docker-compose.yml" <<EOF
services:
  ferropix:
    image: \${FERROPIX_IMAGE:-ghcr.io/ferrohealth/ferropix:$7@sha256:$8}
EOF
  }
  local out
  write 1.0.0 "$d1" 1.0.0 "$d1" 1.0.0 "$d1" 1.0.0 "$d1"
  out="$(check_pins "$fixture" 2>&1)" || { echo "self-test: agreeing pins were refused: $out" >&2; exit 1; }
  [[ "$out" == "1.0.0@sha256:$d1" ]] || { echo "self-test: read $out" >&2; exit 1; }
  local case_args
  for case_args in \
    "chart tag|1.0.0 $d1 0.9.0 $d1 1.0.0 $d1 1.0.0 $d1|values.yaml" \
    "chart digest|1.0.0 $d1 1.0.0 $d2 1.0.0 $d1 1.0.0 $d1|values.yaml" \
    "annotation|1.0.0 $d1 1.0.0 $d1 0.9.0 $d2 1.0.0 $d1|Chart.yaml" \
    "hosted|1.0.0 $d1 1.0.0 $d1 1.0.0 $d1 0.9.0 $d2|hosted/docker-compose.yml"; do
    IFS='|' read -r name args file <<<"$case_args"
    # shellcheck disable=SC2086 # the eight fields split on purpose
    write $args
    if out="$(check_pins "$fixture" 2>&1)"; then
      echo "self-test: a stale $name pin passed" >&2
      exit 1
    fi
    [[ "$out" == *"$file"* ]] || { echo "self-test: the $name refusal names no $file: $out" >&2; exit 1; }
  done
  : > "$fixture/deploy/hosted/docker-compose.yml"
  out="$(check_pins "$fixture" 2>&1)" && { echo "self-test: a missing hosted pin passed" >&2; exit 1; }
  [[ "$out" == *"could be read"* ]] || { echo "self-test: a missing pin is not named: $out" >&2; exit 1; }
  check_pins "$here" > /dev/null
  echo "ferropix-pins: self-test OK (agreeing pins pass; a stale chart tag, chart digest, annotation or hosted pin and a missing pin are refused by file)"
}

if [[ "$mode" == self-test ]]; then
  self_test
  exit 0
fi

cd "$(dirname "$0")/../.." || exit 1
pin="$(check_pins .)"
echo "  ok: every FerroPIX pin is $pin"
[[ "$mode" == pins ]] && exit 0

image="ghcr.io/ferrohealth/ferropix:$pin"
docker pull --quiet "$image" > /dev/null || { echo "::error::$image cannot be pulled" >&2; exit 1; }
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
head -c 32 /dev/urandom | base64 > "$work/data-key"
echo "postgres://ferropix:ci@db.invalid:5432/ferropix" > "$work/db-url"

# Runs the pinned image's `config check` over the configuration file $3, read
# from the committed file $1 ($2 says where in it), with dummy secrets at the
# paths its [db] names.
config_check() {
  local source="$1" name="$1$2" file="$3" url_file key_file
  url_file="$(sed -nE 's/^[[:space:]]*url_file[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' "$file" | sed -n 1p)"
  key_file="$(sed -nE 's/^[[:space:]]*data_key_file[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' "$file" | sed -n 1p)"
  chmod 0644 "$file" "$work/data-key" "$work/db-url"
  local args=(--rm --network none -e FERROPIX_CONFIG=/etc/ferropix-check/ferropix.toml
    -e FERROPIX__USAGE_REPORT__ENABLED=false -v "$file:/etc/ferropix-check/ferropix.toml:ro")
  [[ -z "$url_file" ]] || args+=(-v "$work/db-url:$url_file:ro")
  [[ -z "$key_file" ]] || args+=(-v "$work/data-key:$key_file:ro")
  if docker run "${args[@]}" "$image" config check > "$work/out" 2>&1; then
    echo "  ok: $name reads under $image"
  else
    echo "::error file=$source::$image refuses $name:" >&2
    cat "$work/out" >&2
    return 1
  fi
}

fail=0
cp deploy/hosted/ferropix.sandbox.toml "$work/hosted.toml"
config_check deploy/hosted/ferropix.sandbox.toml "" "$work/hosted.toml" || fail=1
for golden in deploy/helm/golden/*.yaml; do
  yq -r 'select(.kind == "ConfigMap" and (.metadata.name | test("ferropix$"))) | .data."ferropix.toml"' \
    "$golden" > "$work/chart.toml"
  [[ -s "$work/chart.toml" ]] || continue
  config_check "$golden" " (its ferropix.toml)" "$work/chart.toml" || fail=1
done
exit "$fail"
