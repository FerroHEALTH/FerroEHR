#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Records the viewer's replayed CDR (app/ferroehr-viewer/tests/fixtures/cdr/)
# against the FerroEHR of this checkout.
#
# 1. Starts a throwaway PostgreSQL (the published ferroehr-postgres image) and
#    the checkout's own `ferroehr` binary over docker/ferroehr.dev.toml, with
#    the quickstart's posture of no role gate so the one user ferroehr /
#    ferroehr reads every surface the viewer draws.
# 2. Seeds it over ITS-REST as scripts/ui-e2e.sh does (the template, an EHR,
#    a composition committed then updated, a FLAT composition) plus what the
#    in-process screens read: the subject, a tag, a directory, a person, an
#    organisation, their relationship, a demographic tag and a stored query.
# 3. Runs the `ssr_app` tests with the recorder on, each forwarding to the
#    CDR and writing the exchanges it made.
# 4. Runs the recorder's last step, which merges them into recording.json and
#    manifest.json with stand-ins for every id and instant the run stamped,
#    so a second run writes the same bytes.
#
# Usage:
#   scripts/viewer-record.sh
#
# Env:
#   VIEWER_RECORD_CDR       record against this running CDR instead (its
#                           database must be empty; user ferroehr / ferroehr
#                           must read the admin API)
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
PG_NAME="ferroehr-viewer-record-pg"
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
    -e POSTGRES_PASSWORD=postgres -e PG_INIT_USER=ferroehr \
    -e PG_INIT_PASSWORD=ferroehr -e PG_INIT_DB=ferroehr "$PG_IMAGE" >/dev/null
  for _ in $(seq 1 60); do
    if docker exec "$PG_NAME" pg_isready -U ferroehr -d ferroehr >/dev/null 2>&1; then break; fi
    sleep 1
  done
  CDR_URL="http://127.0.0.1:$CDR_PORT"
  echo "── starting ferroehr on $CDR_URL"
  FERROEHR_CONFIG=docker/ferroehr.dev.toml \
    FERROEHR__DB__URL="postgres://ferroehr:ferroehr@127.0.0.1:$PG_PORT/ferroehr" \
    FERROEHR__SERVER__LISTEN="127.0.0.1:$CDR_PORT" \
    FERROEHR__AUTHZ__RBAC__ENABLED=false \
    FERROEHR__USAGE_REPORT__ENABLED=false \
    FERROEHR__TELEMETRY__FORMAT=json \
    "$ROOT/target/debug/ferroehr" >"$WORK/ferroehr.log" 2>&1 &
  SERVER_PID=$!
fi
wait_for "$CDR_URL/health/readiness" 120

V1="$CDR_URL/ferroehr/rest/openehr/v1"
AUTH=(-u ferroehr:ferroehr)
# quiet CURL-ARGS...: a call whose answer only matters when it fails.
quiet() {
  if ! curl -sS --fail-with-body -o "$WORK/answer" "$@"; then
    cat "$WORK/answer" >&2
    echo >&2
    return 1
  fi
}
# ask CURL-ARGS...: a call whose answer is printed, or refused with its body.
ask() {
  quiet "$@" || return 1
  cat "$WORK/answer"
}
ehrs=$(ask "${AUTH[@]}" -X POST "$V1/query/aql" -H "Content-Type: application/json" \
  -d '{"q":"SELECT COUNT(*) FROM EHR e"}' | jq -r '.rows[0][0]')
if [[ "$ehrs" != "0" ]]; then
  echo "FATAL: the CDR already holds $ehrs EHRs; the recording needs an empty one" >&2
  exit 1
fi

# ── 2. The seed ─────────────────────────────────────────────────────────────
date -u +%Y-%m-%dT%H:%M:%SZ >"$WORK/started"
echo "── seeding"
TEMPLATE="minimal_evaluation.en.v1"
quiet "${AUTH[@]}" -X POST "$V1/definition/template/adl1.4" \
  -H "Content-Type: application/xml" \
  --data-binary @app/ferroehr-viewer/tests/fixtures/minimal_evaluation.opt

