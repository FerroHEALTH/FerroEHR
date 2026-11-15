#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The observability probe family (#2162, #2173, #2175).
#
# The far end here is deliberately awkward to reach, and that is the point. The
# defects this family nets were both invisible from the server side:
#
#   #2173 — the bundled Prometheus scraped NOTHING; the server was emitting
#           happily and no metric reached the stack.
#   #2175 — `metrics_push` exported 4 of 10 families: `ferroehr_build_info` and
#           the HTTP histogram never left the process.
#
# In both, `/management/prometheus` answered 200 with the metrics present. Any
# probe that asked the SERVER whether telemetry worked would have passed. So
# these ask the COLLECTOR.
#
# The overlay publishes only Grafana; Prometheus and Tempo stay on the compose
# network, so the queries go through Grafana's datasource proxy — which is also
# how an operator would check, making this the documented path rather than a
# back door.
#
# Sourced by scripts/deploy-probe.sh; never run directly.

GRAFANA="http://localhost:${PROBE_GRAFANA_PORT:-13000}"
GRAFANA_AUTH="admin:admin"

obs_up() {
  GRAFANA_PORT="${PROBE_GRAFANA_PORT:-13000}" \
    dc -f docker-compose.yml -f docker-compose.observability.yml \
       up -d ferroehr otel-lgtm >/dev/null 2>&1
}

# Back to the shipped stack for the families that follow.
obs_down() {
  dc -f docker-compose.yml up -d ferroehr >/dev/null 2>&1
  wait_http "$CDR/health/readiness" 150 || true
}

# The datasource uid for a given type, discovered rather than assumed: the
# bundled stack provisions its own and the names are not ours to pin.
obs_datasource_uid() {
  local ds_type="$1"
  curl -s -u "$GRAFANA_AUTH" "$GRAFANA/api/datasources" \
    | jq -r --arg t "$ds_type" 'map(select(.type == $t)) | .[0].uid // empty' 2>/dev/null
}

# Instant-query Prometheus THROUGH Grafana, returning the number of series.
obs_promql_series() {
  local uid="$1" query="$2"
  curl -s -u "$GRAFANA_AUTH" --get --data-urlencode "query=$query" \
    "$GRAFANA/api/datasources/proxy/uid/$uid/api/v1/query" \
    | jq -r '.data.result | length' 2>/dev/null
}

# Poll until a query yields at least one series, or the deadline passes.
#
# The far end here is EVENTUALLY consistent — the exporter batches on its own
# interval and the collector ingests on its own — so a fixed sleep is a race
# with two clocks nobody controls. The first version of this family used one and
# reported three false failures against a server that was exporting correctly.
# Echoes the count so a real absence still reports what it saw.
obs_wait_series() {
  local uid="$1" query="$2" tries="${3:-30}" n
  for _ in $(seq 1 "$tries"); do
    n="$(obs_promql_series "$uid" "$query")"
    if [[ "${n:-0}" -ge 1 ]] 2>/dev/null; then printf '%s' "$n"; return 0; fi
    sleep 4
  done
  printf '%s' "${n:-0}"
  return 1
}

# The same shape for the trace store.
obs_wait_traces() {
  local uid="$1" tries="${2:-30}" n
  for _ in $(seq 1 "$tries"); do
    n="$(curl -s -u "$GRAFANA_AUTH" --get \
      --data-urlencode 'q={ resource.service.name="ferroehr" }' \
      "$GRAFANA/api/datasources/proxy/uid/$uid/api/search" \
      | jq -r '.traces | length' 2>/dev/null)"
    if [[ "${n:-0}" -ge 1 ]] 2>/dev/null; then printf '%s' "$n"; return 0; fi
    sleep 4
  done
  printf '%s' "${n:-0}"
  return 1
}

