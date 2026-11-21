#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The capacity test (#5722): how many requests per second FerroEHR sustains
# before it degrades. The perf gate (scripts/perf/gate.sh) drives a fixed 20
# rps to compare a head against its base; this steps the arrival rate up until
# the knee and records what the server, PostgreSQL and the generator spent at
# every step.
#
# The load is ferrohealth-perf's open loop at a constant arrival rate, the
# version docs/VERSIONS.md pins: every latency runs from the arrival's
# intended instant, so a stalled server raises the latency of every request
# it held back (no coordinated omission), and a run whose generator lags its
# schedule is refused rather than reported. Each step is one
# `ferrohealth-perf run` at one rate.
#
# The topology is one or more FerroEHR instances on one PostgreSQL 18, with
# Caddy in front when there is more than one (round robin, no sticky
# sessions). The seed is the perf gate's (scripts/perf/access-seed.sh): an
# ACCESS_POLICY with a controller rule, a care team with the clinician as
# member, CAPACITY_POPULATION patients under its care, each EHR with five
# directives, and an Ed25519 key signing the access log. On top of it the
# first CAPACITY_SPREAD population EHRs get one composition each, so the
# single-EHR groups read and write across that many EHRs, past the 4096
# EHRs the access settings cache holds. The commits go to the next
# CAPACITY_SPREAD population EHRs, so the reads' answers do not grow. The
# single-EHR AQL reads EHRs of its own, sized as a chart is (#5746): aql_ehr
# the next CAPACITY_SPREAD EHRs with CAPACITY_AQL_ROWS lab results each (a
# typical answer), and aql_chart the next CAPACITY_CHART_EHRS with
# CAPACITY_CHART_ROWS each (a whole chart).
#
# PostgreSQL counts each statement's planning beside its execution
# (pg_stat_statements.track_planning), and every step's statements.json names
# both: the AQL is custom-planned per call, so its execution alone is about
# half of what it costs PostgreSQL at a small answer.
#
# A step passes when every request was answered as expected, the generator
# kept its schedule and was never saturated, the measured throughput is at
# least 98 % of the offered rate, and every group's p99 lies within its SLO
# (CAPACITY_SLO_MS, CAPACITY_SLO_SLOW_MS for the population query, the
# access-log read and the export), as the generator measured it and as the
# server's own request log did. The knee is the highest rate that passes
# with every rate below it passing; a sweep stops at the first step that
# fails unless CAPACITY_CONTINUE asks it to run every rate.
#
# Each step also records what makes its numbers trustworthy or not (#5777):
# the host's and the containers' kernel's load, its clock source and the CPU
# of every other container over the step (host.json); the peak memory of each
# instance and of PostgreSQL (memory.json); and the store's bytes per
# composition version and per access-log entry (storage.json). A step run
# CAPACITY_REPEAT times reports each group's median p99 with its spread, and
# `compare` calls a difference between two arms only outside both spreads.
# `snapshot` keeps the seeded store and `restore` brings it back with the
# servers restarted, so every arm, or every step under CAPACITY_FRESH,
# starts from the same store; `arms` interleaves several configurations on
# it. No openEHR spec governs this: our own design.
#
# Usage:
#   scripts/perf/capacity.sh up <instances>
#       start PostgreSQL, <instances> servers and, above one, the balancer
#   scripts/perf/capacity.sh seed
#       provision and seed the running topology
#   scripts/perf/capacity.sh snapshot
#       keep the seeded store as the database ferroehr_seed
#   scripts/perf/capacity.sh restore
#       the store back to the snapshot, the servers restarted under the
#       current CAPACITY_SERVER_ENV
#   scripts/perf/capacity.sh sweep <group|mix> [<rate>...]
#       step <group> (or the mix) through the rates until one fails
#   scripts/perf/capacity.sh step <group|mix> <rate>
#       one rate
#   scripts/perf/capacity.sh arms <group|mix> <rate> <arm>=<env>...
#       CAPACITY_REPEAT rounds of one rate, each arm restored from the
#       snapshot and run under its FERROEHR__ settings (space-separated
#       KEY=VALUE pairs, empty for the configuration as it is), the arms'
#       order alternating from round to round
#   scripts/perf/capacity.sh compare <arm> <arm>
#       each step both arms ran, group by group, with the median p99 of each
#       and whether they differ outside both spreads
#   scripts/perf/capacity.sh soak <group|mix> <rate> <seconds>
#       hold one rate for <seconds>, sampling each instance's resident set,
#       and fail when it keeps rising after the warm-up and the settling
#   scripts/perf/capacity.sh profile <group|mix> <rate> <seconds>
#       hold one rate while an external sampler profiles the first instance
#   scripts/perf/capacity.sh summary
#       the Markdown tables of every step run so far
#   scripts/perf/capacity.sh generator
#       build the pinned ferrohealth-perf for Linux, for CAPACITY_GENERATOR=network
#   scripts/perf/capacity.sh down
#       stop what `up` started
#   scripts/perf/capacity.sh groups
#       the groups a sweep takes, and the mix
#
# Environment:
#   CAPACITY_OUT         the output directory (default target/capacity)
#   CAPACITY_TOPOLOGY    native (default) or docker
#   CAPACITY_FERROEHR    native: the ferroehr binary
#   CAPACITY_PG_BIN      native: the directory of PostgreSQL 18's binaries
#   CAPACITY_CADDY       native: the caddy binary, for more than one instance
#   CAPACITY_IMAGE       docker: the FerroEHR image
#   CAPACITY_PG_IMAGE    docker: the FerroEHR PostgreSQL image
#   CAPACITY_CADDY_IMAGE docker: the Caddy image, pinned by digest
#   CAPACITY_SERVER_CPUS docker: the cpuset of the servers and the balancer
#   CAPACITY_PG_CPUS     docker: the cpuset of PostgreSQL
#   CAPACITY_GENERATOR_CPUS
#                        Linux: the cores the generator runs on (taskset), so
#                        its load stays off the server's
#   CAPACITY_BUDGET      each instance's database.connection_budget (default
#                        96: three instances inside the max_connections of
#                        300 this harness gives PostgreSQL)
#   CAPACITY_POOL        each instance's database.max_connections (default
#                        unset: the server sizes its pools from its cores,
#                        within the budget)
#   CAPACITY_MAX_CONCURRENT
#                        each instance's server.max_concurrent_requests
#                        (default 256, the quickstart's): past it a request
#                        is answered 503, so an overloaded step fails on
#                        refusals instead of the server running out of memory
#   CAPACITY_SERVER_ENV  further FERROEHR__ settings every instance runs
#                        with, space-separated KEY=VALUE pairs, for a variant
#                        the summary must then name
#   CAPACITY_POPULATION  the patients the seed puts under care (default 20000)
#   CAPACITY_SPREAD      the EHRs the single-EHR groups spread over (default 500)
#   CAPACITY_AQL_ROWS    the lab results in each EHR aql_ehr reads (default 20)
#   CAPACITY_CHART_EHRS  the EHRs aql_chart reads (default 50)
#   CAPACITY_CHART_ROWS  the lab results in each EHR aql_chart reads
#                        (default 100)
#   CAPACITY_PRIME_WIDTH the requests of each route `prime` sends at once to an
#                        instance (default 32): at least the connections of
#                        its largest pool, so each one runs every route once
#   CAPACITY_WARMUP      seconds of warm-up per step (default 15)
#   CAPACITY_DURATION    measured seconds per step (default 60)
#   CAPACITY_SLO_MS      the p99 SLO of the single-EHR groups (default 100)
#   CAPACITY_SLO_SLOW_MS the p99 SLO of the population groups (default 500)
#   CAPACITY_RATES       the default rates of a sweep
#                        (default "50 100 200 500 1000 1500 2000 3000")
#   CAPACITY_REPEAT      the runs of each step (default 1); a step that
#                        reports a p99 a target is judged on runs at least 3
#   CAPACITY_CONTINUE    1: a sweep runs every rate past a failed step, and
#                        records each step's outcome (default 0)
#   CAPACITY_FRESH       1: each run of a step starts from the snapshot
#                        (`restore` before it), so no step reads a store an
#                        earlier one grew (default 0)
#   CAPACITY_ARM         the label of the configuration under test; its
#                        steps go under steps/<arm>/ (default none)
#   CAPACITY_SOAK_INTERVAL
#                        seconds between a soak's samples (default 10)
#   CAPACITY_SOAK_SETTLE seconds after the warm-up before the soak's fit
#                        starts, for the bounded caches to fill (default 600)
#   CAPACITY_SOAK_MAX_MIB_PER_HOUR
#                        the resident-set slope past which a soak fails
#                        (default 16)
#   CAPACITY_GENERATOR   host (default) or network: docker topology, the
#                        generator in a container on the servers' network,
#                        so no port forwarder sits in the latency it measures
#                        (`capacity.sh generator` builds it)
#   PERF_BIN             a ferrohealth-perf binary (default
#                        target/tools/perf/bin/ferrohealth-perf)
#
# Exit codes: 0 done, 1 a sweep's first step already failed (or a step
# failed), 2 a usage or harness error.
set -euo pipefail
shopt -s inherit_errexit