# The subject is a synthetic record number (.claude/rules/personal-data.md).
EHR=$(ask "${AUTH[@]}" -X POST "$V1/ehr" -H "Content-Type: application/json" \
  -H "Accept: application/json" -H "Prefer: return=representation" -d '{
    "_type": "EHR_STATUS",
    "name": {"_type": "DV_TEXT", "value": "EHR Status"},
    "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
    "archetype_details": {"_type": "ARCHETYPED", "rm_version": "1.2.0",
      "archetype_id": {"_type": "ARCHETYPE_ID", "value": "openEHR-EHR-EHR_STATUS.generic.v1"}},
    "subject": {"_type": "PARTY_SELF", "external_ref": {"_type": "PARTY_REF",
      "namespace": "demo.hospital.mrn", "type": "PERSON",
      "id": {"_type": "GENERIC_ID", "value": "p-0417", "scheme": "mrn"}}},
    "is_queryable": true, "is_modifiable": true}' | jq -r '.ehr_id.value')

etag_of() { tr -d '\r' | sed -n 's/^[Ee][Tt]ag: W\/"\(.*\)"$/\1/p'; }

ask "${AUTH[@]}" "$V1/definition/template/adl1.4/$TEMPLATE/example" \
  -H "Accept: application/json" >"$WORK/example.json"
VUID=$(curl -sS --fail-with-body -D - -o /dev/null "${AUTH[@]}" -X POST "$V1/ehr/$EHR/composition" \
  -H "Content-Type: application/json" -H "Prefer: return=minimal" \
  --data-binary @"$WORK/example.json" | etag_of)
COMPOSITION="${VUID%%::*}"
quiet "${AUTH[@]}" -X PUT "$V1/ehr/$EHR/composition/$COMPOSITION" \
  -H "Content-Type: application/json" -H "If-Match: \"$VUID\"" \
  --data-binary @"$WORK/example.json"

FLAT_VUID=$(curl -sS --fail-with-body -D - -o /dev/null "${AUTH[@]}" -X POST "$V1/ehr/$EHR/composition" \
  -H "Content-Type: application/openehr.wt.flat+json" -H "openehr-template-id: $TEMPLATE" \
  -H "Prefer: return=minimal" -d '{
    "ctx/language": "en", "ctx/territory": "US",
    "ctx/composer_name": "Seed composer", "ctx/time": "2026-07-14T08:00:00Z",
    "minimal/minimal/quantity|magnitude": 36.5, "minimal/minimal/quantity|unit": "kg"}' \
  | etag_of)
COMPOSITION_FLAT="${FLAT_VUID%%::*}"

quiet "${AUTH[@]}" -X PUT "$V1/ehr/$EHR/composition/$COMPOSITION/tags" \
  -H "Content-Type: application/json" -d '[{"key": "reviewed", "value": "yes"}]'

quiet "${AUTH[@]}" -X POST "$V1/ehr/$EHR/directory" \
  -H "Content-Type: application/json" -d "{
    \"_type\": \"FOLDER\", \"name\": {\"_type\": \"DV_TEXT\", \"value\": \"root\"},
    \"archetype_node_id\": \"openEHR-EHR-FOLDER.generic.v1\",
    \"folders\": [{\"_type\": \"FOLDER\",
      \"name\": {\"_type\": \"DV_TEXT\", \"value\": \"Episodes\"},
      \"archetype_node_id\": \"at0001\",
      \"items\": [{\"_type\": \"OBJECT_REF\", \"namespace\": \"local\",
        \"type\": \"VERSIONED_COMPOSITION\",
        \"id\": {\"_type\": \"HIER_OBJECT_ID\", \"value\": \"$COMPOSITION\"}}]}]}"

