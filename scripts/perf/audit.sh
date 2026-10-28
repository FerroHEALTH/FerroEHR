#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The logging component's performance evaluation (EHDS Annex II 1.1, Annex III
# point 2): one image, measured with the access log off and with it on, failing
# closed, over the release performance guard's workload.
#
# Each run starts the quickstart (docker-compose.yml) on fresh volumes, once per
# side, the order alternating between runs. The "on" side runs the load test
# with --audit, which counts the measured window's access records through ITI-81
# and times paged retrievals. The verdict on the claims stated in
# website/book/src/compliance/logging-performance.md is
# scripts/perf/audit-verdict.sh over the medians. No specification names a
# method: our own design.
#
# Usage: scripts/perf/audit.sh <image> <postgres-image> <out-dir>
#   LOADTEST_BIN   the built load test binary (default target/release/loadtest)
#   PERF_RUNS      runs per side, odd (default 3)
#   FERROEHR_PORT  the host port the quickstart publishes (default 8080)
#   Writes <out-dir>/{off,on}/run-N.json and <out-dir>/summary.md.
#   Exit 0 when every claim holds, 1 when one does not, 2 on a usage error or
#   a run that could not be measured.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly ROOT_DIR
readonly PROJECT=ferroehr-audit-perf
readonly COMPOSE_FILE="$ROOT_DIR/docker-compose.yml"

usage() {
  echo "usage: $0 <image> <postgres-image> <out-dir>" >&2
  exit 2
}

[[ "$#" -eq 3 ]] || usage
image="$1"
pg_image="$2"
out="$3"
runs="${PERF_RUNS:-3}"
if ! [[ "$runs" =~ ^[1-9][0-9]*$ ]] || ((runs % 2 == 0)); then
  echo "error: PERF_RUNS must be an odd positive number (got '$runs')" >&2
  exit 2
fi
loadtest="${LOADTEST_BIN:-$ROOT_DIR/target/release/loadtest}"
[[ -x "$loadtest" ]] || {
  echo "error: no load test binary at $loadtest (cargo build --release -p loadtest)" >&2
  exit 2
}
port="${FERROEHR_PORT:-8080}"
[[ "$port" =~ ^[0-9]+$ ]] || {
  echo "error: FERROEHR_PORT must be a port number (got '$port')" >&2
  exit 2
}
base_url="http://localhost:${port}/ferroehr/rest/openehr/v1"

# A test instance sends no usage report (the quickstart reads this switch).
export FERROEHR__USAGE_REPORT__ENABLED=false
export COMPOSE_PROJECT_NAME="$PROJECT"
mkdir -p "$out/off" "$out/on"

# The posture of each side, as a Compose override beside the quickstart: the
# quickstart passes no [audit] key through, so the override names them.
cat >"$out/off.compose.yml" <<'YAML'
services:
  ferroehr:
    environment:
      FERROEHR__AUDIT__ENABLED: "false"
YAML
cat >"$out/on.compose.yml" <<'YAML'
services:
  ferroehr:
    environment:
      FERROEHR__AUDIT__ENABLED: "true"
      FERROEHR__AUDIT__FAIL_MODE: closed
      FERROEHR__AUDIT__STORE__ENABLED: "true"
YAML

compose() {
  local side="$1"
  shift
  docker compose -f "$COMPOSE_FILE" -f "$out/$side.compose.yml" "$@"
}

down() {
  compose "$1" down -v --remove-orphans >/dev/null 2>&1 || true
}

# measure SIDE N: one run of SIDE on fresh volumes, its report at SIDE/run-N.json.
measure() {
  local side="$1" n="$2"
  local -a audit=()
  [[ "$side" == "on" ]] && audit=(--audit)
  echo "==> run $n: access log $side ($image)"
  down "$side"
  if ! FERROEHR_IMAGE="$image" FERROEHR_POSTGRES_IMAGE="$pg_image" \
    compose "$side" up -d --wait --wait-timeout 300 ferroehr-postgres ferroehr; then
    echo "::error::the stack with the access log $side did not become healthy" >&2
    compose "$side" logs ferroehr >&2 || true
    down "$side"
    exit 2
  fi
  if ! "$loadtest" --base-url "$base_url" ${audit[@]+"${audit[@]}"} --out "$out/$side/run-$n.json"; then
    echo "::error::the load test failed with the access log $side" >&2
    compose "$side" logs --tail 200 ferroehr >&2 || true
    down "$side"
    exit 2
  fi
  down "$side"
  jq -r '"    \(.throughput_ops_per_second * 10 | round / 10) op/s, p50 \(.p50_ms * 10 | round / 10) ms, p95 \(.p95_ms * 10 | round / 10) ms"' \
    "$out/$side/run-$n.json"
}

for n in $(seq 1 "$runs"); do
  if ((n % 2 == 1)); then
    measure off "$n"
    measure on "$n"
  else
    measure on "$n"
    measure off "$n"
  fi
done

status=0
bash "$ROOT_DIR/scripts/perf/audit-verdict.sh" "$out/off" "$out/on" >"$out/summary.md" || status=$?
{
  cat "$out/summary.md"
  printf "\nimage: \`%s\`; %s runs per side\n" "$image" "$runs"
} | tee "$out/summary.full.md"
mv "$out/summary.full.md" "$out/summary.md"
exit "$status"