# TODO(#5662): temporary timeline sampler, removed before the fix merges.
obs_diag_value() {
  local uid="$1" query="$2"
  curl -s -m 3 -u "$GRAFANA_AUTH" --get --data-urlencode "query=$query" \
    "$GRAFANA/api/datasources/proxy/uid/$uid/api/v1/query" \
    | jq -r '[.data.result[].value[1] | tonumber] | add // "none"' 2>/dev/null
}
obs_diag_line() {
  local prom tempo acc ref sent fail recv live search
  prom="$(obs_datasource_uid prometheus)"; tempo="$(obs_datasource_uid tempo)"
  if [[ -n "$prom" ]]; then
    acc="$(obs_diag_value "$prom" '{__name__=~"otelcol_receiver_accepted_spans.*"}')"
    ref="$(obs_diag_value "$prom" '{__name__=~"otelcol_receiver_refused_spans.*"}')"
    sent="$(obs_diag_value "$prom" '{__name__=~"otelcol_exporter_sent_spans.*"}')"
    fail="$(obs_diag_value "$prom" '{__name__=~"otelcol_exporter_send_failed_spans.*"}')"
  fi
  if [[ -n "$tempo" ]]; then
    local m
    m="$(curl -s -m 3 -u "$GRAFANA_AUTH" "$GRAFANA/api/datasources/proxy/uid/$tempo/metrics")"
    recv="$(grep -E '^tempo_distributor_spans_received_total' <<<"$m" | awk '{print $2}')"
    live="$(grep -E '^tempo_live_store_traces_created_total' <<<"$m" | awk '{print $2}')"
    search="$(curl -s -m 3 -u "$GRAFANA_AUTH" --get \
      --data-urlencode 'q={ resource.service.name="ferroehr" }' \
      "$GRAFANA/api/datasources/proxy/uid/$tempo/api/search" | jq -r '.traces | length' 2>/dev/null)"
  fi
  printf 'diag %s accepted=%s refused=%s sent=%s failed=%s tempo_received=%s live_created=%s search=%s\n' \
    "$(date -u +%T)" "${acc:--}" "${ref:--}" "${sent:--}" "${fail:--}" "${recv:--}" "${live:--}" "${search:--}"
}
obs_diag_start() {
  ( while :; do obs_diag_line; sleep 5; done ) >"$PROBE_TMP/obs-diag.txt" 2>&1 &
  OBS_DIAG_PID=$!
}
obs_diag_stop() {
  kill "$OBS_DIAG_PID" 2>/dev/null
  wait "$OBS_DIAG_PID" 2>/dev/null
  dim "    obs-diag timeline:"
  sed 's/^/      /' "$PROBE_TMP/obs-diag.txt"
  local c
  for c in ferroehr otel-lgtm; do
    dim "    obs-diag $c started: $(docker inspect -f '{{.State.StartedAt}}' "$(dc ps -q "$c")" 2>&1)"
  done
  dim "    obs-diag otel-lgtm log:"
  dc -f docker-compose.yml -f docker-compose.observability.yml logs --no-color --timestamps otel-lgtm 2>&1 \
    | grep -i -E 'up and running|startup time|error|warn|drop|refus|fail' | tail -80 | sed 's/^/      /'
  dim "    obs-diag traces in the store:"
  local tuid
  tuid="$(obs_datasource_uid tempo)"
  curl -s -u "$GRAFANA_AUTH" --get --data-urlencode 'q={ resource.service.name="ferroehr" }' \
    --data-urlencode 'limit=100' "$GRAFANA/api/datasources/proxy/uid/$tuid/api/search" \
    | jq -r '.traces[] | "\(.startTimeUnixNano | tonumber / 1e9 | todate) \(.rootTraceName)"' 2>&1 | sort | sed 's/^/      /'
  dim "    obs-diag ferroehr log (request lines):"
  dc -f docker-compose.yml -f docker-compose.observability.yml logs --no-color --timestamps ferroehr 2>&1 \
    | grep -v -i -E 'password|token|secret' | cut -c1-400 | tail -80 | sed 's/^/      /'
}