# party RM_TYPE SEGMENT LABEL...: creates a party whose one identity holds
# one ELEMENT per label and prints its versioned-object id.
party() {
  local rm_type="$1" segment="$2"
  shift 2
  local archetype="openEHR-DEMOGRAPHIC-$rm_type.$segment.v1"
  local items
  items=$(jq -nc --args '[$ARGS.positional | to_entries[] | (.value | index(":")) as $at | {
    "_type": "ELEMENT", "name": {"_type": "DV_TEXT", "value": .value[0:$at]},
    "archetype_node_id": "at000\(.key + 2)",
    "value": {"_type": "DV_TEXT", "value": .value[$at + 1:]}}]' "$@")
  jq -nc --arg t "$rm_type" --arg a "$archetype" --arg s "$segment" --argjson items "$items" '{
    "_type": $t, "name": {"_type": "DV_TEXT", "value": $t}, "archetype_node_id": $a,
    "archetype_details": {"_type": "ARCHETYPED",
      "archetype_id": {"_type": "ARCHETYPE_ID", "value": $a}, "rm_version": "1.1.0"},
    "identities": [{"_type": "PARTY_IDENTITY",
      "name": {"_type": "DV_TEXT", "value": "legal name"},
      "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_IDENTITY.\($s)_name.v1",
      "details": {"_type": "ITEM_TREE", "name": {"_type": "DV_TEXT", "value": "Details"},
        "archetype_node_id": "at0001", "items": $items}}]}' \
    | ask "${AUTH[@]}" -X POST "$V1/demographic/$segment" \
      -H "Content-Type: application/json" -H "Accept: application/json" \
      -H "Prefer: return=representation" --data-binary @- \
    | jq -r '.uid.value | split("::")[0]'
}
PARTY=$(party PERSON person "Given name:Demo" "Family name:Person 0417")
ORGANISATION=$(party ORGANISATION organisation "Name:Demo Clinic")
RELATIONSHIP=$(jq -nc --arg s "$PARTY" --arg t "$ORGANISATION" '{
    "_type": "PARTY_RELATIONSHIP", "name": {"_type": "DV_TEXT", "value": "employment"},
    "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.employment.v1",
    "source": {"_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
      "id": {"_type": "HIER_OBJECT_ID", "value": $s}},
    "target": {"_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
      "id": {"_type": "HIER_OBJECT_ID", "value": $t}}}' \
  | ask "${AUTH[@]}" -X POST "$V1/demographic/party_relationship" \
    -H "Content-Type: application/json" -H "Accept: application/json" \
    -H "Prefer: return=representation" --data-binary @- \
  | jq -r '.uid.value | split("::")[0]')
quiet "${AUTH[@]}" -X PUT "$V1/demographic/person/$PARTY/tags" \
  -H "Content-Type: application/json" -d '[{"key": "follow-up", "value": "2026-11-04"}]'

QUERY="org.example.cardio::latest"
quiet "${AUTH[@]}" -X PUT "$V1/definition/query/$QUERY" \
  -H "Content-Type: text/plain" \
  --data-binary "SELECT e/ehr_id/value AS ehr, c/name/value AS name FROM EHR e CONTAINS COMPOSITION c"

jq -nc --arg ehr "$EHR" --arg composition "$COMPOSITION" \
  --arg composition_flat "$COMPOSITION_FLAT" --arg party "$PARTY" \
  --arg organisation "$ORGANISATION" --arg relationship "$RELATIONSHIP" \
  --arg template "$TEMPLATE" --arg query "$QUERY" \
  '{$ehr, $composition, $composition_flat, $party, $organisation, $relationship,
    $template, $query, "query_version": "1.0.0"}' >"$WORK/manifest.json"
for id in "$EHR" "$COMPOSITION" "$COMPOSITION_FLAT" "$PARTY" "$ORGANISATION" "$RELATIONSHIP"; do
  if [[ -z "$id" || "$id" == null ]]; then
    echo "FATAL: the seed is incomplete: $(cat "$WORK/manifest.json")" >&2
    exit 1
  fi
done
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
