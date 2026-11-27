#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Records the viewer's replayed CDR (app/ferroehr-viewer/tests/fixtures/cdr/)
# against the FerroEHR of this checkout.
#
# 1. Starts a throwaway PostgreSQL (the published ferroehr-postgres image) and
#    the checkout's own `ferroehr` binary over docker/ferroehr.dev.toml, the
#    posture the browser journeys run, and binds its Basic users to PERSONs
#    through SCIM as scripts/ui-e2e.sh does.
# 2. Seeds it with the one demo seed (scripts/sandbox/reseed.sh) as the
#    journeys do: ferroehr-admin seeds, ferroehr is the clinician and
#    ferroehr-proxy the representative. The ids the screens open are read
#    from the ids file the seed writes.
# 3. Runs the `ssr_app` tests with the recorder on, each forwarding to the
#    CDR as ferroehr-admin and writing the exchanges it made.
# 4. Runs the recorder's last step, which merges them into recording.json and
#    manifest.json with stand-ins for every id and instant the run stamped and
#    for the dates the seed counted from the day it ran, so a second run
#    writes the same bytes.
#
# Usage:
#   scripts/viewer-record.sh
#
# Env:
#   VIEWER_RECORD_CDR       record against this running CDR instead (its
#                           database must be empty, and its logins bound and
#                           granted as scripts/ui-e2e.sh leaves them)
#   VIEWER_RECORD_PG_NAME   the PostgreSQL container's name (default
#                           ferroehr-viewer-record-pg)
#   VIEWER_RECORD_PG_IMAGE  the PostgreSQL image (default: the compose pin)
#   VIEWER_RECORD_PG_PORT   its host port (default 55432)
#   VIEWER_RECORD_PORT      the CDR's host port (default 18080)
#   VIEWER_RECORD_KEEP      if set, leave the work directory in place
#
# A podman host without a docker CLI runs the container through podman.
set -Eeuo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

PG_IMAGE="${VIEWER_RECORD_PG_IMAGE:-$(grep -o -m 1 'ghcr.io/ferrohealth/ferroehr-postgres:[^}]*' docker-compose.yml)}"
PG_PORT="${VIEWER_RECORD_PG_PORT:-55432}"
CDR_PORT="${VIEWER_RECORD_PORT:-18080}"
PG_NAME="${VIEWER_RECORD_PG_NAME:-ferroehr-viewer-record-pg}"
# The throwaway database's role, which the image's init script creates.
PG_ROLE="ferroehr"
PG_SECRET="ferroehr"
WORK="$(mktemp -d)"
SERVER_PID=""

if ! command -v docker >/dev/null 2>&1 && command -v podman >/dev/null 2>&1; then
  docker() { podman "$@"; }
fi

cleanup() {
  if [[ -n "$SERVER_PID" ]]; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  if [[ -z "${VIEWER_RECORD_CDR:-}" ]]; then
    docker rm -f "$PG_NAME" >/dev/null 2>&1 || true
  fi
  if [[ -z "${VIEWER_RECORD_KEEP:-}" ]]; then
    rm -rf "$WORK"
  else
    echo "work directory kept: $WORK"
  fi
}
trap cleanup EXIT

wait_for() {
  local url="$1" tries="$2"
  for _ in $(seq 1 "$tries"); do
    if curl -sf -o /dev/null "$url"; then return 0; fi
    sleep 1
  done
  echo "FATAL: $url did not answer" >&2
  return 1
}

# ── 1. The CDR ──────────────────────────────────────────────────────────────
if [[ -n "${VIEWER_RECORD_CDR:-}" ]]; then
  CDR_URL="${VIEWER_RECORD_CDR%/}"
else
  echo "── building the checkout's ferroehr"
  # The commit time stamps the build, so its build date is the checkout's.
  SOURCE_DATE_EPOCH="$(git log -1 --format=%ct)" \
    cargo build --locked -p ferroehr-server --bin ferroehr
  echo "── starting PostgreSQL ($PG_IMAGE)"
  docker rm -f "$PG_NAME" >/dev/null 2>&1 || true
  docker run -d --name "$PG_NAME" -p "127.0.0.1:$PG_PORT:5432" \
    -e POSTGRES_PASSWORD=postgres -e PG_INIT_USER="$PG_ROLE" \
    -e PG_INIT_PASSWORD="$PG_SECRET" -e PG_INIT_DB="$PG_ROLE" "$PG_IMAGE" >/dev/null
  for _ in $(seq 1 60); do
    if docker exec "$PG_NAME" pg_isready -U "$PG_ROLE" -d "$PG_ROLE" >/dev/null 2>&1; then break; fi
    sleep 1
  done
  CDR_URL="http://127.0.0.1:$CDR_PORT"
  # The dev configuration's key sets, at the paths the containers mount them.
  SERVER_ENV=(FERROEHR_CONFIG=docker/ferroehr.dev.toml
    FERROEHR__DATABASE__URL="postgres://$PG_ROLE:$PG_SECRET@127.0.0.1:$PG_PORT/$PG_ROLE"
    FERROEHR__SERVER__LISTEN="127.0.0.1:$CDR_PORT"
    "FERROEHR__AUDIT__PURPOSE_CODES=TREAT,ETREAT"
    FERROEHR__USAGE_REPORT__ENABLED=false
    FERROEHR__TELEMETRY__FORMAT=json
    FERROEHR__AUTH__ISSUER__BASIC_USERS__JWKS_FILE="$ROOT/deploy/hosted/basic-users-jwks.json"
    FERROEHR__AUTH__ISSUER__SCIM__JWKS_FILE="$ROOT/docs/conformance/party/scim/jwks.json")
  echo "── starting ferroehr on $CDR_URL"
  env "${SERVER_ENV[@]}" "$ROOT/target/debug/ferroehr" >"$WORK/ferroehr.log" 2>&1 &
  SERVER_PID=$!
