#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The login lookup's cost at scale (#5087): one image measured over the
# binding side table the quickstart writes for its own user and over one seeded
# with 50 000 bound persons and 5 000 groups, and the lookup itself timed
# against its 2 ms p99 budget (#5004).
#
# The quickstart binds its Basic user, so every request looks the user up; the
# server refuses to start with a persons issuer it does not bind, so no side
# runs without the lookup. Both sides run the candidate from this checkout's
# docker-compose.yml on fresh volumes and drive the same fixed operation count
# with the load test (tools/loadtest), alternating which side runs first; the
# seeded side first seeds the side table with tools/scaleseed (`lookup
# --seed-only`). scripts/perf/compare.sh reports the two sides' medians, the
# seeded side as the candidate, so its ratios are what the 55 000 rows cost. After the last run `scaleseed lookup` times the lookup
# itself on that store. Every figure is reported; none fails the run. No
# openEHR spec governs this: our own design.
#
# Usage: scripts/perf/party-lookup.sh <image> <postgres-image> <out-dir>
#   LOADTEST_BIN   the built load test (default target/release/loadtest)
#   SCALESEED_BIN  the built seeder (default target/release/scaleseed)
#   PERF_RUNS      runs per side, odd (default 3)
#   PERF_DB_PORT   the host port the database is published on (default 55433)
#   Writes <out-dir>/{unseeded,seeded}/run-N.json, <out-dir>/lookup/report.{json,md}
#   and <out-dir>/summary.md. Exit 0 when everything was measured, 2 on a
#   usage error or a run that could not be measured.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly ROOT_DIR
readonly PROJECT=ferroehr-lookup
readonly BASE_URL="http://localhost:8080/ferroehr/rest/openehr/v1"

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
db_port="${PERF_DB_PORT:-55433}"
loadtest="${LOADTEST_BIN:-$ROOT_DIR/target/release/loadtest}"
scaleseed="${SCALESEED_BIN:-$ROOT_DIR/target/release/scaleseed}"
for bin in "$loadtest" "$scaleseed"; do
  [[ -x "$bin" ]] || {
    echo "error: no binary at $bin (cargo build --release -p loadtest -p scaleseed)" >&2
    exit 2
  }
done

# A test instance sends no usage report (the quickstart reads this switch).
export FERROEHR__USAGE_REPORT__ENABLED=false
export COMPOSE_PROJECT_NAME="$PROJECT"
export FERROEHR_IMAGE="$image" FERROEHR_POSTGRES_IMAGE="$pg_image"
export FERROEHR_DB_PORT="$db_port"
readonly DATABASE_URL="postgres://postgres:postgres@127.0.0.1:${db_port}/ferroehr"
mkdir -p "$out/unseeded" "$out/seeded"
out="$(cd "$out" && pwd)"

COMPOSE=(-f "$ROOT_DIR/docker-compose.yml" -f "$ROOT_DIR/docker-compose.db-publish.yml")

down() {
  docker compose "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
}

# measure SIDE N: one run of SIDE on fresh volumes, its report at SIDE/run-N.json;
# after the last run of the seeded side, the lookup itself is timed.
measure() {
  local side="$1" n="$2"
  echo "==> run $n: $side"
  down
  if ! docker compose "${COMPOSE[@]}" up -d --wait --wait-timeout 300 \
    ferroehr-postgres ferroehr; then
    echo "::error::the $side stack did not become healthy" >&2
    docker compose "${COMPOSE[@]}" logs ferroehr >&2 || true
    down
    exit 2
  fi
  if [[ "$side" == seeded ]] && ! "$scaleseed" --database-url "$DATABASE_URL" lookup --seed-only; then
    echo "::error::the side table could not be seeded" >&2
    down
    exit 2
  fi
  if ! "$loadtest" --base-url "$BASE_URL" --out "$out/$side/run-$n.json"; then
    echo "::error::the load test failed on the $side stack" >&2
    docker compose "${COMPOSE[@]}" logs --tail 200 ferroehr >&2 || true
    down
    exit 2
  fi
  jq -r '"    \(.throughput_ops_per_second * 10 | round / 10) op/s, p95 \(.p95_ms * 10 | round / 10) ms, p99 \(.p99_ms * 10 | round / 10) ms"' \
    "$out/$side/run-$n.json"
  if [[ "$side" == seeded && "$n" -eq "$runs" ]]; then
    "$scaleseed" --database-url "$DATABASE_URL" lookup --out "$out/lookup" || {
      echo "::error::the lookup could not be timed" >&2
      down
      exit 2
    }
  fi
  down
}

for n in $(seq 1 "$runs"); do
  if ((n % 2 == 1)); then
    measure unseeded "$n"
    measure seeded "$n"
  else
    measure seeded "$n"
    measure unseeded "$n"
  fi
done

compared=0
bash "$ROOT_DIR/scripts/perf/compare.sh" "$out/unseeded" "$out/seeded" >"$out/compare.md" || compared=$?
# NOTE: no openEHR spec governs this, our own design: the cost is reported, so
# compare.sh's regression exit (1) decides nothing here; a data error (2) does.
if [[ "$compared" -eq 2 ]]; then
  echo "error: the runs could not be compared" >&2
  exit 2
fi
{
  echo "## The login lookup's cost at scale (#5087)"
  echo
  echo "The previous side below runs over the quickstart's own binding rows, the candidate over 50 000 seeded persons and 5 000 groups."
  echo
  cat "$out/compare.md"
  echo
  cat "$out/lookup/report.md"
  printf "\nimage: \`%s\`\n" "$image"
} | tee "$out/summary.md"
