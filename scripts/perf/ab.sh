#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The performance guard's A/B run: the release candidate against the previous
# release, on one machine, with the same workload and data.
#
# Each side runs from its own quickstart file (the candidate from this
# checkout's docker-compose.yml, the previous release from the file at its
# tag), so each is measured in the posture it ships. Every run starts on fresh
# volumes, seeds the same records and drives the same fixed operation count
# with the load test (tools/loadtest); the order of the two sides alternates
# between runs so a runner that slows over the job penalises neither. The
# verdict is scripts/perf/compare.sh over the medians. No openEHR spec governs
# this: our own design.
#
# Usage: scripts/perf/ab.sh <previous-tag> <candidate-image> <candidate-postgres-image> <out-dir>
#   LOADTEST_BIN  the built load test binary (default target/release/loadtest)
#   PERF_RUNS     runs per side, odd (default 3)
#   Writes <out-dir>/{previous,candidate}/run-N.json and <out-dir>/summary.md.
#   Exit 0 within budget, 1 on a regression, 2 on a usage error or a run that
#   could not be measured.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly ROOT_DIR
readonly TAG_RE='^v[0-9]+\.[0-9]+\.[0-9]+$'
readonly PROJECT=ferroehr-perf
readonly BASE_URL="http://localhost:8080/ferroehr/rest/openehr/v1"

usage() {
  echo "usage: $0 <previous-tag> <candidate-image> <candidate-postgres-image> <out-dir>" >&2
  exit 2
}

[[ "$#" -eq 4 ]] || usage
prev_tag="$1"
cand_image="$2"
cand_pg_image="$3"
out="$4"
[[ "$prev_tag" =~ $TAG_RE ]] || {
  echo "error: '$prev_tag' is not a vX.Y.Z release tag" >&2
  exit 2
}
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

# A test instance sends no usage report (the quickstart reads this switch).
export FERROEHR__USAGE_REPORT__ENABLED=false
export COMPOSE_PROJECT_NAME="$PROJECT"
prev_version="${prev_tag#v}"
mkdir -p "$out/previous" "$out/candidate"

# The previous release's quickstart file, read at its tag. The file needs no
# checkout beside it, so it runs from the output directory.
if ! git -C "$ROOT_DIR" rev-parse -q --verify "refs/tags/$prev_tag^{commit}" >/dev/null; then
  git -C "$ROOT_DIR" fetch --no-tags --depth=1 origin "+refs/tags/$prev_tag:refs/tags/$prev_tag"
fi
git -C "$ROOT_DIR" show "$prev_tag:docker-compose.yml" >"$out/previous-compose.yml"

compose_of() {
  case "$1" in
  previous) echo "$out/previous-compose.yml" ;;
  candidate) echo "$ROOT_DIR/docker-compose.yml" ;;
  esac
}

down() {
  local file="$1"
  docker compose -f "$file" down -v --remove-orphans >/dev/null 2>&1 || true
}

# measure SIDE N: one run of SIDE on fresh volumes, its report at SIDE/run-N.json.
measure() {
  local side="$1" n="$2" file image pg_image
  file="$(compose_of "$side")"
  if [[ "$side" == "previous" ]]; then
    image="ghcr.io/ferrohealth/ferroehr:$prev_version"
    pg_image="ghcr.io/ferrohealth/ferroehr-postgres:$prev_version"
  else
    image="$cand_image"
    pg_image="$cand_pg_image"
  fi
  echo "==> run $n: $side ($image)"
  down "$file"
  if ! FERROEHR_IMAGE="$image" FERROEHR_POSTGRES_IMAGE="$pg_image" \
    docker compose -f "$file" up -d --wait --wait-timeout 300 ferroehr-postgres ferroehr; then
    echo "::error::the $side stack did not become healthy" >&2
    docker compose -f "$file" logs ferroehr >&2 || true
    down "$file"
    exit 2
  fi
  if ! "$loadtest" --base-url "$BASE_URL" --out "$out/$side/run-$n.json"; then
    echo "::error::the load test failed against the $side release" >&2
    docker compose -f "$file" logs --tail 200 ferroehr >&2 || true
    down "$file"
    exit 2
  fi
  down "$file"
  jq -r '"    \(.throughput_ops_per_second * 10 | round / 10) op/s, p95 \(.p95_ms * 10 | round / 10) ms"' \
    "$out/$side/run-$n.json"
}

for n in $(seq 1 "$runs"); do
  if ((n % 2 == 1)); then
    measure previous "$n"
    measure candidate "$n"
  else
    measure candidate "$n"
    measure previous "$n"
  fi
done

status=0
bash "$ROOT_DIR/scripts/perf/compare.sh" "$out/previous" "$out/candidate" >"$out/summary.md" || status=$?
{
  cat "$out/summary.md"
  printf "\nprevious release: \`%s\`; candidate: \`%s\`\n" "$prev_tag" "$cand_image"
} | tee "$out/summary.full.md"
mv "$out/summary.full.md" "$out/summary.md"
exit "$status"