probes_observability() {
  observability_family
  obs_down
}

observability_family() {
  bold "observability (traces + metrics at the collector)"

  dim "    obs-diag up at $(date -u +%T)"
  obs_up
  dim "    obs-diag up returned at $(date -u +%T)"
  obs_diag_start
  if ! wait_http "$CDR/health/readiness" 150; then
    probe "P-OBS-UP" "$PROBE_WORKING" "compose" "-" "the observability overlay starts"
    probe_fail "a serving CDR under the observability overlay" "readiness never returned"
    probe_done
    return 0
  fi
  dim "    obs-diag ready at $(date -u +%T)"
  # Grafana takes a while to come up inside the bundle.
  wait_http "$GRAFANA/api/health" 120 || true
  dim "    obs-diag grafana at $(date -u +%T)"

  # Give the server something to report. How long it takes to ARRIVE is the
  # pipeline's business, so each probe below waits on its own observation
  # rather than this loop guessing an interval.
  local _n
  for _n in $(seq 1 5); do
    curl -s -o /dev/null -u "$BASIC" -X POST "$API/ehr" || true
  done
  dim "    obs-diag posts done at $(date -u +%T)"

  local prom_uid
  prom_uid="$(obs_datasource_uid prometheus)"

  # #2173: metrics reaching the stack AT ALL. The server answering 200 on its
  # own scrape endpoint proved nothing — that was true throughout the defect.
  probe "P-OBS-METRICS" "$PROBE_WORKING" "server" "#2173" \
    "FerroEHR metrics arrive in the collector's Prometheus"
  if [[ -z "$prom_uid" ]]; then
    probe_fail "a Prometheus datasource in Grafana" "none found" \
      "the bundled stack provisions it; without it nothing here can be observed"
  else
    local http_series
    if ! http_series="$(obs_wait_series "$prom_uid" '{__name__=~"http_server_request_duration.*"}')"; then
      probe_fail "at least one http_server_request_duration series" "${http_series:-0} after 120s" \
        "the HTTP histogram is one of the families #2175 found never leaving the process"
    fi
  fi
  probe_done

  # #2175: the specific family that went missing while others arrived. A probe
  # asserting "some metrics arrived" would have passed throughout that defect,
  # so this one names the family.
  probe "P-OBS-BUILDINFO" "$PROBE_WORKING" "server" "#2175" \
    "ferroehr_build_info reaches the collector, not just the local scrape"
  if [[ -z "$prom_uid" ]]; then
    probe_fail "a Prometheus datasource" "none found"
  else
    local bi
    if ! bi="$(obs_wait_series "$prom_uid" 'ferroehr_build_info')"; then
      probe_fail "a ferroehr_build_info series in the collector" "${bi:-0} after 120s" \
        "it is emitted locally; the defect was that it never left the process"
    fi
  fi
  probe_done

  # A span in the collector — the trace half, which no server-side check can
  # confirm either.
  probe "P-OBS-TRACES" "$PROBE_WORKING" "server" "-" \
    "a span for this service arrives in the collector's trace store"
  local tempo_uid found
  tempo_uid="$(obs_datasource_uid tempo)"
  if [[ -z "$tempo_uid" ]]; then
    # Not a failure of the SUT: without a queryable trace store this run cannot
    # observe the far end, and saying so is the honest outcome. It is declared
    # as a gap rather than passed silently.
    dim "    SKIP  no tempo datasource exposed — declared as not exercised"
    uncovered "traces in the collector" \
      "the bundled stack exposed no tempo datasource to query through Grafana"
    return 0
  fi
  if ! found="$(obs_wait_traces "$tempo_uid")"; then
    probe_fail "at least one trace tagged service.name=ferroehr" "${found:-0} after 120s" \
      "an OTLP exporter that cannot reach its collector fails silently, so this is the only place it shows"
  fi
  probe_done
  sleep 60
  obs_diag_stop
}