CAPACITY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
readonly CAPACITY_ROOT
CAP_OUT="${CAPACITY_OUT:-$CAPACITY_ROOT/target/capacity}"
readonly CAP_OUT
mkdir -p "$CAP_OUT"
CAP_OUT_ABS="$(cd "$CAP_OUT" && pwd -P)"
readonly CAP_OUT_ABS
# The gate's seed, its helpers and its constants (AUTH, BASE, the corpus
# paths, the access seed); gate.sh dispatches nothing when sourced.
export PERF_OUT="$CAP_OUT_ABS/seed"
export PERF_POPULATION="${CAPACITY_POPULATION:-20000}"
# shellcheck source=scripts/perf/gate.sh
source "$CAPACITY_ROOT/scripts/perf/gate.sh"

readonly TOPOLOGY="${CAPACITY_TOPOLOGY:-native}"
readonly POOL="${CAPACITY_POOL:-}"
readonly BUDGET="${CAPACITY_BUDGET:-96}"
readonly MAX_CONCURRENT="${CAPACITY_MAX_CONCURRENT:-256}"
readonly SPREAD="${CAPACITY_SPREAD:-500}"
readonly AQL_ROWS="${CAPACITY_AQL_ROWS:-20}"
readonly CHART_EHRS="${CAPACITY_CHART_EHRS:-50}"
readonly CHART_ROWS="${CAPACITY_CHART_ROWS:-100}"
readonly WARMUP="${CAPACITY_WARMUP:-15}"
readonly DURATION="${CAPACITY_DURATION:-60}"
readonly SLO_MS="${CAPACITY_SLO_MS:-100}"
readonly SLO_SLOW_MS="${CAPACITY_SLO_SLOW_MS:-500}"
readonly RATES="${CAPACITY_RATES:-50 100 200 500 1000 1500 2000 3000}"
readonly PRIME_WIDTH="${CAPACITY_PRIME_WIDTH:-32}"
readonly REPEAT="${CAPACITY_REPEAT:-1}"
[[ "$REPEAT" =~ ^[1-9][0-9]*$ ]] || { echo "capacity: CAPACITY_REPEAT must be a positive count" >&2; exit 2; }
readonly CONTINUE="${CAPACITY_CONTINUE:-0}"
readonly FRESH="${CAPACITY_FRESH:-0}"
readonly ARM="${CAPACITY_ARM:-}"
[[ "$ARM" =~ ^[A-Za-z0-9._-]*$ ]] || { echo "capacity: CAPACITY_ARM takes letters, digits, '.', '_' and '-'" >&2; exit 2; }
readonly GENERATOR="${CAPACITY_GENERATOR:-host}"
readonly SNAPSHOT_DB=ferroehr_seed
readonly SOAK_INTERVAL="${CAPACITY_SOAK_INTERVAL:-10}"
readonly SOAK_SETTLE="${CAPACITY_SOAK_SETTLE:-600}"
readonly SOAK_MAX_MIB_PER_HOUR="${CAPACITY_SOAK_MAX_MIB_PER_HOUR:-16}"
readonly CAP_PG_PORT=55450
readonly CAP_HTTP_PORT=58100
readonly CAP_LB_PORT=58099
readonly CAP_NET=ferroehr-capacity
readonly CAP_PG=ferroehr-capacity-postgres
readonly CAP_SERVER=ferroehr-capacity
readonly CAP_LB=ferroehr-capacity-lb
readonly CAP_GEN=ferroehr-capacity-generator
readonly STATE="$CAP_OUT_ABS/state"
# shellcheck source=scripts/perf/capacity-measure.sh
source "$CAPACITY_ROOT/scripts/perf/capacity-measure.sh"
# shellcheck source=scripts/perf/capacity-report.sh
source "$CAPACITY_ROOT/scripts/perf/capacity-report.sh"

# The groups a sweep takes one by one, each with its caller; the mix takes
# them at once, each as many times per round as its weight.
#   group               weight  caller
readonly GROUPS_TABLE="
ehr_read             3  clinician
composition_read     5  clinician
composition_commit   3  clinician
aql_ehr              3  clinician
aql_chart            1  clinician
aql_population       1  clinician
access_log_read      1  administrator
party_read           2  administrator
export               0  clinician
health               0  none
"

cap_die() {
  echo "capacity: $*" >&2
  exit 2
}

cap_usage() {
  sed -n '/^# Usage:$/,/^#$/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

state_get() { # <key>
  [[ -f "$STATE/$1" ]] || cap_die "no $1 recorded: run \`capacity.sh up\` and \`capacity.sh seed\` first"
  cat "$STATE/$1"
}

state_put() { # <key> <value>
  mkdir -p "$STATE"
  printf '%s\n' "$2" >"$STATE/$1"
}

# The configuration every instance runs: the gate's, with each path the
# topology mounts and the management surface the instruments read.
instance_config() { # <dir> <jwks> <scim-jwks> <key> <passphrase>
  sed -e "s#/opt/sandbox-posture/basic-users-jwks.json#$2#" \
    -e "s#/etc/ferroehr/scim-jwks.json#$3#" \
    -e "s#/etc/ferroehr/signing.asc#$4#" \
    -e "s#/etc/ferroehr/signing-passphrase#$5#" \
    "$CAPACITY_ROOT/perf/ferroehr.toml" >"$1/ferroehr.toml"
  cat >>"$1/ferroehr.toml" <<'EOF'

[management]
enabled = true

[management.endpoints]
prometheus = "admin_only"
EOF
}

# PostgreSQL's settings, the same for both topologies.
pg_settings() {
  cat <<'EOF'
max_connections = 300
shared_buffers = 4GB
effective_cache_size = 12GB
work_mem = 16MB
maintenance_work_mem = 512MB
max_wal_size = 8GB
checkpoint_timeout = 15min
wal_compression = lz4
shared_preload_libraries = 'pg_stat_statements'
pg_stat_statements.track = all
pg_stat_statements.track_planning = on
pg_stat_statements.max = 10000
track_io_timing = on
log_lock_waits = on
deadlock_timeout = 100ms
EOF
}

up_native() { # <instances>
  local count="$1" pg_bin="${CAPACITY_PG_BIN:?CAPACITY_PG_BIN names the bin directory of PostgreSQL 18}"
  local bin="${CAPACITY_FERROEHR:?CAPACITY_FERROEHR names the ferroehr binary}" i pgdata="$CAP_OUT_ABS/pg"
  [[ -x "$bin" ]] || cap_die "$bin is not executable"
  if [[ ! -d "$pgdata" ]]; then
    printf postgres >"$CAP_OUT_ABS/pg.pw"
    "$pg_bin/initdb" -D "$pgdata" -U postgres --pwfile="$CAP_OUT_ABS/pg.pw" -A scram-sha-256 \
      -E UTF8 --locale=C >/dev/null
    { echo "port = $CAP_PG_PORT"; echo "listen_addresses = '127.0.0.1'"; pg_settings; } \
      >>"$pgdata/postgresql.conf"
    "$pg_bin/pg_ctl" -D "$pgdata" -l "$CAP_OUT_ABS/pg.log" -w start >/dev/null
    PATH="$pg_bin:$PATH" PGHOST=127.0.0.1 PGPORT=$CAP_PG_PORT PGPASSWORD=postgres \
      POSTGRES_USER=postgres POSTGRES_DB=postgres PG_INIT_USER=ferroehr \
      PG_INIT_PASSWORD="$PG_SECRET" PG_INIT_DB=ferroehr \
      bash "$CAPACITY_ROOT/docker/postgres/initdb/10-ferroehr-init.sh" >/dev/null
  else
    "$pg_bin/pg_ctl" -D "$pgdata" -l "$CAP_OUT_ABS/pg.log" -w start >/dev/null
  fi
  state_put psql "$pg_bin/psql postgres://postgres:postgres@127.0.0.1:$CAP_PG_PORT/ferroehr"
  cap_sql "CREATE EXTENSION IF NOT EXISTS pg_stat_statements" >/dev/null
  state_put pg_cpu "children:$(head -1 "$pgdata/postmaster.pid")"
  # A key an earlier `up` generated is kept, so a restored store verifies.
  [[ -f "$PERF_OUT/signing/signing.asc" ]] || signing_key
  instance_config "$CAP_OUT_ABS" "$CAPACITY_ROOT/deploy/hosted/basic-users-jwks.json" \
    "$CAPACITY_ROOT/docs/conformance/party/scim/jwks.json" "$PERF_OUT/signing/signing.asc" \
    "$PERF_OUT/signing/passphrase"
  state_put instances "$count"
  servers_start_native
}

# servers_start_native: the recorded number of instances under the current
# CAPACITY_SERVER_ENV and, above one, the balancer.
servers_start_native() {
  local count bin="${CAPACITY_FERROEHR:?CAPACITY_FERROEHR names the ferroehr binary}" i
  count=$(state_get instances)
  local -a env=(FERROEHR_CONFIG="$CAP_OUT_ABS/ferroehr.toml"
    FERROEHR__DATABASE__URL="postgres://ferroehr:${PG_SECRET}@127.0.0.1:$CAP_PG_PORT/ferroehr"
    FERROEHR__USAGE_REPORT__ENABLED=false
    FERROEHR__SERVER__MAX_CONCURRENT_REQUESTS="$MAX_CONCURRENT"
    FERROEHR__DATABASE__CONNECTION_BUDGET="$BUDGET")
  [[ -z "$POOL" ]] || env+=(FERROEHR__DATABASE__MAX_CONNECTIONS="$POOL")
  local -a kvs=()
  read -r -a kvs <<<"${CAPACITY_SERVER_ENV:-}"
  env+=(${kvs[@]+"${kvs[@]}"})
  local targets="" urls=""
  for ((i = 1; i <= count; i++)); do
    env "${env[@]}" FERROEHR__SERVER__LISTEN="127.0.0.1:$((CAP_HTTP_PORT + i))" \
      FERROEHR__SERVER__PUBLIC_BASE_URL="http://127.0.0.1:$((CAP_HTTP_PORT + i))" \
      "$bin" >"$CAP_OUT_ABS/server-$i.log" 2>&1 &
    targets+="pid:$! "
    urls+="http://127.0.0.1:$((CAP_HTTP_PORT + i)) "
    wait_for "FerroEHR $i" 120 curl -fsS "http://127.0.0.1:$((CAP_HTTP_PORT + i))/health/readiness"
  done
  state_put ferroehr "env ${env[*]} $bin"
  state_put servers "$targets"
  state_put instance_urls "$urls"
  if ((count > 1)); then
    local caddy="${CAPACITY_CADDY:?CAPACITY_CADDY names the caddy binary for more than one instance}"
    lb_config "${urls//http:\/\//}" "127.0.0.1:$CAP_LB_PORT" >"$CAP_OUT_ABS/Caddyfile"
    "$caddy" run --config "$CAP_OUT_ABS/Caddyfile" --adapter caddyfile >"$CAP_OUT_ABS/caddy.log" 2>&1 &
    state_put lb "pid:$!"
    wait_for "Caddy" 30 curl -fsS "http://127.0.0.1:$CAP_LB_PORT/health/readiness"
    state_put url "http://127.0.0.1:$CAP_LB_PORT"
  else
    state_put url "http://127.0.0.1:$((CAP_HTTP_PORT + 1))"
  fi
}

# lb_config <upstreams> <bind>: round robin over the instances, no sticky
# sessions, the request log off (the instances log every request).
lb_config() {
  cat <<EOF
{
  admin off
  auto_https off
}
http://$2 {
  reverse_proxy $1 {
    lb_policy round_robin
    transport http {
      keepalive 90s
      keepalive_idle_conns_per_host 512
    }
  }
}
EOF
}

