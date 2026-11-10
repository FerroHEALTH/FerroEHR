#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The performance gate (#5081): one FerroEHR and its PostgreSQL, idle and then
# under the committed request set, judged by ferrohealth-perf against the
# committed baseline. The run fails when the server works while idle (CPU, or
# transactions in its database), or is slower, heavier or busier under load
# than perf/baseline.toml allows. No openEHR spec governs this: our own design.
#
# The server runs in a container of the image under test with
# perf/ferroehr.toml as its configuration, its database in a container of the
# FerroEHR PostgreSQL image, both on a network of their own; ferrohealth-perf
# reads the server's cgroup and counts the transactions of its database.
# Before serve, the professional templates of deploy/definitions/professional/
# are loaded, and once it is ready the Basic user is bound to a PERSON through
# SCIM (scripts/scim-provision.sh). The seed writes one EHR the reads use and
# never changes, one EHR the commits write into, the lab-results template with
# a result, and a PERSON, and the plan is rendered with their ids
# (perf/plan.toml, `@NAME@`).
#
# Usage:
#   scripts/perf/gate.sh install                     the pinned ferrohealth-perf into target/tools/perf
#   scripts/perf/gate.sh run <image> <pg-image>      the gate
#   scripts/perf/gate.sh update <image> <pg-image>   the gate, writing perf/baseline.toml
#   scripts/perf/gate.sh self-test <image> <pg-image>
#       an idle busy-loop against the database must fail the idle phase,
#       naming idle.transactions_per_second
#   scripts/perf/gate.sh pin                         print the pinned version
#
# Environment:
#   PERF_OUT      the report directory (default target/perf)
#   PERF_BIN      a ferrohealth-perf binary (default target/tools/perf/bin/ferrohealth-perf)
#   PERF_SERVER   with PERF_DATABASE and PERF_TARGET, measure a server already
#                 running (its base URL), whose database URL and ferrohealth-perf
#                 target (`--pid N` or `--cgroup DIR`) they name, and start no
#                 container: a developer machine without docker
#
# Exit codes: 0 the run passed, 1 a breach, 2 the run could not be judged or a
# usage or harness error, as ferrohealth-perf answers.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
readonly ROOT
readonly PREFIX=ferroehr-perf
readonly NETWORK="${PREFIX}-net"
readonly PG="${PREFIX}-postgres"
readonly SERVER="${PREFIX}-server"
# The throwaway database's roles, which the image's init script creates.
readonly PG_SECRET=ferroehr
readonly PG_SUPER_SECRET=postgres
readonly PG_PORT=55432
readonly HTTP_PORT=58080
readonly BASE="/ferroehr/rest/openehr/v1"
readonly AUTH="Basic ZmVycm9laHI6ZmVycm9laHI="
readonly TEMPLATE="$ROOT/corpus/templates/ckm/generic-lab-test-result.opt"
readonly COMPOSITION="$ROOT/corpus/templates/ckm/generic-lab-test-result.example.json"
readonly PERSON="$ROOT/corpus/fixtures/demographic/person.v1.json"
OUT="${PERF_OUT:-$ROOT/target/perf}"
readonly OUT

