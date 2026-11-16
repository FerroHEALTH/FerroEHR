#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The performance gate (#5081, #5326): FerroEHR and its PostgreSQL, idle and
# then under the committed request set, measured by ferrohealth-perf. `ab`
# judges the head of a change against its base, both measured side by side on
# one runner in one job (ferrohealth-perf's A/B mode), each arm the image of
# its own tree under its own tree's gate configuration, templates, SCIM
# client and seed data (the family rule). It fails when the head works while
# idle (CPU, or transactions in its database), or when the head is slower,
# heavier or busier under load than its base by more than the plan's
# tolerance. The pull-request gate runs it against main and the release lane
# against the previous published release, so no baseline is stored (#5095).
# No openEHR spec governs this: our own design.
#
# The server runs in a container of the image under test with
# perf/ferroehr.toml as its configuration, its database in a container of the
# FerroEHR PostgreSQL image, both on a network of their own; ferrohealth-perf
# reads the server's cgroup and counts the transactions of its database.
# The server stores its default demographic definitions at boot (a base tree
# that predates that has them loaded before serve), and once it is ready the
# Basic user is bound to a PERSON through SCIM (scripts/scim-provision.sh, which grants
# the client scim_provision through the server first, and the PERSON its
# scopes and role after). The seed writes one EHR the reads use and
# never changes, one EHR the commits write into, the lab-results template with
# a result, and a PERSON, and the plan is rendered with their ids
# (perf/plan.toml, `@NAME@`). It then writes the population the population
# query reads, PERF_POPULATION more EHRs through the arm's own server, so the
# query serves its whole LIMIT from a populated store and writes one
# access-log entry per EHR it served. After the run the gate counts those
# entries in each arm's store and adds them to the summary, so the report
# shows the per-EHR entry cost beside the base's (#5564).
#
# An arm whose tree configures the clinician Basic user runs with the access
# path in force (#5590, scripts/perf/access-seed.sh): an access policy that
# requires a care relationship, the clinician a member of the care team with
# a coded role an access-policy rule lets read HIV data, every seeded EHR's
# subject under that team's care with five
# directives on the EHR, and the clinical groups sent as the clinician. Its
# access log seals every append in a checkpoint signed with an OpenPGP key the
# gate generates for the run (PERF_SIGNING_KEY), mounted where the tree's
# configuration names it.
# A base that accepts this tree's configuration runs under it as well, so the
# two arms differ in the code alone (base_config_tree).
#
# Usage:
#   scripts/perf/gate.sh install                     the pinned ferrohealth-perf into target/tools/perf
#   scripts/perf/gate.sh ab <image> <pg-image> <base-image> <base-pg-image> [<base-rev>]
#       the head images against the base images; the base's tree is
#       <base-rev>, or the commit the base image names
#       (org.opencontainers.image.revision), taken from this clone's history
#       with `git archive`
#   scripts/perf/gate.sh self-test <image> <pg-image>
#       an idle busy-loop against the database must fail the idle phase,
#       naming idle.transactions_per_second
#   scripts/perf/gate.sh replicas <image> <pg-image> [<instances>...]
#       the reload load of 1, 2 and 3 instances (or the counts given) on one
#       PostgreSQL, one JSON line per count and change in $PERF_OUT/replicas.jsonl
#       (scripts/perf/replicas.sh)
#   scripts/perf/gate.sh pin                         print the pinned version
#
# Environment:
#   PERF_OUT      the report directory (default target/perf)
#   PERF_POPULATION  the EHRs the seed adds for the population query
#                 (default 2000; the release lane seeds 20000)
#   PERF_SIGNING_KEY the algorithm of the generated signing key, ed25519
#                 (default) or rsa3072
#   PERF_BIN      a ferrohealth-perf binary (default target/tools/perf/bin/ferrohealth-perf)
#   PERF_SERVER   with PERF_DATABASE and PERF_TARGET, measure a server already
#                 running (its base URL), whose database URL and ferrohealth-perf
#                 target (`--pid N` or `--cgroup DIR`) they name, and start no
#                 container: a developer machine without docker; PERF_FERROEHR
#                 is then the command that runs its binary against its
#                 database, for the access seed's grants
#
# Exit codes: 0 the run passed, 1 a breach, 2 the run could not be judged or a
# usage or harness error, as ferrohealth-perf answers.
set -euo pipefail
# A failure inside a command substitution, such as a seed call, stops the gate.
shopt -s inherit_errexit

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
readonly ROOT
readonly PREFIX=ferroehr-perf
readonly NETWORK="${PREFIX}-net"
# The head's containers; the base's carry the suffix -base.
readonly PG="${PREFIX}-postgres"
readonly SERVER="${PREFIX}-server"
# The throwaway database's roles, which the image's init script creates.
readonly PG_SECRET=ferroehr
readonly PG_SUPER_SECRET=postgres
readonly PG_PORT=55432
readonly HTTP_PORT=58080
readonly BASE_PG_PORT=55433
readonly BASE_HTTP_PORT=58081
readonly BASE="/ferroehr/rest/openehr/v1"
readonly AUTH="Basic ZmVycm9laHI6ZmVycm9laHI="
# Paths in an arm's tree: the head's is this checkout, the base's the
# archive of its commit under target/perf-base.
readonly TEMPLATE_PATH=corpus/templates/ckm/generic-lab-test-result.opt
readonly COMPOSITION_PATH=corpus/templates/ckm/generic-lab-test-result.example.json
readonly PERSON_PATH=corpus/fixtures/demographic/person.v1.json
readonly TEMPLATE="$ROOT/$TEMPLATE_PATH"
readonly COMPOSITION="$ROOT/$COMPOSITION_PATH"
readonly BASE_TREE="$ROOT/target/perf-base"
# The population query of the plan (`@AQL_POPULATION@`): it names no EHR, so
# the access log writes one entry for each EHR it serves.
readonly POPULATION_AQL="SELECT e/ehr_id/value FROM EHR e LIMIT 20"
OUT="${PERF_OUT:-$ROOT/target/perf}"
readonly OUT
# shellcheck source=scripts/perf/access-seed.sh
source "$ROOT/scripts/perf/access-seed.sh"
# shellcheck source=scripts/perf/replicas.sh
source "$ROOT/scripts/perf/replicas.sh"