up_docker() { # <instances>
  local count="$1" image="${CAPACITY_IMAGE:?CAPACITY_IMAGE names the FerroEHR image}"
  local pg_image="${CAPACITY_PG_IMAGE:?CAPACITY_PG_IMAGE names the PostgreSQL image}" i
  local -a pg_args=()
  local line
  while IFS= read -r line; do
    [[ -n "$line" ]] && pg_args+=(-c "$(sed -E "s/ = /=/; s/'//g" <<<"$line")")
  done < <(pg_settings)
  local -a pg_cpus=()
  [[ -n "${CAPACITY_PG_CPUS:-}" ]] && pg_cpus=(--cpuset-cpus "$CAPACITY_PG_CPUS")
  docker network create "$CAP_NET" >/dev/null
  docker run -d --name "$CAP_PG" --network "$CAP_NET" --shm-size 1g ${pg_cpus[@]+"${pg_cpus[@]}"} \
    -p "127.0.0.1:${CAP_PG_PORT}:5432" -e "POSTGRES_PASSWORD=$PG_SUPER_SECRET" \
    -e PG_INIT_USER=ferroehr -e "PG_INIT_PASSWORD=$PG_SECRET" -e PG_INIT_DB=ferroehr \
    "$pg_image" "${pg_args[@]}" >/dev/null
  wait_for "PostgreSQL" 60 docker exec "$CAP_PG" pg_isready -h 127.0.0.1 -U ferroehr -d ferroehr
  # No -i: psql takes its statement as an argument, and an attached stdin
  # would swallow the input of a loop that runs a sweep.
  state_put psql "docker exec $CAP_PG psql -U postgres -d ferroehr"
  cap_sql "CREATE EXTENSION IF NOT EXISTS pg_stat_statements" >/dev/null
  state_put pg_cpu "container:$(docker inspect -f '{{.Id}}' "$CAP_PG")"
  # A key an earlier `up` generated is kept, so a restored store verifies.
  [[ -f "$PERF_OUT/signing/signing.asc" ]] || signing_key
  instance_config "$CAP_OUT_ABS" /opt/sandbox-posture/basic-users-jwks.json \
    /etc/ferroehr/scim-jwks.json /etc/ferroehr/signing.asc /etc/ferroehr/signing-passphrase
  state_put instances "$count"
  state_put image "$image"
  state_put server_cpus "${CAPACITY_SERVER_CPUS:-}"
  servers_start_docker
}

# servers_start_docker: the recorded number of instances of the recorded
# image under the current CAPACITY_SERVER_ENV and, above one, the balancer.
servers_start_docker() {
  local count image cpus i
  count=$(state_get instances)
  image=$(state_get image)
  cpus=$(cat "$STATE/server_cpus" 2>/dev/null || true)
  local -a server_cpus=()
  [[ -n "$cpus" ]] && server_cpus=(--cpuset-cpus "$cpus")
  local -a extra_env=(-e "FERROEHR__DATABASE__CONNECTION_BUDGET=$BUDGET")
  [[ -z "$POOL" ]] || extra_env+=(-e "FERROEHR__DATABASE__MAX_CONNECTIONS=$POOL")
  local kv
  local -a kvs=()
  read -r -a kvs <<<"${CAPACITY_SERVER_ENV:-}"
  for kv in ${kvs[@]+"${kvs[@]}"}; do extra_env+=(-e "$kv"); done
  # json-file: the request log is an instrument (server_latency), and journald,
  # Podman's default, suppresses a container's lines past its rate limit.
  local -a process=(--network "$CAP_NET" ${server_cpus[@]+"${server_cpus[@]}"} --read-only --tmpfs /tmp --cap-drop ALL
    --log-driver json-file
    --security-opt no-new-privileges:true
    -e FERROEHR_CONFIG=/etc/ferroehr/ferroehr.toml
    -e "FERROEHR__DATABASE__URL=postgres://ferroehr:${PG_SECRET}@${CAP_PG}:5432/ferroehr"
    -e "FERROEHR__SERVER__MAX_CONCURRENT_REQUESTS=$MAX_CONCURRENT"
    ${extra_env[@]+"${extra_env[@]}"}
    -e FERROEHR__USAGE_REPORT__ENABLED=false
    -v "$CAP_OUT_ABS/ferroehr.toml:/etc/ferroehr/ferroehr.toml:ro"
    -v "$CAPACITY_ROOT/docs/conformance/party/scim/jwks.json:/etc/ferroehr/scim-jwks.json:ro"
    -v "$PERF_OUT/signing/signing.asc:/etc/ferroehr/signing.asc:ro"
    -v "$PERF_OUT/signing/passphrase:/etc/ferroehr/signing-passphrase:ro")
  local targets="" upstreams="" urls=""
  for ((i = 1; i <= count; i++)); do
    docker run -d --name "$CAP_SERVER-$i" -p "127.0.0.1:$((CAP_HTTP_PORT + i)):8080" \
      "${process[@]}" "$image" >/dev/null
    targets+="container:$(docker inspect -f '{{.Id}}' "$CAP_SERVER-$i") "
    upstreams+="$CAP_SERVER-$i:8080 "
    urls+="http://127.0.0.1:$((CAP_HTTP_PORT + i)) "
    wait_for "FerroEHR $i" 120 curl -fsS "http://127.0.0.1:$((CAP_HTTP_PORT + i))/health/readiness"
  done
  state_put ferroehr "docker exec $CAP_SERVER-1 /usr/local/bin/ferroehr"
  state_put servers "$targets"
  state_put instance_urls "$urls"
  if ((count > 1)); then
    local caddy="${CAPACITY_CADDY_IMAGE:?CAPACITY_CADDY_IMAGE names the Caddy image for more than one instance}"
    lb_config "$upstreams" ":8080" >"$CAP_OUT_ABS/Caddyfile"
    docker run -d --name "$CAP_LB" --network "$CAP_NET" ${server_cpus[@]+"${server_cpus[@]}"} -p "127.0.0.1:${CAP_LB_PORT}:8080" \
      -v "$CAP_OUT_ABS/Caddyfile:/etc/caddy/Caddyfile:ro" "$caddy" >/dev/null
    state_put lb "container:$(docker inspect -f '{{.Id}}' "$CAP_LB")"
    wait_for "Caddy" 30 curl -fsS "http://127.0.0.1:$CAP_LB_PORT/health/readiness"
    state_put url "http://127.0.0.1:$CAP_LB_PORT"
    state_put net_url "http://$CAP_LB:8080"
  else
    state_put url "http://127.0.0.1:$((CAP_HTTP_PORT + 1))"
    state_put net_url "http://$CAP_SERVER-1:8080"
  fi
}

# servers_stop: the balancer and every instance stopped, the store and the
# database left as they are.
servers_stop() {
  local spec pid
  if [[ "$TOPOLOGY" == docker ]]; then
    docker rm -f "$CAP_LB" >/dev/null 2>&1 || true
    for spec in 1 2 3 4 5 6 7 8; do docker rm -f "$CAP_SERVER-$spec" >/dev/null 2>&1 || true; done
  else
    for spec in $(cat "$STATE/lb" 2>/dev/null) $(cat "$STATE/servers" 2>/dev/null); do
      pid="${spec#pid:}"
      kill "$pid" 2>/dev/null || true
      while kill -0 "$pid" 2>/dev/null; do sleep 0.2; done
    done
  fi
  rm -f "$STATE/lb" "$STATE/servers"
}

servers_start() {
  if [[ "$TOPOLOGY" == docker ]]; then servers_start_docker; else servers_start_native; fi
}

down() {
  servers_stop
  if [[ "$TOPOLOGY" == docker ]]; then
    docker rm -f "$CAP_GEN" >/dev/null 2>&1 || true
    # -v: the PostgreSQL image declares its data directory a volume, and a
    # container removed without it leaves that volume behind on the host.
    docker rm -f -v "$CAP_PG" >/dev/null 2>&1 || true
    docker network rm "$CAP_NET" >/dev/null 2>&1 || true
  else
    [[ -d "$CAP_OUT_ABS/pg" && -n "${CAPACITY_PG_BIN:-}" ]] \
      && "$CAPACITY_PG_BIN/pg_ctl" -D "$CAP_OUT_ABS/pg" -m fast stop >/dev/null 2>&1 || true
  fi
  rm -rf "$STATE"
}

# cap_admin_sql <statement>: one statement as the superuser in the database
# postgres, for the statements that copy or drop the store.
cap_admin_sql() {
  local line
  line=$(state_get psql)
  local -a psql
  read -r -a psql <<<"${line%ferroehr}postgres"
  "${psql[@]}" -Atq -c "$1"
}

# snapshot: the seeded store kept as a template database, the servers
# stopped while PostgreSQL copies it (a template has no other session).
snapshot() {
  [[ -s "$STATE/spread.tsv" ]] || cap_die "the store is not seeded: run \`capacity.sh seed\` first"
  servers_stop
  cap_admin_sql "DROP DATABASE IF EXISTS $SNAPSHOT_DB" >/dev/null
  cap_admin_sql "CREATE DATABASE $SNAPSHOT_DB TEMPLATE ferroehr STRATEGY FILE_COPY" >/dev/null
  servers_start
  state_put snapshot "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "capacity: the seeded store is kept as $SNAPSHOT_DB"
}

# restore: the store replaced by a copy of the snapshot, and the servers
# started again, so no in-memory copy outlives the store it was read from.
restore() {
  [[ -f "$STATE/snapshot" ]] || cap_die "no snapshot: run \`capacity.sh snapshot\` after the seed"
  servers_stop
  cap_admin_sql "DROP DATABASE ferroehr WITH (FORCE)" >/dev/null
  cap_admin_sql "CREATE DATABASE ferroehr TEMPLATE $SNAPSHOT_DB OWNER ferroehr STRATEGY FILE_COPY" >/dev/null
  servers_start
  prime
}

# prime: each route of the mix sent PRIME_WIDTH times at once to each
# instance, so every pooled connection runs each route's statements once.
# It warms the connections only (opened, their backends and statements
# prepared), never the working set: each route reads one EHR, so the
# per-EHR copies of a restarted server stay as cold as a restart leaves
# them, and the step measures that cost. The commit lands in the first EHR
# the writes take.
prime() {
  local inst clinician ehr uid write aql_ehr round k pid
  local body="$CAP_OUT_ABS/prime.$BASHPID"
  clinician=$(seed_value CLINICAL_AUTH)
  IFS=$'\t' read -r ehr uid <"$STATE/spread.tsv"
  read -r write <"$STATE/writes.ids"
  read -r aql_ehr <"$STATE/aql.ids"
  aql "$aql_ehr" true >"$body.aql"
  jq -cn --arg q "$POPULATION_AQL" '{q: $q}' >"$body.population"
  for inst in $(state_get instance_urls); do
    local -a pids=()
    for ((k = 0; k < PRIME_WIDTH; k++)); do
      prime_get "$inst$BASE/ehr/$ehr" "$clinician" &
      pids+=("$!")
      prime_get "$inst$BASE/ehr/$ehr/composition/$uid" "$clinician" &
      pids+=("$!")
      prime_post "$inst$BASE/ehr/$write/composition" "$clinician" "$COMPOSITION" &
      pids+=("$!")
      prime_post "$inst$BASE/query/aql" "$clinician" "$body.aql" &
      pids+=("$!")
      prime_post "$inst$BASE/query/aql" "$clinician" "$body.population" &
      pids+=("$!")
      prime_get "$inst/ferroehr/rest/fhir/r4/AuditEvent?date=ge2026-01-01&_count=20" "$AUTH" &
      pids+=("$!")
      prime_get "$inst$BASE/demographic/person/$(seed_value READ_PARTY)" "$AUTH" &
      pids+=("$!")
    done
    for pid in "${pids[@]}"; do
      wait "$pid" || cap_die "priming $inst failed"
    done
  done
  rm -f "$body.aql" "$body.population"
}

# prime_get <url> <auth>: one primed read.
prime_get() {
  curl -fsS -o /dev/null -H "authorization: $2" -H "accept: application/json" "$1"
}

# prime_post <url> <auth> <body-file>: one primed POST of a JSON body.
prime_post() {
  curl -fsS -o /dev/null -H "authorization: $2" -H "accept: application/json" \
    -H "content-type: application/json" -H "prefer: return=minimal" --data-binary "@$3" "$1"
}

# cap_sql <statement>: one statement as the superuser, unaligned.
cap_sql() {
  local -a psql
  read -r -a psql <<<"$(state_get psql)"
  "${psql[@]}" -Atq -c "$1"
}

# cpu_table <spec>: "<id> <CPU seconds>" for a process, each of a process
# and its children, or a container.
cpu_table() {
  local spec="$1" id stat
  case "$spec" in
  pid:*) printf '%s ' "${spec#pid:}"; ps -o time= -p "${spec#pid:}" | ps_seconds ;;
  children:*)
    id="${spec#children:}"
    ps -A -o pid=,ppid=,time= | awk -v p="$id" '$1 == p || $2 == p { print $1, $3 }' \
      | while read -r child time; do printf '%s ' "$child"; ps_seconds <<<"$time"; done
    ;;
  container:*)
    id="${spec#container:}"
    # A host with no cgroup tree (macOS) has nothing to find, which is no failure.
    stat=$({ find /sys/fs/cgroup -maxdepth 3 -type d \( -name "docker-$id.scope" -o -name "$id" \) 2>/dev/null || true; } | sed -n 1p)
    if [[ -n "$stat" ]]; then
      awk '$1 == "usage_usec" { printf "c %.3f\n", $2 / 1e6 }' "$stat/cpu.stat"
    else
      # Podman in a VM (macOS): its stats name the container's CPU time as a
      # Go duration.
      docker stats --no-stream --format json "$id" 2>/dev/null | jq -r '.[0].cpu_time // "0s"' \
        | awk '{ t = $1; s = 0
            while (match(t, /^[0-9.]+(h|ms|us|µs|ns|m|s)/)) {
              u = substr(t, RSTART, RLENGTH); t = substr(t, RLENGTH + 1)
              n = u + 0; sub(/^[0-9.]+/, "", u)
              s += n * (u == "h" ? 3600 : u == "m" ? 60 : u == "s" ? 1 : u == "ms" ? 1e-3 : u == "ns" ? 1e-9 : 1e-6) }
            printf "c %.3f\n", s }'
    fi
    ;;
  esac
}