fi
wait_for "$CDR_URL/health/readiness" 120
if [[ -z "${VIEWER_RECORD_CDR:-}" ]]; then
  echo "── binding the Basic users to PERSONs through SCIM"
  SCIM_GRANT="env ${SERVER_ENV[*]} $ROOT/target/debug/ferroehr" bash scripts/scim-provision.sh \
    "$CDR_URL" basic-users urn:example:basic-user \
    ferroehr ferroehr-admin --roles READONLY ferroehr-readonly
fi

ehrs=$(curl -sS --fail-with-body -u ferroehr-admin:ferroehr -X POST \
  "$CDR_URL/ferroehr/rest/openehr/v1/query/aql" -H "Content-Type: application/json" \
  -d '{"q":"SELECT COUNT(*) FROM EHR e"}' | jq -r '.rows[0][0]')
if [[ "$ehrs" != "0" ]]; then
  echo "FATAL: the CDR already holds $ehrs EHRs; the recording needs an empty one" >&2
  exit 1
fi

# ── 2. The seed ─────────────────────────────────────────────────────────────
date -u +%Y-%m-%dT%H:%M:%SZ >"$WORK/started"
# The days from the run its bodies' `__DATE:<days>__` tokens name, which the
# last step moves with the stand-in epoch.
grep -rhoE '__DATE:[+-]?[0-9]+__' scripts/sandbox/seed \
  | sed -E 's/__DATE:\+?(-?[0-9]+)__/\1/' | sort -un >"$WORK/seed-days"
echo "── seeding the demo dataset"
SANDBOX_BASE="$CDR_URL" SANDBOX_USER=ferroehr-admin SANDBOX_CLINICIAN=ferroehr \
  SANDBOX_TERMINOLOGY=without-loinc \
  SANDBOX_IDS_OUT="$WORK/seed-ids.json" bash scripts/sandbox/reseed.sh

# The ids the screens open, each read from the seed's ids file at the slot or
# key the seed's manifest gives it.
jq --slurpfile m scripts/sandbox/seed/manifest.json '
  $m[0] as $m | .tokens as $t | .ehrs as $e
  | ($m.world.legal_marks) as $marks
  | {
      ehr: $e[0],
      composition: $t["__SERIES:minimal-evaluation__"],
      held: $t["__VO:access-delay__"],
      ehr_access: $e[$m.world.ehr_access[0].ehr],
      ehr_restricted: $e[$marks.restriction.ehr],
      ehr_objected: $e[$marks.research_objection.ehr],
      ehr_due: $e[$marks.retention_anchors[0].ehr],
      retention_held: $t["__VO:retention-hold-1__"],
      ehr_care_ended: $e[$m.world.care_relationships[] | select(.key == "care-patient-02") | .ehr],
      patient: $t["__ID:patient-01__"],
      carer: $t["__ID:group-cardiology__"],
      care: $t["__ID:care-patient-01__"],
      care_ended: $t["__ID:care-patient-02__"],
      care_end: $t["__ID:care-patient-03__"],
      party: $t["__ID:person-01__"],
      organisation: $t["__ID:organisation-hospital__"],
      relationship: $t["__ID:representation-guardian__"],
      template: "minimal_evaluation.en.v1",
      query: "eu.ferroehr.sandbox::composition_index",
      query_version: "1.0.0"
    }' "$WORK/seed-ids.json" >"$WORK/manifest.json"
if jq -e '[.[] | select(. == null or . == "")] | length > 0' "$WORK/manifest.json" >/dev/null; then
  echo "FATAL: the seed left an id unnamed: $(cat "$WORK/manifest.json")" >&2
  exit 1
fi
echo "   seeded $(cat "$WORK/manifest.json")"
# A second later, so the marker falls after every instant the seed stamped.
sleep 1
date -u +%Y-%m-%dT%H:%M:%SZ >"$WORK/seeded"

# ── 3 and 4. The recording ──────────────────────────────────────────────────
export FERROEHR_VIEWER_RECORD_CDR="$CDR_URL"
export FERROEHR_VIEWER_RECORD_DIR="$WORK"
echo "── recording the ssr_app screens"
cargo nextest run --locked -p ferroehr-viewer -E 'test(/^ssr_app::/)'
echo "── writing the recording"
cargo nextest run --locked -p ferroehr-viewer --run-ignored only \
  -E 'test(=cdr_record::write_the_recording)'
echo "── wrote app/ferroehr-viewer/tests/fixtures/cdr/recording.json and manifest.json"
