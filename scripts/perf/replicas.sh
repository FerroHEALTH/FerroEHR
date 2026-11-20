#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The reload load of several instances on one PostgreSQL (#5590, moved from
# #5589): `scripts/perf/gate.sh replicas`. Sourced by gate.sh, which defines
# `call`, `die`, `wait_for`, `signing_key`, the access seed and the gate's
# constants; never run directly.
#
# For each instance count it starts the FerroEHR PostgreSQL with
# pg_stat_statements and that many servers of the image under test, each
# under this tree's gate configuration and signing key, as the "Running
# several instances" page deploys them: every instance on the primary, no
# sticky sessions. The first instance is provisioned and seeded with the
# access path (scripts/perf/access-seed.sh). After the instances settle, it
# makes one change of each kind the copies listen for and measures the window
# after it:
#   none      no change, the idle floor every other window is read against;
#   grant     a grant written through `ferroehr party-binding grant`, which
#             moves the grant revision, so each copy reads every grant;
#   facts     a patient admitted to the care team, which moves the facts
#             revision alone;
#   policy    a new version of the access policy;
#   settings  new directives on an EHR, which drop that EHR from each copy.
# Each window counts the transactions and the statements the database ran,
# and reads each instance's grants refreshes from its Prometheus surface. The
# connections the instances hold are counted once they settle. One JSON line
# per instance count and change is written to $OUT/replicas.jsonl. No openEHR
# spec governs this: our own design.

readonly RELOAD_PG="${PREFIX}-reload-postgres"
readonly RELOAD_SERVER="${PREFIX}-reload"
readonly RELOAD_PG_PORT=55434
readonly RELOAD_HTTP_PORT=58090
# The seconds a change is given before its window is read: the copies read on
# the notification, well inside it.
readonly RELOAD_WINDOW=3

replicas_teardown() {
  local i
  for i in 1 2 3 4 5 6 7 8 9; do
    docker rm -f "$RELOAD_SERVER-$i" >/dev/null 2>&1 || true
  done
  docker rm -f "$RELOAD_PG" >/dev/null 2>&1 || true
  docker network rm "$NETWORK" >/dev/null 2>&1 || true
}

# reload_sql <statement>: one statement as the superuser, unaligned.
reload_sql() {
  docker exec "$RELOAD_PG" psql -U postgres -d ferroehr -Atq -c "$1"
}

# reload_up <image> <pg-image> <instances>: the database and the instances,
# the first provisioned and seeded.
reload_up() {
  local image="$1" pg_image="$2" count="$3" i
  docker network create "$NETWORK" >/dev/null
  docker run -d --name "$RELOAD_PG" --network "$NETWORK" -p "127.0.0.1:${RELOAD_PG_PORT}:5432" \
    -e "POSTGRES_PASSWORD=$PG_SUPER_SECRET" -e PG_INIT_USER=ferroehr -e "PG_INIT_PASSWORD=$PG_SECRET" \
    -e PG_INIT_DB=ferroehr "$pg_image" \
    -c shared_preload_libraries=pg_stat_statements -c pg_stat_statements.track=all >/dev/null
  wait_for "PostgreSQL" 60 docker exec "$RELOAD_PG" pg_isready -h 127.0.0.1 -U ferroehr -d ferroehr
  reload_sql "CREATE EXTENSION IF NOT EXISTS pg_stat_statements" >/dev/null
  for ((i = 1; i <= count; i++)); do
    docker run -d --name "$RELOAD_SERVER-$i" --network "$NETWORK" \
      -p "127.0.0.1:$((RELOAD_HTTP_PORT + i)):8080" --read-only --tmpfs /tmp --cap-drop ALL \
      --security-opt no-new-privileges:true \
      -e FERROEHR_CONFIG=/etc/ferroehr/ferroehr.toml \
      -e "FERROEHR__DATABASE__URL=postgres://ferroehr:${PG_SECRET}@${RELOAD_PG}:5432/ferroehr" \
      -e FERROEHR__USAGE_REPORT__ENABLED=false \
      -e FERROEHR__MANAGEMENT__ENABLED=true \
      -e FERROEHR__MANAGEMENT__ENDPOINTS__PROMETHEUS=admin_only \
      -v "$ROOT/perf/ferroehr.toml:/etc/ferroehr/ferroehr.toml:ro" \
      -v "$ROOT/docs/conformance/party/scim/jwks.json:/etc/ferroehr/scim-jwks.json:ro" \
      -v "$OUT/signing/signing.asc:/etc/ferroehr/signing.asc:ro" \
      -v "$OUT/signing/passphrase:/etc/ferroehr/signing-passphrase:ro" \
      "$image" >/dev/null
    wait_for "FerroEHR $i" 120 curl -fsS "http://127.0.0.1:$((RELOAD_HTTP_PORT + i))/health/readiness"
  done
  SERVER_URL="http://127.0.0.1:$((RELOAD_HTTP_PORT + 1))"
  local -a grant=(docker exec "$RELOAD_SERVER-1" /usr/local/bin/ferroehr)
  SCIM_GRANT="${grant[*]}" bash "$ROOT/scripts/scim-provision.sh" "$SERVER_URL" basic-users \
    urn:example:basic-user ferroehr >/dev/null
  ACCESS_IDS="$OUT/scim-ids-reload"
  : >"$ACCESS_IDS"
  SCIM_IDS="$ACCESS_IDS" SCIM_GRANT="${grant[*]}" bash "$ROOT/scripts/scim-provision.sh" \
    "$SERVER_URL" basic-users urn:example:basic-user --roles USER clinician >/dev/null
  ACCESS_FERROEHR=("${grant[@]}")
  call POST /definition/template/adl1.4 application/xml "$TEMPLATE" 201 >/dev/null
  access_seed
  RELOAD_EHR=$(access_patient_ehr "$ACCESS_TEAM")
}