# cpu_delta <spec> <before>: the CPU seconds spent since the table <before>
# was taken; a process that started since counts whole, one that ended is
# lost (the sampler's own short-lived backends).
cpu_delta() {
  cpu_table "$1" | awk -v f="$2" 'BEGIN { while ((getline l < f) > 0) { split(l, a, " "); b[a[1]] = a[2] } }
    { d = $2 - ($1 in b ? b[$1] : 0); if (d > 0) t += d } END { printf "%.3f\n", t }'
}

# ps_seconds: the sum of ps `time` cells ([[dd-]hh:]mm:ss[.cc]) on stdin.
ps_seconds() {
  awk '{ t = $1; d = 0; if (index(t, "-")) { split(t, a, "-"); d = a[1]; t = a[2] }
    n = split(t, p, ":"); s = 0; for (i = 1; i <= n; i++) s = s * 60 + p[i]
    total += s + d * 86400 } END { printf "%.3f\n", total }'
}

# spread_one <ehr>: one composition committed into <ehr>, and the line
# naming both on stdout.
spread_one() {
  local uid
  uid=$(call POST "/ehr/$1/composition" application/json "$COMPOSITION" 201 | jq -er '.uid.value')
  printf '%s\t%s\n' "$1" "$uid"
}

# seed_capacity: the gate's seed and the spread compositions. A seed that
# stopped after the gate's seed (its values recorded) resumes at the spread.
seed_capacity() {
  local ferroehr first_instance ehr
  ferroehr=$(state_get ferroehr)
  read -r first_instance _ <<<"$(state_get instance_urls)"
  mkdir -p "$PERF_OUT"
  # The gate's seed through the first instance, whose binary grants.
  SERVER_URL="$first_instance"
  read -r -a ACCESS_FERROEHR <<<"$ferroehr"
  if [[ ! -s "$STATE/seed.env" ]]; then
    SCIM_GRANT="$ferroehr" bash "$CAPACITY_ROOT/scripts/scim-provision.sh" "$SERVER_URL" \
      basic-users urn:example:basic-user ferroehr >/dev/null
    ACCESS_IDS="$PERF_OUT/scim-ids"
    : >"$ACCESS_IDS"
    SCIM_IDS="$ACCESS_IDS" SCIM_GRANT="$ferroehr" bash "$CAPACITY_ROOT/scripts/scim-provision.sh" \
      "$SERVER_URL" basic-users urn:example:basic-user --roles USER clinician >/dev/null
    seed "$CAPACITY_ROOT"
    printf '%s\n' "${SEED[@]}" "NAMED_PROFESSIONAL=$NAMED_PROFESSIONAL" \
      "REPRESENTATIVE=$REPRESENTATIVE" >"$STATE/seed.env"
  fi
  # The parties the directives name, for a seed that resumes.
  NAMED_PROFESSIONAL=$(seed_value NAMED_PROFESSIONAL)
  REPRESENTATIVE=$(seed_value REPRESENTATIVE)
  local held
  held=$(wc -l <"$PERF_OUT/access-population.ids" | tr -d ' ')
  ((held >= 3 * SPREAD + CHART_EHRS)) \
    || cap_die "$held population EHRs hold no $((3 * SPREAD + CHART_EHRS)) for the reads, the commits and the AQL: raise CAPACITY_POPULATION"
  # One composition in each of the first SPREAD population EHRs, sixteen at
  # a time.
  : >"$STATE/spread.tsv"
  while read -r ehr; do
    spread_one "$ehr" >>"$STATE/spread.tsv" 2>>"$PERF_OUT/spread.log" &
    if (($(jobs -pr | wc -l) >= 16)); then wait -n || true; fi
  done < <(head -n "$SPREAD" "$PERF_OUT/access-population.ids")
  wait || true
  [[ "$(wc -l <"$STATE/spread.tsv" | tr -d ' ')" == "$SPREAD" ]] \
    || cap_die "$(wc -l <"$STATE/spread.tsv" | tr -d ' ') of $SPREAD spread compositions were committed: $(head -c 400 "$PERF_OUT/spread.log")"
  sed -n "$((SPREAD + 1)),$((2 * SPREAD))p" "$PERF_OUT/access-population.ids" >"$STATE/writes.ids"
  sed -n "$((2 * SPREAD + 1)),$((3 * SPREAD))p" "$PERF_OUT/access-population.ids" >"$STATE/aql.ids"
  sed -n "$((3 * SPREAD + 1)),$((3 * SPREAD + CHART_EHRS))p" "$PERF_OUT/access-population.ids" \
    >"$STATE/chart.ids"
  fill "$STATE/aql.ids" "$AQL_ROWS" "$STATE/aql.tsv"
  fill "$STATE/chart.ids" "$CHART_ROWS" "$STATE/chart.tsv"
  cap_sql "VACUUM ANALYZE" >/dev/null
  echo "capacity: seeded $PERF_POPULATION patients under care, $SPREAD with a composition," \
    "$SPREAD with $AQL_ROWS lab results and $CHART_EHRS with $CHART_ROWS"
}

