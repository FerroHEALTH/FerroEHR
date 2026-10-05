#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# End-to-end smoke test for the two container images (amd64), run against the
# TRUE end-user artifact: the standalone docker-compose.yml, pinned with an
# explicit -f. (The dev overlay stopped auto-merging in #2868 — its explicit
# name keeps this pin as belt-and-braces, not as the defence.)
#
# Only postgres + the server start (the viewer sits behind the
# `viewer` profile). The server configuration is the quickstart posture the
# compose `configs:` block carries inline — Basic user ferroehr/ferroehr,
# RBAC off. The steps:
#   1. waits for the app healthcheck to report healthy;
#   2. asserts GET /rest/status returns 200;
#   3. creates an EHR with the dev Basic credentials (ferroehr/ferroehr);
#   4. restarts the app and proves the second-boot migration run is a no-op
#      (the sqlx ledger is unchanged — the library migrator is silent on no-op,
#      so the row count is the reliable idempotency signal; app logs are dumped
#      for visibility);
#   5. tears everything down.
#
# FERROEHR_IMAGE / FERROEHR_POSTGRES_IMAGE select the images to run.
set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Own project (docs.docker.com/compose/how-tos/project-name) so the `down -v`
# teardown is scoped to `ferroehr-smoke` and never wipes a running dev
# (`ferroehr`) stack (issue #282 D3). Only the two core services are started
# (the viewer and seaweedfs are behind profiles and stay down).
export COMPOSE_PROJECT_NAME=ferroehr-smoke
# The single compose model for every call below: the standalone quickstart file
# ONLY. An explicit -f suppresses the automatic override merge
# (docs.docker.com/compose/how-tos/multiple-compose-files), so this smoke test
# exercises what a downloader actually runs, never the repo dev posture.
COMPOSE=(docker compose -f "$ROOT_DIR/docker-compose.yml")
BASE="http://localhost:8080/ferroehr/rest"
CORE_SERVICES=(ferroehr-postgres ferroehr)

cleanup() {
  echo "::group::app logs"
  "${COMPOSE[@]}" logs ferroehr || true
  echo "::endgroup::"
  "${COMPOSE[@]}" down -v || true
}
trap cleanup EXIT

wait_healthy() {
  local cid
  for _ in $(seq 1 60); do
    cid=$("${COMPOSE[@]}" ps -q ferroehr)
    if [[ -n "$cid" ]] && [[ "$(docker inspect -f '{{.State.Health.Status}}' "$cid")" = "healthy" ]]; then
      return 0
    fi
    sleep 5
  done
  echo "::error::app container did not become healthy"
  "${COMPOSE[@]}" ps
  "${COMPOSE[@]}" logs ferroehr || true
  return 1
}

migration_count() {
  # Query as the bootstrap superuser (peer auth over the container's unix
  # socket maps the postgres OS user to the postgres role). The second
  # generation keeps one ledger per migration set, so the count sums the five
  # (`ferroehr::db::prepare` runs them in this order); a set that failed to
  # apply shows as a smaller total, never as a missing relation.
  "${COMPOSE[@]}" exec -T ferroehr-postgres \
    psql -U postgres -d "${PG_INIT_DB:-ferroehr}" -tAc \
    "SELECT (SELECT count(*) FROM ext._sqlx_migrations)
          + (SELECT count(*) FROM clinical._sqlx_migrations)
          + (SELECT count(*) FROM party._sqlx_migrations)
          + (SELECT count(*) FROM linkage._sqlx_migrations)
          + (SELECT count(*) FROM audit._sqlx_migrations)" | tr -d '[:space:]'
}

echo "==> Starting core services"
"${COMPOSE[@]}" up -d --no-build "${CORE_SERVICES[@]}"

echo "==> Waiting for app to become healthy"
wait_healthy

echo "==> GET /rest/status must be 200"
code=$(curl -fsS -o /dev/null -w '%{http_code}' "$BASE/status")
[[ "$code" = "200" ]] || { echo "::error::/rest/status returned $code"; exit 1; }

echo "==> POST /ehr with dev Basic creds must be 201"
code=$(curl -sS -o /dev/null -w '%{http_code}' -u ferroehr:ferroehr \
  -X POST "$BASE/openehr/v1/ehr" \
  -H 'Prefer: return=minimal')
[[ "$code" = "201" ]] || { echo "::error::POST /ehr returned $code (expected 201)"; exit 1; }

echo "==> Recording migration ledger before restart"
before=$(migration_count)
echo "    _sqlx_migrations rows across the five sets = $before"
[[ "$before" -ge 1 ]] || { echo "::error::migrations did not apply on first boot"; exit 1; }

echo "==> Restarting app (second boot must be a migration no-op)"
"${COMPOSE[@]}" restart ferroehr
wait_healthy
after=$(migration_count)
echo "    _sqlx_migrations rows across the five sets = $after"
[[ "$before" = "$after" ]] || {
  echo "::error::migration count changed across restart ($before -> $after); not idempotent"
  exit 1
}

echo "==> Smoke test passed"