usage() {
  sed -n '21,29p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

die() {
  echo "perf gate: $*" >&2
  exit 2
}

# The ferrohealth-perf version docs/VERSIONS.md pins.
pin() {
  local version
  version=$(awk -F'|' '$2 ~ /`ferrohealth-perf`/ { split($3, cell, "`"); print cell[2] }' "$ROOT/docs/VERSIONS.md")
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "docs/VERSIONS.md pins no ferrohealth-perf version"
  echo "$version"
}

binary() {
  local bin="${PERF_BIN:-$ROOT/target/tools/perf/bin/ferrohealth-perf}" want have
  [[ -x "$bin" ]] || die "no ferrohealth-perf at $bin: run scripts/perf/gate.sh install"
  want=$(pin)
  have=$("$bin" --version | awk '{ print $2 }')
  [[ "$have" == "$want" ]] || die "$bin is ferrohealth-perf $have; docs/VERSIONS.md pins $want"
  echo "$bin"
}

install() {
  cargo install ferrohealth-perf --version "=$(pin)" --locked --root "$ROOT/target/tools/perf"
}

# Stops the self-test's busy loop and waits for it.
stop_busy() {
  [[ -n "${BUSY_PID:-}" ]] || return 0
  touch "$BUSY_STOP"
  wait "$BUSY_PID" 2>/dev/null || true
  BUSY_PID=""
}

teardown() {
  stop_busy
  [[ -n "${PERF_SERVER:-}" ]] && return 0
  if [[ "${STARTED:-0}" == 1 ]]; then
    docker logs --tail 40 "$SERVER" >"$OUT/server.log" 2>&1 || true
  fi
  docker rm -f "$SERVER" "$PG" >/dev/null 2>&1 || true
  docker network rm "$NETWORK" >/dev/null 2>&1 || true
}

wait_for() { # <what> <seconds> <command...>
  local what="$1" seconds="$2" i
  shift 2
  for ((i = 0; i < seconds; i++)); do
    if "$@" >/dev/null 2>&1; then return 0; fi
    sleep 1
  done
  die "$what was not ready after ${seconds}s"
}

up() { # <image> <pg-image>
  local image="$1" pg_image="$2"
  teardown
  docker network create "$NETWORK" >/dev/null
  docker run -d --name "$PG" --network "$NETWORK" -p "127.0.0.1:${PG_PORT}:5432" \
    -e "POSTGRES_PASSWORD=$PG_SUPER_SECRET" -e PG_INIT_USER=ferroehr -e "PG_INIT_PASSWORD=$PG_SECRET" \
    -e PG_INIT_DB=ferroehr "$pg_image" >/dev/null
  wait_for "PostgreSQL" 60 docker exec "$PG" pg_isready -h 127.0.0.1 -U ferroehr -d ferroehr
  STARTED=1
  local -a process=(--network "$NETWORK" --read-only --tmpfs /tmp --cap-drop ALL
    --security-opt no-new-privileges:true
    -e FERROEHR_CONFIG=/etc/ferroehr/ferroehr.toml
    -e "FERROEHR__DATABASE__URL=postgres://ferroehr:${PG_SECRET}@${PG}:5432/ferroehr"
    -e FERROEHR__USAGE_REPORT__ENABLED=false
    -v "$ROOT/perf/ferroehr.toml:/etc/ferroehr/ferroehr.toml:ro"
    -v "$ROOT/docs/conformance/party/scim/jwks.json:/etc/ferroehr/scim-jwks.json:ro"
    -v "$ROOT/deploy/definitions/professional:/etc/ferroehr/definitions:ro")
  # The professional templates the party mapping names are stored before serve.
  docker run --rm "${process[@]}" "$image" definitions load \
    /etc/ferroehr/definitions/openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0.adls \
    /etc/ferroehr/definitions/openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1.0.0.adls >/dev/null
  docker run -d --name "$SERVER" -p "127.0.0.1:${HTTP_PORT}:8080" "${process[@]}" \
    "$image" >/dev/null
  SERVER_URL="http://127.0.0.1:${HTTP_PORT}"
  DATABASE_URL="postgres://postgres:${PG_SUPER_SECRET}@127.0.0.1:${PG_PORT}/ferroehr"
  TARGET=(--container "$(docker inspect -f '{{.Id}}' "$SERVER")")
  wait_for "FerroEHR" 120 curl -fsS "$SERVER_URL/health/readiness"
  bash "$ROOT/scripts/scim-provision.sh" "$SERVER_URL" basic-users ferroehr >/dev/null
}

# call <method> <path> <content-type> <body-file|-> <want-status>: the
# response body on stdout and the headers in $HEADERS.
call() {
  local method="$1" path="$2" type="$3" body="$4" want="$5" status
  local -a data=()
  [[ "$body" != - ]] && data=(--data-binary "@$body" -H "content-type: $type")
  status=$(curl -sS -o "$OUT/seed.body" -D "$OUT/seed.headers" -w '%{http_code}' -X "$method" \
    -H "authorization: $AUTH" -H "accept: application/json" -H "prefer: return=representation" \
    ${data[@]+"${data[@]}"} "$SERVER_URL$BASE$path")
  [[ "$status" == "$want" ]] || die "seed: $method $path answered $status, want $want: $(head -c 400 "$OUT/seed.body")"
  cat "$OUT/seed.body"
}

seed() {
  local read_ehr write_ehr uid party
  call POST /definition/template/adl1.4 application/xml "$TEMPLATE" 201 >/dev/null
  read_ehr=$(call POST /ehr "" - 201 | jq -er '.ehr_id.value')
  write_ehr=$(call POST /ehr "" - 201 | jq -er '.ehr_id.value')
  uid=$(call POST "/ehr/$read_ehr/composition" application/json "$COMPOSITION" 201 | jq -er '.uid.value')
  party=$(call POST /demographic/person application/json "$PERSON" 201 | jq -er '.uid.value')
  SEED=(
    "AUTH=$AUTH"
    "READ_EHR=$read_ehr"
    "WRITE_EHR=$write_ehr"
    "READ_COMPOSITION=$uid"
    "READ_PARTY=$party"
    "READ_PARTY_VO=${party%%::*}"
  )
}

# The AQL body of the lab-results query (Cadasto/FerroHEALTH-backlog#25),
# pinned to the read EHR, with or without its ORDER BY.
aql() { # <read-ehr> <ordered: true|false>
  local q="SELECT o/data[at0001]/events[at0002]/time/value, a/items[at0001]/value FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.laboratory_test_result.v1] CONTAINS CLUSTER a[openEHR-EHR-CLUSTER.laboratory_test_analyte.v1] WHERE e/ehr_id/value = \$ehr"
  [[ "$2" == true ]] && q+=" ORDER BY o/data[at0001]/events[at0002]/time/value DESC"
  jq -cn --arg q "$q" --arg ehr "$1" '{q: $q, query_parameters: {ehr: $ehr}}'
}

# render <plan-in> <plan-out>: every @NAME@ replaced; a placeholder left over
# is refused.
render() {
  local plan entry name read_ehr=""
  plan=$(<"$1")
  shopt -u patsub_replacement 2>/dev/null || true
  for entry in "${SEED[@]}"; do
    name="${entry%%=*}"
    [[ "$name" == READ_EHR ]] && read_ehr="${entry#*=}"
    plan="${plan//@${name}@/${entry#*=}}"
  done
  plan="${plan//@COMPOSITION@/$(<"$COMPOSITION")}"
  plan="${plan//@TEMPLATE@/$(<"$TEMPLATE")}"
  plan="${plan//@AQL_ORDER_BY@/$(aql "$read_ehr" true)}"
  plan="${plan//@AQL_UNORDERED@/$(aql "$read_ehr" false)}"
  if grep -qE '@[A-Z_]+@' <<<"$plan"; then
    die "the plan names a placeholder the seed does not fill: $(grep -oE '@[A-Z_]+@' <<<"$plan" | sort -u | paste -sd, -)"
  fi
  printf '%s\n' "$plan" >"$2"
}

prepare() { # <image> <pg-image>
  mkdir -p "$OUT"
  if [[ -n "${PERF_SERVER:-}" ]]; then
    [[ -n "${PERF_DATABASE:-}" && -n "${PERF_TARGET:-}" ]] || die "PERF_SERVER needs PERF_DATABASE and PERF_TARGET"
    SERVER_URL="$PERF_SERVER"
    DATABASE_URL="$PERF_DATABASE"
    read -r -a TARGET <<<"$PERF_TARGET"
  else
    [[ "$#" -eq 2 ]] || usage
    up "$1" "$2"
  fi
  seed
}

gate() { # <update: true|false> <image> <pg-image>
  local update="$1" bin
  shift
  bin=$(binary)
  prepare "$@"
  render "$ROOT/perf/plan.toml" "$OUT/plan.toml"
  local -a extra=()
  [[ "$update" == true ]] && extra=(--update)
  local status=0
  "$bin" run --plan "$OUT/plan.toml" --baseline "$ROOT/perf/baseline.toml" "${TARGET[@]}" \
    --base-url "$SERVER_URL" --database-url "$DATABASE_URL" --out-dir "$OUT" \
    --summary "$OUT/summary.md" ${extra[@]+"${extra[@]}"} || status=$?
  # TODO(#5095): commit the load baseline perf-baseline.yml writes per CPU
  # model; until then a run judges the idle limits and reports the load.
  if [[ "$status" == 2 && "$update" == false && ! -f "$ROOT/perf/baseline.toml" ]] \
    && unjudged_for_no_baseline "$OUT/summary.md"; then
    echo "perf gate: the idle limits held; the load phase is reported, not judged, until perf/baseline.toml lands (#5095)"
    return 0
  fi
  return "$status"
}

# Whether the run's only reason it could not be judged is the missing
# baseline: every check passed, every request was answered as the plan
# expects, and the generator and the window were sound.
unjudged_for_no_baseline() { # <summary.md>
  local reasons
  reasons=$(awk '/^### Why it could not be judged/ { on = 1; next } /^### / { on = 0 } on && /^- / { print }' "$1")
  [[ "$reasons" == "- there is no baseline; write one with --update" ]] || return 1
  ! grep -qE '^### Breaches|^### Requests answered other than expected|\| FAIL \|$' "$1"
}

# The plan with a short idle window and a short load, so the self-test spends
# about half a minute; the idle limits are the committed plan's own.
short_plan() { # <out>
  sed -E -e '/^\[idle\]/,/^\[/ s/^duration_secs = .*/duration_secs = 15/' \
    -e '/^\[load\]/,/^\[/ s/^(warmup_secs|duration_secs) = .*/\1 = 4/' \
    -e '/^\[load\]/,/^\[/ s/^rate = .*/rate = 5.0/' "$ROOT/perf/plan.toml" >"$1"
}

self_test() { # <image> <pg-image>
  local bin status=0 plan="$OUT/self-test-plan.toml"
  bin=$(binary)
  prepare "$@"
  short_plan "$OUT/self-test-plan.in"
  render "$OUT/self-test-plan.in" "$plan"
  # The busy loop FerroPIX #490 shipped: a reader polling a table with no
  # wait, each poll its own transaction. It ends at the next batch once the
  # stop file exists, and psql at the end of its input.
  local -a psql=(psql "$DATABASE_URL")
  [[ -z "${PERF_SERVER:-}" ]] && psql=(docker exec -i "$PG" psql -U postgres -d ferroehr)
  BUSY_STOP="$OUT/self-test.stop"
  rm -f "$BUSY_STOP"
  (
    while [[ ! -e "$BUSY_STOP" ]]; do
      printf 'SELECT count(*) FROM clinical.ehr;\n%.0s' {1..500}
    done | "${psql[@]}" -q -o /dev/null
  ) &
  BUSY_PID=$!
  "$bin" run --plan "$plan" --baseline "$OUT/self-test-baseline.toml" "${TARGET[@]}" \
    --base-url "$SERVER_URL" --database-url "$DATABASE_URL" --out-dir "$OUT/self-test" \
    --summary "$OUT/self-test.md" || status=$?
  stop_busy
  [[ "$status" == 1 ]] || die "self-test: an idle busy-loop exited $status, want 1 (a breach)"
  grep -qE '^\| idle\.transactions_per_second \|.*\| FAIL \|$' "$OUT/self-test.md" \
    || die "self-test: the breach does not name idle.transactions_per_second"
  echo "perf gate: self-test OK (an idle busy-loop fails idle.transactions_per_second)"
}

[[ "$#" -ge 1 ]] || usage
command="$1"
shift
case "$command" in
pin) pin ;;
install) install ;;
run | update | self-test)
  trap teardown EXIT
  case "$command" in
  run) gate false "$@" ;;
  update) gate true "$@" ;;
  self-test) self_test "$@" ;;
  esac
  ;;
*) usage ;;
esac