# fill <ids> <per-ehr> <out>: <per-ehr> compositions committed into each EHR
# of <ids>, sixteen at a time, one line each in <out>; a fill already
# complete is kept, so a seed resumes.
fill() {
  local ids="$1" per="$2" out="$3" ehr k want
  want=$(($(wc -l <"$ids" | tr -d ' ') * per))
  [[ -f "$out" && "$(wc -l <"$out" | tr -d ' ')" == "$want" ]] && return 0
  : >"$out"
  while read -r ehr; do
    for ((k = 0; k < per; k++)); do
      spread_one "$ehr" >>"$out" 2>>"$PERF_OUT/spread.log" &
      if (($(jobs -pr | wc -l) >= 16)); then wait -n || true; fi
    done
  done <"$ids"
  wait || true
  [[ "$(wc -l <"$out" | tr -d ' ')" == "$want" ]] \
    || cap_die "$(wc -l <"$out" | tr -d ' ') of $want compositions for $(basename "$out" .tsv) were committed: $(head -c 400 "$PERF_OUT/spread.log")"
}

seed_value() { # <name>
  awk -F= -v n="$1" '$1 == n { sub(/^[^=]*=/, ""); print; exit }' "$STATE/seed.env"
}

# plan_request <group> <method> <path> <status> <auth> [<body-file> <type>]:
# one [[requests]] entry.
plan_request() {
  local group="$1" method="$2" path="$3" status="$4" auth="$5" body="${6:-}" type="${7:-}"
  echo "[[requests]]"
  echo "group = \"$group\""
  echo "method = \"$method\""
  echo "path = \"$path\""
  echo "status = $status"
  if [[ -n "$body" ]]; then
    printf "body = '''%s'''\n" "$(<"$body")"
  fi
  local headers="accept = \"application/json\""
  [[ -n "$auth" ]] && headers="authorization = \"$auth\", $headers"
  [[ -n "$type" ]] && headers+=", content-type = \"$type\""
  [[ "$method" == POST ]] && headers+=", prefer = \"return=minimal\""
  echo "headers = { $headers }"
  echo
}

# group_requests <group>: the entries of one group, spread over the seeded
# EHRs where the group names one.
group_requests() {
  local group="$1" clinician admin ehr uid i=0 body="$CAP_OUT_ABS/body.$BASHPID"
  clinician=$(seed_value CLINICAL_AUTH)
  admin="$AUTH"
  case "$group" in
  ehr_read)
    while IFS=$'\t' read -r ehr uid; do
      plan_request ehr_read GET "$BASE/ehr/$ehr" 200 "$clinician"
    done <"$STATE/spread.tsv"
    ;;
  composition_read)
    while IFS=$'\t' read -r ehr uid; do
      plan_request composition_read GET "$BASE/ehr/$ehr/composition/$uid" 200 "$clinician"
    done <"$STATE/spread.tsv"
    ;;
  composition_commit)
    # The next SPREAD population EHRs, never the read set: a commit into an
    # EHR the single-EHR queries read grows their answers step by step.
    while read -r ehr; do
      plan_request composition_commit POST "$BASE/ehr/$ehr/composition" 201 "$clinician" \
        "$COMPOSITION" application/json
    done <"$STATE/writes.ids"
    ;;
  aql_ehr | aql_chart)
    # aql_ehr reads EHRs holding a typical answer, aql_chart EHRs holding a
    # whole chart (#5746).
    local ids="$STATE/aql.ids"
    [[ "$group" == aql_chart ]] && ids="$STATE/chart.ids"
    while read -r ehr; do
      aql "$ehr" true >"$body.$i"
      plan_request "$group" POST "$BASE/query/aql" 200 "$clinician" "$body.$i" application/json
      rm -f "$body.$i"
      i=$((i + 1))
    done <"$ids"
    ;;
  aql_population)
    jq -cn --arg q "$POPULATION_AQL" '{q: $q}' >"$body"
    plan_request aql_population POST "$BASE/query/aql" 200 "$clinician" "$body" application/json
    rm -f "$body"
    ;;
  access_log_read)
    plan_request access_log_read GET "/ferroehr/rest/fhir/r4/AuditEvent?date=ge2026-01-01&_count=20" \
      200 "$admin"
    ;;
  party_read)
    plan_request party_read GET "$BASE/demographic/person/$(seed_value READ_PARTY)" 200 "$admin"
    ;;
  export)
    while IFS=$'\t' read -r ehr uid; do
      plan_request export GET "/ferroehr/rest/extensions/v1/message/export/$ehr" 200 "$clinician"
    done <"$STATE/spread.tsv"
    ;;
  health)
    plan_request health GET /health/liveness 200 ""
    ;;
  *) cap_die "unknown group $group (capacity.sh groups lists them)" ;;
  esac
}

group_weight() { # <group>
  awk -v g="$1" '$1 == g { print $2 }' <<<"$GROUPS_TABLE"
}

# group_slo <group>: its p99 SLO in milliseconds.
group_slo() {
  case "$1" in
  aql_population | access_log_read | export) echo "$SLO_SLOW_MS" ;;
  *) echo "$SLO_MS" ;;
  esac
}