# The transactions the database has committed and rolled back.
reload_transactions() {
  reload_sql "SELECT xact_commit + xact_rollback FROM pg_stat_database WHERE datname = 'ferroehr'"
}

# The statements pg_stat_statements counted since its reset, its own reads
# and this script's left out.
reload_statements() {
  reload_sql "SELECT coalesce(sum(calls), 0) FROM pg_stat_statements \
    WHERE query NOT ILIKE '%pg_stat%' AND query NOT ILIKE '%pg_stat_database%'"
}

# reload_refreshes <instance>: the sum in seconds and the count of the grants
# refreshes instance <instance> made, of the kinds that read rows.
reload_refreshes() {
  curl -fsS -H "authorization: $AUTH" "http://127.0.0.1:$((RELOAD_HTTP_PORT + $1))/management/prometheus" \
    | awk '/^grants_refresh_duration_seconds_(sum|count)\{/ && /kind="(grants|facts)"/ {
        if ($1 ~ /_sum\{/) sum += $2; else count += $2 }
        END { printf "%.6f %d\n", sum, count }'
}

# reload_change <kind>: one change of <kind>.
reload_change() {
  local version
  case "$1" in
  none) ;;
  grant)
    "${ACCESS_FERROEHR[@]}" party-binding grant --basic-user ferroehr --operation access_service >/dev/null
    ;;
  facts) access_patient_ehr "$ACCESS_TEAM" >/dev/null ;;
  policy)
    # The version in force is the read's (weak) ETag.
    version=$(curl -fsS -o /dev/null -D - -H "authorization: $AUTH" "$SERVER_URL$EXT/admin/access_policy" \
      | awk 'tolower($1) == "etag:" { sub(/^W\//, "", $2); gsub(/[\r"]/, "", $2); print $2 }')
    [[ -n "$version" ]] || die "replicas: the access policy read named no version"
    curl -fsS -o /dev/null -X PUT -H "authorization: $AUTH" -H "content-type: application/json" \
      -H "if-match: $version" --data-binary "@$OUT/access-policy.json" \
      "$SERVER_URL$EXT/admin/access_policy" || die "replicas: the policy change was refused"
    ;;
  settings) access_directives "$RELOAD_EHR" ;;
  esac
}

# reload_window <instances> <kind>: one change and its window, as one JSON
# line.
reload_window() {
  local count="$1" kind="$2" i before after seconds=0 made=0 sum n was_sum was_n
  local -a held=()
  for ((i = 1; i <= count; i++)); do held+=("$(reload_refreshes "$i")"); done
  reload_sql "SELECT pg_stat_statements_reset()" >/dev/null
  before=$(reload_transactions)
  reload_change "$kind"
  sleep "$RELOAD_WINDOW"
  after=$(reload_transactions)
  for ((i = 1; i <= count; i++)); do
    read -r sum n <<<"$(reload_refreshes "$i")"
    read -r was_sum was_n <<<"${held[$((i - 1))]}"
    seconds=$(awk -v a="$sum" -v b="$was_sum" -v s="$seconds" 'BEGIN { printf "%.6f", s + a - b }')
    made=$((made + n - was_n))
  done
  jq -cn --argjson instances "$count" --arg change "$kind" \
    --argjson transactions "$((after - before))" --argjson statements "$(reload_statements)" \
    --argjson refreshes "$made" --argjson seconds "$seconds" \
    '{instances: $instances, change: $change, transactions: $transactions,
      statements: $statements, refreshes: $refreshes,
      refresh_ms: (if $refreshes > 0 then ($seconds * 1000 / $refreshes) else null end)}'
}

# replicas <image> <pg-image> [<instances>...]: every window at each count.
replicas() {
  [[ "$#" -ge 2 ]] || usage
  local image="$1" pg_image="$2" count kind connections
  shift 2
  local -a counts=("$@")
  [[ "${#counts[@]}" -gt 0 ]] || counts=(1 2 3)
  mkdir -p "$OUT"
  : >"$OUT/replicas.jsonl"
  signing_key
  for count in "${counts[@]}"; do
    [[ "$count" =~ ^[1-9]$ ]] || die "an instance count is 1 to 9, got '$count'"
    replicas_teardown
    reload_up "$image" "$pg_image" "$count"
    sleep 15
    connections=$(reload_sql "SELECT count(*) FROM pg_stat_activity \
      WHERE datname = 'ferroehr' AND backend_type = 'client backend' AND application_name <> 'psql'")
    for kind in none grant facts policy settings; do
      reload_window "$count" "$kind" | jq -c --argjson connections "$connections" \
        '. + {connections: $connections}' >>"$OUT/replicas.jsonl"
    done
    echo "perf gate: replicas measured at $count instance(s)"
  done
  replicas_teardown
  cat "$OUT/replicas.jsonl"
}