usage() {
  sed -n '/^# Usage:$/,/^#$/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
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
    docker logs --tail 40 "$SERVER-base" >"$OUT/base-server.log" 2>&1 || true
  fi
  docker rm -f "$SERVER" "$PG" "$SERVER-base" "$PG-base" >/dev/null 2>&1 || true
  docker network rm "$NETWORK" >/dev/null 2>&1 || true
}

# signing_key: the OpenPGP key the access log seals each append with, generated
# for this run into $OUT/signing (a committed key would be a defect of its
# own): PERF_SIGNING_KEY's algorithm, its passphrase in a file beside it.
signing_key() {
  local algorithm="${PERF_SIGNING_KEY:-ed25519}" home fpr
  [[ "$algorithm" == ed25519 || "$algorithm" == rsa3072 ]] \
    || die "PERF_SIGNING_KEY must be ed25519 or rsa3072, got '$algorithm'"
  command -v gpg >/dev/null || die "gpg is needed to generate the signing key"
  mkdir -p "$OUT/signing"
  # gpg-agent's socket path is bounded, so its home is short-lived and short.
  home=$(mktemp -d)
  GNUPGHOME="$home" gpg --batch --pinentry-mode loopback --passphrase perf \
    --quick-generate-key "FerroEHR perf gate <perf@ferroehr.invalid>" "$algorithm" sign never \
    >/dev/null 2>&1 || die "gpg would not generate an $algorithm key"
  fpr=$(GNUPGHOME="$home" gpg --batch --list-secret-keys --with-colons | awk -F: '/^fpr:/ { print $10; exit }')
  GNUPGHOME="$home" gpg --batch --pinentry-mode loopback --passphrase perf --armor \
    --export-secret-keys "$fpr" >"$OUT/signing/signing.asc"
  printf perf >"$OUT/signing/passphrase"
  GNUPGHOME="$home" gpgconf --kill gpg-agent 2>/dev/null || true
  rm -rf "$home"
  # The identifier-protection root key, whose clinical lookup subkey digests
  # the subject references of the care projection (#5699), generated per run.
  od -An -tx1 -N32 /dev/urandom | tr -d ' \n' >"$OUT/signing/identifier-key"
  # The container runs as the distroless nonroot uid.
  chmod 644 "$OUT/signing/signing.asc" "$OUT/signing/passphrase" "$OUT/signing/identifier-key"
  echo "$algorithm" >"$OUT/signing/algorithm"
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

# up <image> <pg-image> [<tree> <suffix> <http-port> <pg-port>]: one arm's
# PostgreSQL and server, configured, provisioned and seeded from <tree> (this
# checkout by default), its containers named with <suffix>. Sets SERVER_URL,
# DATABASE_URL, TARGET and SEED for that arm.
up() {
  local image="$1" pg_image="$2" tree="${3:-$ROOT}" suffix="${4:-}"
  local http_port="${5:-$HTTP_PORT}" pg_port="${6:-$PG_PORT}"
  local pg="$PG$suffix" server="$SERVER$suffix"
  docker network inspect "$NETWORK" >/dev/null 2>&1 || docker network create "$NETWORK" >/dev/null
  # The cluster lives in memory (#5866): every request commits before it
  # answers (#5789), and a hosted runner's WAL fsync ranges from 0.08 to
  # 1.4 ms a call between runs, which tails every route on both arms. A commit
  # still flushes; wal_summary reports each arm's WAL and flushes per request.
  docker run -d --name "$pg" --network "$NETWORK" -p "127.0.0.1:${pg_port}:5432" \
    --tmpfs /var/lib/postgresql:rw,mode=0755 \
    -e "POSTGRES_PASSWORD=$PG_SUPER_SECRET" -e PG_INIT_USER=ferroehr -e "PG_INIT_PASSWORD=$PG_SECRET" \
    -e PG_INIT_DB=ferroehr "$pg_image" -c shared_preload_libraries=pg_stat_statements \
    -c pg_stat_statements.track=all -c pg_stat_statements.track_planning=on >/dev/null
  wait_for "PostgreSQL$suffix" 60 docker exec "$pg" pg_isready -h 127.0.0.1 -U ferroehr -d ferroehr
  docker exec "$pg" psql -U postgres -d ferroehr -Atq \
    -c "CREATE EXTENSION IF NOT EXISTS pg_stat_statements" >/dev/null
  STARTED=1
  local -a process=(--network "$NETWORK" --read-only --tmpfs /tmp --cap-drop ALL
    --security-opt no-new-privileges:true
    -e FERROEHR_CONFIG=/etc/ferroehr/ferroehr.toml
    -e "FERROEHR__DATABASE__URL=postgres://ferroehr:${PG_SECRET}@${pg}:5432/ferroehr"
    -e FERROEHR__USAGE_REPORT__ENABLED=false
    -v "$tree/perf/ferroehr.toml:/etc/ferroehr/ferroehr.toml:ro"
    -v "$tree/docs/conformance/party/scim/jwks.json:/etc/ferroehr/scim-jwks.json:ro"
    -v "$tree/deploy/definitions/professional:/etc/ferroehr/definitions:ro"
    -v "$OUT/signing/signing.asc:/etc/ferroehr/signing.asc:ro"
    -v "$OUT/signing/passphrase:/etc/ferroehr/signing-passphrase:ro"
    -v "$OUT/signing/identifier-key:/etc/ferroehr/identifier-key:ro")
  # A base tree whose configuration still names the professional templates
  # predates the default party mapping, and its server stores no default
  # definitions, so they are stored before serve; a newer server stores them
  # at boot.
  if grep -q '^template_id' "$tree/perf/ferroehr.toml"; then
    docker run --rm "${process[@]}" "$image" definitions load \
      /etc/ferroehr/definitions/openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0.adls \
      /etc/ferroehr/definitions/openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1.0.0.adls >/dev/null
  fi
  docker run -d --name "$server" -p "127.0.0.1:${http_port}:8080" "${process[@]}" \
    "$image" >/dev/null
  SERVER_URL="http://127.0.0.1:${http_port}"
  DATABASE_URL="postgres://postgres:${PG_SUPER_SECRET}@127.0.0.1:${pg_port}/ferroehr"
  TARGET=(--container "$(docker inspect -f '{{.Id}}' "$server")")
  wait_for "FerroEHR$suffix" 120 curl -fsS "$SERVER_URL/health/readiness"
  # A tree whose SCIM clients are grants in the CDR grants its client through
  # the server first; an older tree declares it in its configuration.
  if grep -q 'binding-issuer' "$tree/scripts/scim-provision.sh"; then
    SCIM_GRANT="docker exec $server /usr/local/bin/ferroehr" \
      bash "$tree/scripts/scim-provision.sh" "$SERVER_URL" basic-users urn:ferroehr:basic-user \
      ferroehr >/dev/null
  else
    bash "$tree/scripts/scim-provision.sh" "$SERVER_URL" basic-users ferroehr >/dev/null
  fi
  if access_arm "$tree"; then
    ACCESS_IDS="$OUT/scim-ids$suffix"
    : >"$ACCESS_IDS"
    SCIM_IDS="$ACCESS_IDS" SCIM_GRANT="docker exec $server /usr/local/bin/ferroehr" \
      bash "$tree/scripts/scim-provision.sh" "$SERVER_URL" basic-users urn:ferroehr:basic-user \
      --roles USER clinician >/dev/null
    ACCESS_FERROEHR=(docker exec "$server" /usr/local/bin/ferroehr)
  fi
  seed "$tree"
}

# call <method> <path> <content-type> <body-file|-> <want-status>: the
# response body on stdout. A <path> under the openEHR base is given below it;
# one under /ferroehr/ is given whole. Each process answers into a file of its
# own, so the access seed's calls run side by side.
call() {
  local method="$1" path="$2" type="$3" body="$4" want="$5" status
  local answer="$OUT/seed.$BASHPID.body"
  local -a data=()
  [[ "$body" != - ]] && data=(--data-binary "@$body" -H "content-type: $type")
  [[ "$path" == /ferroehr/* ]] || path="$BASE$path"
  status=$(curl -sS -o "$answer" -w '%{http_code}' -X "$method" \
    -H "authorization: $AUTH" -H "accept: application/json" -H "prefer: return=representation" \
    ${data[@]+"${data[@]}"} "$SERVER_URL$path")
  [[ "$status" == "$want" ]] || die "seed: $method $path answered $status, want $want: $(head -c 400 "$answer")"
  cat "$answer"
  rm -f "$answer"
}

# seed <tree>: the arm's records, from its tree's corpus; with the access
# path in force when the tree runs the clinician.
seed() {
  local tree="$1" read_ehr write_ehr uid party clinical="$AUTH"
  call POST /definition/template/adl1.4 application/xml "$tree/$TEMPLATE_PATH" 201 >/dev/null
  if access_arm "$tree"; then
    access_seed
    read_ehr=$(access_patient_ehr "$ACCESS_TEAM")
    write_ehr=$(access_patient_ehr "$ACCESS_TEAM")
    clinical="$CLINICIAN_AUTH"
  else
    read_ehr=$(call POST /ehr "" - 201 | jq -er '.ehr_id.value')
    write_ehr=$(call POST /ehr "" - 201 | jq -er '.ehr_id.value')
  fi
  uid=$(call POST "/ehr/$read_ehr/composition" application/json "$tree/$COMPOSITION_PATH" 201 | jq -er '.uid.value')
  party=$(call POST /demographic/person application/json "$tree/$PERSON_PATH" 201 | jq -er '.uid.value')
  if access_arm "$tree"; then
    access_population "$(population_count)"
  else
    populate
  fi
  SEED=(
    "CLINICAL_AUTH=$clinical"
    "READ_EHR=$read_ehr"
    "WRITE_EHR=$write_ehr"
    "READ_COMPOSITION=$uid"
    "READ_PARTY=$party"
    "READ_PARTY_VO=${party%%::*}"
  )
}

# populate: PERF_POPULATION more EHRs, created eight at a time through the
# arm's own server.
population_count() {
  local count="${PERF_POPULATION:-2000}"
  [[ "$count" =~ ^[0-9]+$ ]] || die "PERF_POPULATION must be a whole number, got '$count'"
  echo "$count"
}

populate() {
  local count created
  count=$(population_count)
  ((count > 0)) || return 0
  seq "$count" | xargs -P 8 -I{} curl -sS -o /dev/null -w '%{http_code}\n' -X POST \
    -H "authorization: $AUTH" -H "prefer: return=minimal" "$SERVER_URL$BASE/ehr" \
    >"$OUT/populate.status"
  created=$(grep -c '^201$' "$OUT/populate.status" || true)
  [[ "$created" == "$count" ]] || die "seed: $created of $count population EHRs were created"
}

# population_entries <pg-container>: the access-log entries the population
# query wrote in that arm's store, as "<statements> <per-EHR entries>", or
# "none" for a store with no access log. Entries name a query by the SHA-256
# of its text, never the text.
population_entries() {
  local -a psql=(docker exec "$1" psql -U postgres -d ferroehr -Atq -c)
  local digest
  digest=$(printf '%s' "$POPULATION_AQL" | { sha256sum 2>/dev/null || shasum -a 256; } | awk '{ print $1 }')
  if [[ "$("${psql[@]}" "SELECT to_regclass('audit.access_log_record') IS NOT NULL")" != t ]]; then
    echo none
    return 0
  fi
  "${psql[@]}" "SELECT count(*) FILTER (WHERE ehr_id IS NULL) || ' ' || \
    count(*) FILTER (WHERE ehr_id IS NOT NULL) FROM audit.access_log_record \
    WHERE content LIKE '%${digest}%'"
}

# population_summary <summary>: each arm's population entries appended to the
# summary, with the per-EHR entries one statement wrote.
population_summary() {
  local arm container counts statements per_ehr
  {
    echo
    echo "### The population query's access-log entries"
    echo
    echo "\`$POPULATION_AQL\` over $((${PERF_POPULATION:-2000} + 2)) EHRs and those the load creates."
    echo
    echo "| arm | statements | per-EHR entries | per statement |"
    echo "|---|---:|---:|---:|"
    for arm in head base; do
      container="$PG"
      [[ "$arm" == base ]] && container="$PG-base"
      counts=$(population_entries "$container" 2>/dev/null) || counts="unreadable"
      if [[ "$counts" =~ ^([0-9]+)\ ([0-9]+)$ ]]; then
        statements="${BASH_REMATCH[1]}"
        per_ehr="${BASH_REMATCH[2]}"
        echo "| $arm | $statements | $per_ehr | $(awk -v e="$per_ehr" -v s="$statements" 'BEGIN { if (s > 0) printf "%.1f", e / s; else print "-" }') |"
      else
        echo "| $arm | $counts | | |"
      fi
    done
  } >>"$1"
}

# statements_summary <summary>: each arm's statements by PostgreSQL time over
# the measured run, its planning counted beside its execution
# (pg_stat_statements.track_planning): the AQL is custom-planned per call, so
# its execution alone leaves out about half of its cost at a small answer
# (#5746).
statements_summary() {
  local arm container
  {
    echo
    echo "### Statements by PostgreSQL time, planning included"
    for arm in head base; do
      container="$PG"
      [[ "$arm" == base ]] && container="$PG-base"
      echo
      echo "#### $arm"
      echo
      echo "| calls | plan ms (mean) | execution ms (mean) | rows per call | buffer hits per call | statement |"
      echo "|---:|---:|---:|---:|---:|---|"
      docker exec "$container" psql -U postgres -d ferroehr -Atq -F '|' -c "SELECT calls, \
        round(mean_plan_time::numeric, 3), round(mean_exec_time::numeric, 3), \
        round(rows::numeric / calls, 1), round(shared_blks_hit::numeric / calls, 1), \
        replace(left(regexp_replace(query, '\s+', ' ', 'g'), 120), '|', '/') \
        FROM pg_stat_statements WHERE calls > 0 AND query NOT ILIKE '%pg_stat%' \
        ORDER BY total_plan_time + total_exec_time DESC LIMIT 10" 2>/dev/null \
        | awk -F'|' '{ printf "| %s | %s | %s | %s | %s | `%s` |\n", $1, $2, $3, $4, $5, $6 }' \
        || echo "| unreadable | | | | | |"
    done
  } >>"$1"
}

# statements_reset: both arms' statement counters zeroed, so the summary
# covers the measured run and not the seed.
statements_reset() {
  local container
  for container in "$PG" "$PG-base"; do
    docker exec "$container" psql -U postgres -d ferroehr -Atq \
      -c "SELECT pg_stat_statements_reset(), pg_stat_reset_shared('wal'), pg_stat_reset_shared('io')" \
      >/dev/null
  done
}

# wal_summary <summary> <report>: each arm's WAL bytes and WAL flushes over
# the run, and per request the arm was sent (#5866), so a change in what a
# request writes durably shows although the cluster lives in memory.
wal_summary() {
  local arm container counts sent bytes flushes
  {
    echo
    echo "### Write-ahead log"
    echo
    echo "| arm | requests | WAL bytes | per request | WAL flushes | per request |"
    echo "|---|---:|---:|---:|---:|---:|"
    for arm in head base; do
      container="$PG"
      [[ "$arm" == base ]] && container="$PG-base"
      sent=$(jq --arg arm "$arm" '[.pairs[][$arm].sent] | add // 0' "$2" 2>/dev/null) || sent=0
      counts=$(docker exec "$container" psql -U postgres -d ferroehr -Atq -F ' ' -c \
        "SELECT (SELECT wal_bytes FROM pg_stat_wal), (SELECT sum(fsyncs) FROM pg_stat_io WHERE object = 'wal')" \
        2>/dev/null) || counts="unreadable"
      if [[ "$sent" =~ ^[1-9][0-9]*$ && "$counts" =~ ^([0-9]+)\ ([0-9]+)$ ]]; then
        bytes="${BASH_REMATCH[1]}"
        flushes="${BASH_REMATCH[2]}"
        echo "| $arm | $sent | $bytes | $((bytes / sent)) | $flushes | $(awk -v f="$flushes" -v s="$sent" 'BEGIN { printf "%.2f", f / s }') |"
      else
        echo "| $arm | $sent | $counts | | | |"
      fi
    done
  } >>"$1"
}

# access_log_storage <pg-container>: the access log's entries, its
# checkpoints and the bytes its record, shared-part and checkpoint tables hold
# with their indexes and TOAST, as "<entries> <checkpoints> <bytes>", or "none" for a
# store with no access log.
access_log_storage() {
  local -a psql=(docker exec "$1" psql -U postgres -d ferroehr -Atq -c)
  if [[ "$("${psql[@]}" "SELECT to_regclass('audit.access_log_record') IS NOT NULL")" != t ]]; then
    echo none
    return 0
  fi
  "${psql[@]}" "SELECT (SELECT count(*) FROM audit.access_log_record) || ' ' || \
    (SELECT count(*) FROM audit.access_log_checkpoint) || ' ' || \
    (pg_total_relation_size('audit.access_log_record') \
     + pg_total_relation_size('audit.access_log_checkpoint') \
     + coalesce(pg_total_relation_size(to_regclass('audit.access_log_shared')), 0))"
}

# storage_summary <summary>: each arm's access-log storage appended to the
# summary, with the bytes one entry takes (#5590).
storage_summary() {
  local arm container counts
  {
    echo
    echo "### The access log's storage"
    echo
    echo "| arm | entries | checkpoints | stored bytes | per entry |"
    echo "|---|---:|---:|---:|---:|"
    for arm in head base; do
      container="$PG"
      [[ "$arm" == base ]] && container="$PG-base"
      counts=$(access_log_storage "$container" 2>/dev/null) || counts="unreadable"
      if [[ "$counts" =~ ^([0-9]+)\ ([0-9]+)\ ([0-9]+)$ ]]; then
        echo "| $arm | ${BASH_REMATCH[1]} | ${BASH_REMATCH[2]} | ${BASH_REMATCH[3]} | $(awk -v b="${BASH_REMATCH[3]}" -v e="${BASH_REMATCH[1]}" 'BEGIN { if (e > 0) printf "%.0f", b / e; else print "-" }') |"
      else
        echo "| $arm | $counts | | | |"
      fi
    done
  } >>"$1"
}

# The AQL body of the lab-results query (Cadasto/FerroHEALTH-backlog#25),
# pinned to the read EHR, with or without its ORDER BY.
aql() { # <read-ehr> <ordered: true|false>
  local q="SELECT o/data[at0001]/events[at0002]/time/value, a/items[at0001]/value FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.laboratory_test_result.v1] CONTAINS CLUSTER a[openEHR-EHR-CLUSTER.laboratory_test_analyte.v1] WHERE e/ehr_id/value = \$ehr"
  [[ "$2" == true ]] && q+=" ORDER BY o/data[at0001]/events[at0002]/time/value DESC"
  jq -cn --arg q "$q" --arg ehr "$1" '{q: $q, query_parameters: {ehr: $ehr}}'
}

# render <plan-in> <plan-out> [per-arm]: every @NAME@ replaced. With
# per-arm, the names an arm's seed fills (READ_EHR and the others) are left
# for ferrohealth-perf, which fills each arm's copy with that arm's values;
# the request bodies are this checkout's. A placeholder left over that the
# seed does not name is refused.
render() {
  local plan entry name read_ehr="@READ_EHR@" leftover
  plan=$(<"$1")
  shopt -u patsub_replacement 2>/dev/null || true
  if [[ "${3:-}" != per-arm ]]; then
    for entry in "${SEED[@]}"; do
      name="${entry%%=*}"
      [[ "$name" == READ_EHR ]] && read_ehr="${entry#*=}"
      plan="${plan//@${name}@/${entry#*=}}"
    done
  fi
  plan="${plan//@AUTH@/$AUTH}"
  plan="${plan//@COMPOSITION@/$(<"$COMPOSITION")}"
  plan="${plan//@TEMPLATE@/$(<"$TEMPLATE")}"
  plan="${plan//@AQL_ORDER_BY@/$(aql "$read_ehr" true)}"
  plan="${plan//@AQL_UNORDERED@/$(aql "$read_ehr" false)}"
  plan="${plan//@AQL_POPULATION@/$(jq -cn --arg q "$POPULATION_AQL" '{q: $q}')}"
  local -a left=()
  # A fully rendered plan holds no placeholder, so grep finding none is no error.
  while IFS= read -r name; do
    [[ -n "$name" ]] || continue
    [[ "${3:-}" == per-arm ]] && grep -q "^${name//@/}=" <<<"$(printf '%s\n' "${SEED[@]}")" && continue
    left+=("$name")
  done < <({ grep -oE '@[A-Z_]+@' <<<"$plan" || true; } | sort -u)
  leftover=$(IFS=,; echo "${left[*]+"${left[*]}"}")
  [[ -z "$leftover" ]] || die "the plan names a placeholder the seed does not fill: $leftover"
  printf '%s\n' "$plan" >"$2"
}

prepare() { # <image> <pg-image>
  mkdir -p "$OUT"
  if [[ -n "${PERF_SERVER:-}" ]]; then
    [[ -n "${PERF_DATABASE:-}" && -n "${PERF_TARGET:-}" ]] || die "PERF_SERVER needs PERF_DATABASE and PERF_TARGET"
    SERVER_URL="$PERF_SERVER"
    DATABASE_URL="$PERF_DATABASE"
    read -r -a TARGET <<<"$PERF_TARGET"
    if access_arm "$ROOT"; then
      [[ -n "${PERF_FERROEHR:-}" ]] || die "PERF_SERVER needs PERF_FERROEHR for the access seed"
      read -r -a ACCESS_FERROEHR <<<"$PERF_FERROEHR"
      ACCESS_IDS="$OUT/scim-ids"
      : >"$ACCESS_IDS"
      SCIM_IDS="$ACCESS_IDS" SCIM_GRANT="$PERF_FERROEHR" bash "$ROOT/scripts/scim-provision.sh" \
        "$SERVER_URL" basic-users urn:ferroehr:basic-user --roles USER clinician >/dev/null
    fi
    seed "$ROOT"
  else
    [[ "$#" -eq 2 ]] || usage
    teardown
    signing_key
    up "$1" "$2"
  fi
}

# base_tree <base-image> [<rev>]: the source tree of <rev>, or of the commit
# the base image names (org.opencontainers.image.revision), archived from
# this clone into BASE_TREE, so the base arm runs under its own tree's
# configuration.
base_tree() {
  local rev="${2:-}"
  if [[ -z "$rev" ]]; then
    rev=$(docker inspect -f '{{ index .Config.Labels "org.opencontainers.image.revision" }}' "$1") \
      || die "the base image $1 cannot be inspected"
  fi
  [[ "$rev" =~ ^[0-9a-f]{40}$ ]] || die "the base image $1 names no revision"
  git -C "$ROOT" cat-file -e "${rev}^{commit}" 2>/dev/null \
    || die "this clone does not hold $rev, the base image's commit: fetch the full history"
  rm -rf "$BASE_TREE"
  mkdir -p "$BASE_TREE"
  git -C "$ROOT" archive "$rev" | tar -x -C "$BASE_TREE"
  [[ -f "$BASE_TREE/perf/ferroehr.toml" ]] \
    || die "the base commit $rev predates the gate's configuration (perf/ferroehr.toml)"
  printf '%s\n' "$rev" >"$OUT/base-rev"
  echo "perf gate: the base is $rev"
}

# base_config_tree <base-image>: the tree the base arm runs under. When the
# base binary accepts this tree's gate configuration (`ferroehr config check`),
# both arms run one configuration, signing key, access seed and request
# credentials, so the gate measures the code alone (#5590); an older base
# that refuses it runs under its own tree's, as BASE_TREE holds it.
base_config_tree() {
  if docker run --rm -e FERROEHR_CONFIG=/etc/ferroehr/ferroehr.toml \
    -e FERROEHR__DATABASE__URL=postgres://check:check@localhost/check \
    -v "$ROOT/perf/ferroehr.toml:/etc/ferroehr/ferroehr.toml:ro" \
    -v "$ROOT/docs/conformance/party/scim/jwks.json:/etc/ferroehr/scim-jwks.json:ro" \
    -v "$OUT/signing/signing.asc:/etc/ferroehr/signing.asc:ro" \
    -v "$OUT/signing/passphrase:/etc/ferroehr/signing-passphrase:ro" \
    -v "$OUT/signing/identifier-key:/etc/ferroehr/identifier-key:ro" \
    "$1" config check >/dev/null 2>&1; then
    echo "perf gate: the base runs under this tree's configuration" >&2
    echo "$ROOT"
  else
    echo "perf gate: the base refuses this tree's configuration and runs under its own" >&2
    echo "$BASE_TREE"
  fi
}

# ab <image> <pg-image> <base-image> <base-pg-image>: the head and the base
# side by side, each seeded and provisioned from its own tree, judged by
# `ferrohealth-perf ab` over this checkout's plan with each arm's ids.
ab() {
  [[ "$#" -eq 4 || "$#" -eq 5 ]] || usage
  local bin status=0 entry
  bin=$(binary)
  mkdir -p "$OUT"
  teardown
  base_tree "$3" "${5:-}"
  signing_key
  up "$1" "$2"
  local head_url="$SERVER_URL" head_db="$DATABASE_URL" head_container="${TARGET[1]}"
  local -a vars=()
  for entry in "${SEED[@]}"; do vars+=(--head-var "$entry"); done
  up "$3" "$4" "$(base_config_tree "$3")" -base "$BASE_HTTP_PORT" "$BASE_PG_PORT"
  for entry in "${SEED[@]}"; do vars+=(--base-var "$entry"); done
  render "$ROOT/perf/plan.toml" "$OUT/plan.toml" per-arm
  statements_reset
  "$bin" ab --plan "$OUT/plan.toml" \
    --head-url "$head_url" --head-container "$head_container" --head-database-url "$head_db" \
    --base-url "$SERVER_URL" --base-container "${TARGET[1]}" --base-database-url "$DATABASE_URL" \
    "${vars[@]}" --out-dir "$OUT" --summary "$OUT/summary.md" || status=$?
  population_summary "$OUT/summary.md"
  storage_summary "$OUT/summary.md"
  statements_summary "$OUT/summary.md"
  wal_summary "$OUT/summary.md" "$OUT/latest.json"
  return "$status"
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

# Sourced by scripts/perf/capacity.sh for the seed and its helpers, the gate
# dispatches nothing.
[[ "${BASH_SOURCE[0]}" == "$0" ]] || return 0
[[ "$#" -ge 1 ]] || usage
command="$1"
shift
case "$command" in
pin) pin ;;
install) install ;;
ab | self-test)
  trap teardown EXIT
  case "$command" in
  ab) ab "$@" ;;
  self-test) self_test "$@" ;;
  esac
  ;;
replicas)
  trap replicas_teardown EXIT
  replicas "$@"
  ;;
*) usage ;;
esac