# plan <group|mix> <rate> <out>: the step's plan. The mix interleaves the
# groups by weight, so each round of the set sends every group its share.
plan() {
  local what="$1" rate="$2" out="$3" group weight dir
  dir=$(dirname "$out")
  {
    echo "[idle]"
    echo "duration_secs = 5"
    echo "max_cpu_percent = 100.0"
    echo "max_transactions_per_second = 1000.0"
    echo "settle_secs = 2"
    echo
    echo "[load]"
    echo "rate = $rate"
    echo "warmup_secs = $WARMUP"
    echo "duration_secs = $DURATION"
    echo "max_in_flight = 4096"
    echo "saturation_ms = 5000"
    echo "timeout_ms = 10000"
    echo "max_lag_ms = 10"
    echo "sample_ms = 100"
    echo "settle_secs = 2"
    echo
  } >"$out"
  if [[ "$what" != mix ]]; then
    group_requests "$what" >>"$out"
    return 0
  fi
  # Each group's entries in a file of their own; then one round of the mix
  # takes `weight` entries of each group, round after round, until the
  # longest group is used up.
  local -a mixed=() files=()
  while read -r group weight _; do
    [[ -n "$group" && "$weight" -gt 0 ]] || continue
    group_requests "$group" | awk -v RS= -v ORS='\n\n' '{ print }' >"$dir/mix.$group"
    mixed+=("$group:$weight")
    files+=("$dir/mix.$group")
  done <<<"$GROUPS_TABLE"
  awk -v RS= -v ORS='\n\n' -v spec="${mixed[*]}" '
    BEGIN { n = split(spec, s, " ")
      for (i = 1; i <= n; i++) { split(s[i], gw, ":"); g[i] = gw[1]; w[i] = gw[2] } }
    { split(FILENAME, f, "mix."); grp = f[length(f)]; c[grp]++; e[grp, c[grp]] = $0 }
    END {
      rounds = 0; for (i = 1; i <= n; i++) { r = int((c[g[i]] + w[i] - 1) / w[i]); if (r > rounds) rounds = r }
      for (r = 0; r < rounds; r++) for (i = 1; i <= n; i++) for (k = 0; k < w[i]; k++) {
        idx = (r * w[i] + k) % c[g[i]] + 1; print e[g[i], idx] } }' \
    "${files[@]}" >>"$out"
  rm -f "$dir"/mix.*
}

perf_bin() {
  local bin="${PERF_BIN:-$CAPACITY_ROOT/target/tools/perf/bin/ferrohealth-perf}"
  [[ -x "$bin" ]] || cap_die "no ferrohealth-perf at $bin: run scripts/perf/gate.sh install"
  [[ "$("$bin" --version | awk '{ print $2 }')" == "$(pin)" ]] \
    || cap_die "$bin is not the ferrohealth-perf docs/VERSIONS.md pins ($(pin))"
  echo "$bin"
}

# scrape <url> <out>: an instance's Prometheus surface.
scrape() {
  curl -fsS -H "authorization: $AUTH" "$1/management/prometheus" >"$2" 2>/dev/null || : >"$2"
}

# sample_activity <out> <stop>: the database's backends by state and wait,
# four times a second until <stop> exists.
sample_activity() {
  local out="$1" stop="$2"
  while [[ ! -e "$stop" ]]; do
    cap_sql "SELECT coalesce(state, '-') || '|' || coalesce(wait_event_type, 'CPU') || ':' || \
      coalesce(wait_event, '-') FROM pg_stat_activity WHERE datname = 'ferroehr' \
      AND pid <> pg_backend_pid() AND backend_type = 'client backend'" >>"$out" 2>/dev/null || true
    echo "--" >>"$out"
    sleep 0.25
  done
}

# The Linux generator `capacity.sh generator` builds, for a generator on the
# servers' network: its root on this host and its path in the container.
readonly GEN_ROOT="$CAPACITY_ROOT/target/tools/perf-linux"
readonly GEN_BIN=/opt/perf/bin/ferrohealth-perf

# builder_image: the Rust image the server's own Dockerfile builds with,
# pinned by digest there, which builds and runs the Linux generator.
builder_image() {
  local version from
  version=$(sed -n 's/^ARG RUST_VERSION=//p' "$CAPACITY_ROOT/docker/Dockerfile" | sed -n 1p)
  # shellcheck disable=SC2016 # ${RUST_VERSION} is the Dockerfile's own text, matched literally
  from=$(sed -n 's/^FROM rust:\${RUST_VERSION}\([^ ]*\) AS builder$/\1/p' "$CAPACITY_ROOT/docker/Dockerfile" | sed -n 1p)
  [[ -n "$version" && "$from" == *@sha256:* ]] || cap_die "docker/Dockerfile names no pinned rust builder image"
  echo "rust:$version$from"
}

generator_install() {
  [[ "$TOPOLOGY" == docker ]] || cap_die "the network generator needs CAPACITY_TOPOLOGY=docker"
  mkdir -p "$GEN_ROOT"
  docker run --rm -v "$GEN_ROOT:/opt/perf" "$(builder_image)" \
    cargo install ferrohealth-perf --version "=$(pin)" --locked --root /opt/perf
}

# generator_check: the Linux generator is built and is the pinned version.
generator_check() {
  [[ "$TOPOLOGY" == docker ]] || cap_die "CAPACITY_GENERATOR=network needs CAPACITY_TOPOLOGY=docker"
  [[ -x "$GEN_ROOT/bin/ferrohealth-perf" ]] || cap_die "no Linux generator at $GEN_ROOT: run \`capacity.sh generator\`"
  [[ "$(docker run --rm -v "$GEN_ROOT:/opt/perf:ro" "$(builder_image)" "$GEN_BIN" --version | awk '{ print $2 }')" == "$(pin)" ]] \
    || cap_die "the Linux generator at $GEN_ROOT is not the ferrohealth-perf docs/VERSIONS.md pins ($(pin)): run \`capacity.sh generator\`"
}

# kernel_now: the epoch second on the clock the servers' request log reads.
kernel_now() {
  if [[ "$TOPOLOGY" == docker ]]; then docker exec "$CAP_PG" date +%s; else date +%s; fi
}

# step_dir <group|mix> <rate> [<arm>]: where a step's runs and verdict go.
step_dir() {
  local arm="${3:-$ARM}"
  echo "$CAP_OUT_ABS/steps${arm:+/$arm}/$1/$2"
}

