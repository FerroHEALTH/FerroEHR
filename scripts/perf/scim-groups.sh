#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The cost of a large SCIM Group (#5015): a Group built to [scim]
# max_group_members (5 000) one member per change, then members removed and
# added at the cap, each change weighed against its budget: 200 ms at p99, 2
# MiB stored per change, 12 GiB stored by the build, one side-table row per
# change.
#
# Every membership change stores every membership of the Group again (RM
# demographic master02 §Versioning Semantics), so these budgets bound the
# version storage the cap allows. The run starts the quickstart's database on a
# fresh volume, migrates it and runs `scaleseed groups`, which writes through
# the platform library's own SCIM service. Unlike the other perf scripts this
# one judges: a missed budget fails the run, and the cap stands only while
# every budget holds. No openEHR spec governs this: our own design.
#
# Usage: scripts/perf/scim-groups.sh <postgres-image> <out-dir>
#   SCALESEED_BIN  the built seeder (default target/release/scaleseed)
#   PERF_DB_PORT   the host port the database is published on (default 55434)
#   Writes <out-dir>/report.{json,md} and <out-dir>/summary.md. Exit 0 when
#   every budget holds, 1 when one is missed, 2 on a usage error or a run that
#   could not be measured.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly ROOT_DIR
readonly PROJECT=ferroehr-scim-groups

usage() {
  echo "usage: $0 <postgres-image> <out-dir>" >&2
  exit 2
}

[[ "$#" -eq 2 ]] || usage
pg_image="$1"
out="$2"
db_port="${PERF_DB_PORT:-55434}"
scaleseed="${SCALESEED_BIN:-$ROOT_DIR/target/release/scaleseed}"
[[ -x "$scaleseed" ]] || {
  echo "error: no binary at $scaleseed (cargo build --release -p scaleseed)" >&2
  exit 2
}

export COMPOSE_PROJECT_NAME="$PROJECT"
export FERROEHR_POSTGRES_IMAGE="$pg_image"
export FERROEHR_DB_PORT="$db_port"
readonly DATABASE_URL="postgres://postgres:postgres@127.0.0.1:${db_port}/ferroehr"
mkdir -p "$out"
out="$(cd "$out" && pwd)"

COMPOSE=(-f "$ROOT_DIR/docker-compose.yml" -f "$ROOT_DIR/docker-compose.db-publish.yml")

down() {
  docker compose "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
}
trap down EXIT

down
if ! docker compose "${COMPOSE[@]}" up -d --wait --wait-timeout 300 ferroehr-postgres; then
  echo "::error::the database did not become healthy" >&2
  exit 2
fi
if ! "$scaleseed" --database-url "$DATABASE_URL" migrate; then
  echo "::error::the database could not be migrated" >&2
  exit 2
fi

measured=0
"$scaleseed" --database-url "$DATABASE_URL" groups --out "$out" || measured=$?
if [[ ! -f "$out/report.md" ]]; then
  echo "::error::the Group could not be measured" >&2
  exit 2
fi
tee "$out/summary.md" <"$out/report.md"
if [[ "$measured" -ne 0 ]]; then
  echo "::error::a SCIM Group at its cap missed a budget (#5015): the owner rules on a lower cap or another storage form" >&2
  exit 1
fi