# step_run <group|mix> <rate> <dir>: one run of one rate with its
# instruments around it, its verdict in <dir>/step.json.
step_run() {
  local what="$1" rate="$2" dir="$3" bin url spec status=0 instances
  if [[ "$GENERATOR" == network ]]; then
    bin="$GEN_BIN"
    url=$(state_get net_url)
  else
    bin=$(perf_bin)
    url=$(state_get url)
  fi
  rm -rf "$dir"
  mkdir -p "$dir"
  plan "$what" "$rate" "$dir/plan.toml"
  local -a servers
  read -r -a servers <<<"$(state_get servers)"
  read -r -a instances <<<"$(state_get instance_urls)"
  local pg_cpu lb=""
  pg_cpu=$(state_get pg_cpu)
  [[ -f "$STATE/lb" ]] && lb=$(state_get lb)
  local n=0
  for spec in "${servers[@]}"; do cpu_table "$spec" >"$dir/cpu.server.$n"; n=$((n + 1)); done
  cpu_table "$pg_cpu" >"$dir/cpu.pg"
  [[ -n "$lb" ]] && cpu_table "$lb" >"$dir/cpu.lb"
  storage_json >"$dir/storage.before.json"
  cap_sql "SELECT pg_stat_statements_reset()" >/dev/null
  local xact_before
  xact_before=$(cap_sql "SELECT xact_commit + xact_rollback FROM pg_stat_database WHERE datname = 'ferroehr'")
  host_snapshot "$dir/host.before"
  rm -f "$dir/stop"
  sample_activity "$dir/activity.txt" "$dir/stop" &
  local sampler=$!
  local started ended since
  since=$(kernel_now)
  started=$(date +%s)
  # The generator's own CPU, from the shell's account of the child.
  local TIMEFORMAT='%U %S'
  local -a pin=()
  if [[ "$GENERATOR" == network ]]; then
    # In a container on the servers' network: it reaches them by name, with
    # no port forwarder between, and its shell times it there.
    [[ -n "${CAPACITY_GENERATOR_CPUS:-}" ]] && pin=(--cpuset-cpus "$CAPACITY_GENERATOR_CPUS")
    docker rm -f "$CAP_GEN" >/dev/null 2>&1 || true
    docker run --rm --name "$CAP_GEN" --network "$CAP_NET" ${pin[@]+"${pin[@]}"} \
      -v "$CAP_OUT_ABS:$CAP_OUT_ABS" -v "$GEN_ROOT:/opt/perf:ro" "$(builder_image)" \
      bash -c 'TIMEFORMAT="%U %S"; d="$1"; shift; { time "$@" >"$d/perf.log" 2>&1; } 2>"$d/generator.time"' _ "$dir" \
      "$bin" run --plan "$dir/plan.toml" --baseline "$dir/none.toml" --pid 1 \
      --base-url "$url" --out-dir "$dir" --summary "$dir/summary.md" || status=$?
  else
    # ferrohealth-perf reads its target's CPU from /proc or the cgroup; a
    # container in a VM (Podman on macOS) has neither on this host, so it is
    # given this shell, and the step reads the container's CPU itself.
    local -a target=(--pid "${servers[0]#pid:}")
    if [[ "${servers[0]}" == container:* ]]; then
      target=(--pid "$$")
      [[ -d /sys/fs/cgroup ]] && target=(--container "${servers[0]#container:}")
    fi
    [[ -n "${CAPACITY_GENERATOR_CPUS:-}" ]] && pin=(taskset -c "$CAPACITY_GENERATOR_CPUS")
    { time ${pin[@]+"${pin[@]}"} "$bin" run --plan "$dir/plan.toml" --baseline "$dir/none.toml" "${target[@]}" \
      --base-url "$url" --out-dir "$dir" --summary "$dir/summary.md" >"$dir/perf.log" 2>&1; } \
      2>"$dir/generator.time" || status=$?
  fi
  ended=$(date +%s)
  # The host's one-minute load average at the end, against its cores: a
  # shared host shows here.
  local load cores
  load=$(load_averages | awk '{ print $1 }')
  cores=$(getconf _NPROCESSORS_ONLN)
  touch "$dir/stop"
  wait "$sampler" || true
  host_snapshot "$dir/host.after"
  local server_cpu=0 pg_cpu_s lb_cpu_s=0 i=1 inst
  n=0
  for spec in "${servers[@]}"; do
    server_cpu=$(awk -v a="$server_cpu" -v b="$(cpu_delta "$spec" "$dir/cpu.server.$n")" 'BEGIN { printf "%.3f", a + b }')
    n=$((n + 1))
  done
  pg_cpu_s=$(cpu_delta "$pg_cpu" "$dir/cpu.pg")
  [[ -n "$lb" ]] && lb_cpu_s=$(cpu_delta "$lb" "$dir/cpu.lb")
  local xact_after
  xact_after=$(cap_sql "SELECT xact_commit + xact_rollback FROM pg_stat_database WHERE datname = 'ferroehr'")
  for inst in "${instances[@]}"; do scrape "$inst" "$dir/prometheus-$i.txt"; i=$((i + 1)); done
  cap_sql "SELECT json_agg(t) FROM (SELECT calls, round(total_exec_time::numeric, 1) AS total_ms, \
    round(mean_exec_time::numeric, 3) AS mean_ms, round(total_plan_time::numeric, 1) AS plan_total_ms, \
    round(mean_plan_time::numeric, 3) AS plan_mean_ms, rows, shared_blks_hit, shared_blks_read, \
    left(regexp_replace(query, '\s+', ' ', 'g'), 240) AS query FROM pg_stat_statements \
    WHERE query NOT ILIKE '%pg_stat%' ORDER BY total_exec_time + total_plan_time DESC LIMIT 15) t" \
    >"$dir/statements.json"
  storage_json >"$dir/storage.after.json"
  storage_step_json "$dir/storage.before.json" "$dir/storage.after.json" >"$dir/storage.json"
  memory_json "${servers[@]}" -- "$pg_cpu" >"$dir/memory.json"
  host_json "$dir/host.before" "$dir/host.after" "$((ended - started))" >"$dir/host.json"
  server_latency "$what" "$since" "$WARMUP" "$DURATION" "${servers[@]}" >"$dir/server_latency.json"
  metrics_json "$dir"/prometheus-*.txt >"$dir/server_metrics.json"
  local report
  report=$(find "$dir" -maxdepth 1 -name '*.json' ! -name statements.json ! -name step.json \
    ! -name 'storage*.json' ! -name memory.json ! -name host.json ! -name server_latency.json \
    ! -name activity.json ! -name server_metrics.json | sed -n 1p)
  [[ -n "$report" ]] || { cat "$dir/perf.log" >&2; cap_die "ferrohealth-perf wrote no report (exit $status)"; }
  activity_json "$dir/activity.txt" >"$dir/activity.json"
  jq -n --arg what "$what" --argjson rate "$rate" --slurpfile r "$report" \
    --argjson wall "$((ended - started))" --argjson server_cpu "$server_cpu" \
    --argjson pg_cpu "$pg_cpu_s" --argjson lb_cpu "$lb_cpu_s" \
    --argjson gen "$(awk '{ printf "%.3f", $1 + $2 }' "$dir/generator.time")" \
    --argjson xacts "$((xact_after - xact_before))" --argjson slo "$SLO_MS" \
    --argjson slo_slow "$SLO_SLOW_MS" --slurpfile activity "$dir/activity.json" \
    --argjson instances "${#servers[@]}" --argjson pool "${POOL:-null}" \
    --argjson host_load "$load" --argjson host_cores "$cores" \
    --slurpfile server "$dir/server_latency.json" --slurpfile host "$dir/host.json" \
    --slurpfile memory "$dir/memory.json" --slurpfile storage "$dir/storage.json" \
    --slurpfile metrics "$dir/server_metrics.json" \
    --arg generator "$GENERATOR" --arg variant "${CAPACITY_SERVER_ENV:-}" -f /dev/stdin >"$dir/step.json" <<'JQ'
    ($r[0].load) as $l
    | ($server[0]) as $s
    | def server_of($g): ($s | map(select(.group == $g or (.group == "aql" and ($g | startswith("aql_"))))) | .[0]);
    ($l.groups | to_entries | map(server_of(.key) as $sv | {group: .key, answered: .value.answered,
        p50: .value.quantiles.p50.value_ms, p95: .value.quantiles.p95.value_ms,
        p99: .value.quantiles.p99.value_ms,
        server_group: $sv.group, server_count: $sv.count,
        server_coverage: (if $sv == null then null else $sv.count / ([$l.groups | to_entries[] | select(.key == $sv.group or ($sv.group == "aql" and (.key | startswith("aql_")))) | .value.answered] | add) end),
        server_p50: $sv.p50, server_p95: $sv.p95, server_p99: $sv.p99,
        slo: (if (.key | test("^(aql_population|access_log_read|export)$")) then $slo_slow else $slo end)})) as $g
    | ([$g[].answered] | add) as $answered
    | ($answered / $l.measured_secs) as $throughput
    | ([($l.failures // {})[] | to_entries[] | .value] | add // 0) as $failed
    | {what: $what, rate: $rate, instances: $instances, pool: $pool, generator: $generator, variant: $variant,
       throughput: $throughput, answered: $answered, failed: $failed, failures: ($l.failures // {}),
       saturated: $l.saturated, lag_p50_ms: $l.lag_p50_ms, lag_p99_ms: $l.lag_p99_ms,
       lag_max_ms: $l.lag_max_ms, max_lag_ms: $l.max_lag_ms,
       groups: $g, server_latency: $s,
       server_cpu_s: $server_cpu, pg_cpu_s: $pg_cpu, lb_cpu_s: $lb_cpu, generator_cpu_s: $gen,
       wall_s: $wall, host_load_1m: $host_load, host_cores: $host_cores,
       host: $host[0], memory: $memory[0], storage: $storage[0], server_metrics: $metrics[0],
       server_cores: ($server_cpu / ($l.measured_secs + ($r[0].plan.warmup_secs))),
       pg_cores: ($pg_cpu / ($l.measured_secs + ($r[0].plan.warmup_secs))),
       server_cpu_ms_per_request: (if $answered > 0 then 1000 * $server_cpu / ($answered * (($l.measured_secs + $r[0].plan.warmup_secs) / $l.measured_secs)) else null end),
       pg_cpu_ms_per_request: (if $answered > 0 then 1000 * $pg_cpu / ($answered * (($l.measured_secs + $r[0].plan.warmup_secs) / $l.measured_secs)) else null end),
       transactions_per_request: (if $answered > 0 then $xacts / ($answered * (($l.measured_secs + $r[0].plan.warmup_secs) / $l.measured_secs)) else null end),
       activity: $activity[0]}
    | .passed = (.failed == 0 and (.saturated | not)
        and (.lag_p99_ms != null and .lag_p99_ms <= .max_lag_ms)
        and .throughput >= 0.98 * .rate
        and all(.groups[]; (.p99 // .p95) != null and (.p99 // .p95) <= .slo
          and (.server_p99 == null or .server_p99 <= .slo)))
JQ
}

# step_report <dir> <arm>: the step's runs folded into <dir>/step.json and
# appended to steps.jsonl, with its line on stdout; 0 when it passed.
step_report() {
  local dir="$1" arm="$2"
  local -a runs=()
  mapfile -t runs < <(find "$dir" -mindepth 2 -maxdepth 2 -name step.json -path '*/run-*' | sort -V)
  ((${#runs[@]})) || cap_die "no run of $dir wrote a verdict"
  fold_runs "$arm" "${runs[@]}" >"$dir/step.json"
  jq -r '"capacity: \(if .arm != "" then .arm + ": " else "" end)\(.what) at \(.rate) rps: \(if .passed then "PASS" else "FAIL" end) over \(.runs) run(s), \(.throughput | floor) rps answered, failed \(.failed), p99 " + ([.groups[] | "\(.group) \(.p99) (server \(.server_p99))"] | join(", ")) + ", server \(.server_cores * 100 | floor / 100) cores, pg \(.pg_cores * 100 | floor / 100) cores, host load \(.host_load_1m), others \(.other_cores_max) cores"' "$dir/step.json"
  jq -c . "$dir/step.json" >>"$CAP_OUT_ABS/steps.jsonl"
  [[ "$(jq -r .passed "$dir/step.json")" == true ]]
}

# step <group|mix> <rate>: CAPACITY_REPEAT runs of one rate, each from the
# snapshot under CAPACITY_FRESH; exits 0 when the step passed.
step() {
  local what="$1" rate="$2" dir k
  dir=$(step_dir "$what" "$rate")
  rm -rf "$dir"
  for ((k = 1; k <= REPEAT; k++)); do
    [[ "$FRESH" == 1 ]] && restore
    step_run "$what" "$rate" "$dir/run-$k"
  done
  step_report "$dir" "$ARM"
}

# activity_json <samples>: per state and wait, the mean backends over the
# samples, and the peak of active ones.
activity_json() {
  awk '
    $0 == "--" { samples++; if (act > peak) peak = act; act = 0; next }
    { split($0, a, "|"); key[$0]++; if (a[1] == "active") act++ }
    END { printf "{\"samples\":%d,\"peak_active\":%d,\"mean\":{", samples, peak; sep = ""
      for (k in key) { printf "%s\"%s\":%.2f", sep, k, (samples ? key[k] / samples : 0); sep = "," }
      print "}}" }' "$1"
}


# busy_host <group|mix> <rate>: whether the host's load average exceeded
# its cores at the end of that step.
busy_host() {
  jq -e '.host_load_1m > .host_cores' "$(step_dir "$1" "$2")/step.json" >/dev/null 2>&1
}

# sweep <group|mix> [<rate>...]: each rate in turn, every step in a process
# of its own so a harness error ends only that step. The knee is the highest
# rate below the first step that did not pass.
sweep() {
  local what="$1" rate knee=0 broken=0 status outcome
  shift
  local -a rates=("$@")
  ((${#rates[@]})) || read -r -a rates <<<"$RATES"
  for rate in "${rates[@]}"; do
    status=0
    "$BASH" "$CAPACITY_ROOT/scripts/perf/capacity.sh" step "$what" "$rate" || status=$?
    if ((status == 1)) && busy_host "$what" "$rate"; then
      # A step that failed while the host ran more than its cores is run once
      # more, so another tenant's burst is not read as the knee.
      status=0
      "$BASH" "$CAPACITY_ROOT/scripts/perf/capacity.sh" step "$what" "$rate" || status=$?
    fi
    case "$status" in
    0) outcome=pass ;;
    1) outcome=fail ;;
    *) outcome=error ;;
    esac
    jq -cn --arg arm "$ARM" --arg what "$what" --argjson rate "$rate" --arg outcome "$outcome" \
      --argjson exit "$status" '{arm: $arm, what: $what, rate: $rate, outcome: $outcome, exit: $exit}' \
      >>"$CAP_OUT_ABS/sweeps.jsonl"
    if [[ "$outcome" == pass ]] && ((broken == 0)); then
      knee="$rate"
      continue
    fi
    broken=1
    echo "capacity: $what at $rate rps: $outcome"
    [[ "$CONTINUE" == 1 ]] || break
  done
  echo "capacity: the knee of $what is ${knee} rps (the highest rate that passed with every rate below it)"
  state_put "knee-${ARM:+$ARM-}$what" "$knee"
  [[ "$knee" != 0 ]] || return 1
}

# arms <group|mix> <rate> <arm>=<env>...: CAPACITY_REPEAT rounds, each arm
# restored from the snapshot and run once per round under its own settings,
# the order reversed every other round so a host that drifts over the rounds
# weighs on every arm alike.
arms() {
  local what="$1" rate="$2" round spec label env status=0
  shift 2
  (($# >= 2)) || cap_die "arms takes at least two <arm>=<env>"
  local -a order=("$@") reversed=()
  for spec in "${order[@]}"; do
    label="${spec%%=*}"
    [[ "$spec" == *=* && "$label" =~ ^[A-Za-z0-9._-]+$ ]] || cap_die "an arm is <label>=<FERROEHR__ settings>, got '$spec'"
    reversed=("$spec" ${reversed[@]+"${reversed[@]}"})
    rm -rf "$(step_dir "$what" "$rate" "$label")"
  done
  local -a this
  for ((round = 1; round <= REPEAT; round++)); do
    this=("${order[@]}")
    ((round % 2 == 0)) && this=("${reversed[@]}")
    for spec in "${this[@]}"; do
      label="${spec%%=*}"
      env="${spec#*=}"
      echo "capacity: round $round, arm $label"
      CAPACITY_SERVER_ENV="$env" restore
      CAPACITY_SERVER_ENV="$env" step_run "$what" "$rate" "$(step_dir "$what" "$rate" "$label")/run-$round"
    done
  done
  for spec in "${order[@]}"; do
    step_report "$(step_dir "$what" "$rate" "${spec%%=*}")" "${spec%%=*}" || status=1
  done
  return "$status"
}

# soak <group|mix> <rate> <seconds>: one rate held for <seconds> while each
# instance's resident set and copies are sampled, as the server reports
# them; exits 0 when the step passed and no instance's resident set kept
# rising after the warm-up and the settling beyond the tolerance, 1 when one
# did, 2 when the soak is too short to fit.
soak() {
  local what="$1" rate="$2" seconds="$3" dir status=0 from started
  [[ "$seconds" =~ ^[1-9][0-9]*$ ]] || cap_die "a soak's length is a count of seconds"
  ((seconds >= SOAK_SETTLE + 3 * SOAK_INTERVAL)) \
    || cap_die "a soak of $seconds s leaves fewer than three samples after the $SOAK_SETTLE s settling (CAPACITY_SOAK_SETTLE)"
  dir="$CAP_OUT_ABS/soaks/$what-$rate"
  mkdir -p "$dir"
  rm -f "$dir/soak.tsv" "$dir/soak.stop"
  soak_sample "$dir/soak.tsv" "$dir/soak.stop" "$SOAK_INTERVAL" &
  local sampler=$!
  started=$(date +%s)
  # The plan's idle phase (5 s and 2 s settling) runs before the load.
  from=$((started + 7 + WARMUP + SOAK_SETTLE))
  CAPACITY_DURATION="$seconds" CAPACITY_ARM=soak CAPACITY_REPEAT=1 CAPACITY_FRESH=0 \
    "$BASH" "$CAPACITY_ROOT/scripts/perf/capacity.sh" step "$what" "$rate" || status=$?
  touch "$dir/soak.stop"
  wait "$sampler" || true
  soak_json "$dir/soak.tsv" "$from" "$SOAK_MAX_MIB_PER_HOUR" >"$dir/soak.json"
  ((status <= 1)) || cap_die "the soak's step could not be measured (exit $status)"
  jq -c --arg what "$what" --argjson rate "$rate" --argjson seconds "$seconds" \
    --argjson settle "$SOAK_SETTLE" --argjson step_passed "$([[ "$status" == 0 ]] && echo true || echo false)" \
    '{arm: "soak", what: $what, rate: $rate, seconds: $seconds, settle_s: $settle, step_passed: $step_passed,
      memory: ., passed: ($step_passed and .passed == true)}' "$dir/soak.json" | tee -a "$CAP_OUT_ABS/soaks.jsonl" \
    | jq -r '"capacity: soak of \(.what) at \(.rate) rps for \(.seconds) s: \(if .passed then "PASS" else "FAIL" end); RSS slope MiB per hour " + ([.memory.instances[] | "instance \(.instance) \(.slope_mib_per_hour)"] | join(", ")) + " against \(.memory.max_mib_per_hour)"'
  [[ "$(jq -r .passed "$dir/soak.json")" != null ]] || return 2
  [[ "$status" == 0 && "$(jq -r .passed "$dir/soak.json")" == true ]]
}

# profile <group|mix> <rate> <seconds>: one rate held while an external
# sampler profiles the first instance: `sample` on macOS, `perf record` on
# Linux. The server's own /management/flamegraph is not used: its unwinder
# can stop the server inside the Ed25519 signature (#5723).
profile() {
  local what="$1" rate="$2" seconds="$3" first dir pid
  read -r first _ <<<"$(state_get servers)"
  [[ "$first" == pid:* ]] || cap_die "profile needs the native topology (a server pid)"
  pid="${first#pid:}"
  dir="$CAP_OUT_ABS/profiles/$what-$rate"
  mkdir -p "$dir"
  (
    sleep "$((WARMUP + 10))"
    if command -v sample >/dev/null; then
      sample "$pid" "$seconds" -file "$dir/sample.txt" >/dev/null 2>&1 || true
    elif command -v perf >/dev/null; then
      perf record -F 999 -g -p "$pid" -o "$dir/perf.data" -- sleep "$seconds" >/dev/null 2>&1 || true
    fi
  ) &
  local taker=$!
  "$BASH" "$CAPACITY_ROOT/scripts/perf/capacity.sh" step "$what" "$rate" || true
  wait "$taker" || true
  echo "capacity: the profile is in $dir"
}

[[ "$#" -ge 1 ]] || cap_usage
cap_command="$1"
shift
case "$cap_command" in
up)
  [[ "$#" -eq 1 && "$1" =~ ^[1-8]$ ]] || cap_usage
  case "$TOPOLOGY" in
  native) up_native "$1" ;;
  docker) up_docker "$1" ;;
  *) cap_die "CAPACITY_TOPOLOGY must be native or docker" ;;
  esac
  ;;
seed) seed_capacity ;;
snapshot) snapshot ;;
restore) restore ;;
generator) generator_install ;;
sweep | step | arms | soak | profile)
  case "$GENERATOR" in
  host) ;;
  network) generator_check ;;
  *) cap_die "CAPACITY_GENERATOR must be host or network" ;;
  esac
  case "$cap_command" in
  sweep) [[ "$#" -ge 1 ]] || cap_usage; sweep "$@" ;;
  step) [[ "$#" -eq 2 ]] || cap_usage; step "$@" ;;
  arms) [[ "$#" -ge 4 ]] || cap_usage; arms "$@" ;;
  soak) [[ "$#" -eq 3 ]] || cap_usage; soak "$@" ;;
  profile) [[ "$#" -eq 3 ]] || cap_usage; profile "$@" ;;
  esac
  ;;
summary) summary ;;
compare) [[ "$#" -eq 2 ]] || cap_usage; compare "$@" ;;
down) down ;;
groups) printf '%s\n' "$GROUPS_TABLE" ;;
*) cap_usage ;;
esac
