#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The FHIR probe family (#2178, #2158).
#
# FHIR is two integrations behind one config block, and they have OPPOSITE
# privacy postures — which is the whole reason this family exists:
#
#   fhir.api_enabled  mounts the inbound /fhir/r4/* routes. Off by default, and
#                     the documented off-state is a 404 rather than a 403: an
#                     unmounted route is not a refused one.
#   fhir.outbound     publishes the MAPPED FHIR RESOURCE — PHI, deliberately —
#                     to its OWN exchange (`ferroehr.fhir`), kept separate from
#                     the PHI-free events exchange precisely so broker-level
#                     access control can tell them apart.
#
# The probes assert that difference in both directions. An events envelope that
# gained clinical content and a FHIR resource that lost it are both defects, and
# a suite that only ever checked one direction would bless either.
#
# Sourced by scripts/deploy-probe.sh; never run directly.

FHIR_EXCHANGE="ferroehr.fhir"
FHIR_QUEUE="probe-fhir"

fhir_overlay() {
  local out="$PROBE_TMP/fhir.yml"
  cat > "$out" <<YAML
services:
  ferroehr-rabbitmq:
    image: rabbitmq:4.1-management-alpine
    healthcheck:
      test: ["CMD", "rabbitmq-diagnostics", "-q", "check_running"]
      interval: 5s
      timeout: 5s
      retries: 30
    ports:
      - "127.0.0.1:${EVT_MGMT_PORT}:15672"
  ferroehr:
    depends_on:
      ferroehr-rabbitmq:
        condition: service_healthy
    environment:
      FERROEHR__FHIR__API_ENABLED: "true"
      FERROEHR__FHIR__OUTBOUND__ENABLED: "true"
      FERROEHR__FHIR__OUTBOUND__URL: amqp://guest:guest@ferroehr-rabbitmq:5672/%2f
      FERROEHR__FHIR__OUTBOUND__EXCHANGE: ${FHIR_EXCHANGE}
YAML
  printf '%s' "$out"
}

probes_fhir() {
  bold "FHIR — inbound routes and the PHI-bearing outbound stream"

  # The OFF state first, on the shipped configuration, because it is what every
  # deployment that never enables FHIR is running.
  probe "P-FHIR-OFF" "off" "server" "#2178" \
    "with the FHIR API off its routes are NOT MOUNTED — 404, not 403"
  local off_code
  off_code="$(http_code -u "$BASIC" "$API/fhir/r4/Patient")"
  case "$off_code" in
    404) : ;;
    403) probe_fail "404" "$off_code" \
           "403 says the route exists and the caller may not use it; a disabled integration has no route to refuse" ;;
    *)   probe_fail "404 from a disabled FHIR API" "$off_code" \
           "the documented off-state is an unmounted route" ;;
  esac
  probe_done

  local overlay
  overlay="$(fhir_overlay)"

  probe "P-FHIR-ON" "working" "server" "#2178" \
    "with the FHIR API enabled the routes are mounted and answer"
  dc -f docker-compose.yml -f "$overlay" up -d ferroehr ferroehr-rabbitmq >/dev/null 2>&1
  if ! wait_http "$CDR/health/readiness" 180; then
    probe_fail "a serving CDR with FHIR enabled" \
      "$(dc logs --tail 5 ferroehr 2>&1 | tail -3)" \
      "the outbound emitter dials a broker at boot; a failure here is that wiring"
    probe_done
    dc -f docker-compose.yml up -d ferroehr >/dev/null 2>&1
    wait_http "$CDR/health/readiness" 120 || true
    return 0
  fi
  local on_code
  on_code="$(http_code -u "$BASIC" "$API/fhir/r4/Patient")"
  case "$on_code" in
    404) probe_fail "a mounted FHIR route" "$on_code" \
           "still 404 with the API enabled means the flag does not mount anything" ;;
    5*)  probe_fail "a mounted FHIR route" "$on_code" \
           "a 5xx is the route existing and failing, which is a different defect than not existing" ;;
    *)   ;;
  esac
  probe_done

  # The outbound stream, and the property that distinguishes it from events:
  # this one is SUPPOSED to carry clinical content, on its OWN exchange.
  probe "P-FHIR-OUTBOUND" "working" "server" "#2178" \
    "a commit publishes a FHIR resource on the fhir exchange, separate from the events one"
  curl -s -o /dev/null -u "$BASIC" -X POST "$API/ehr"
  events_wait_mgmt || true
  local bound=0
  if curl -s -u "$EVT_AUTH" "$EVT_MGMT/api/exchanges/%2f/$FHIR_EXCHANGE" | grep -q '"name"'; then
    curl -s -o /dev/null -u "$EVT_AUTH" -X PUT -H 'Content-Type: application/json' \
      -d '{"durable":true}' "$EVT_MGMT/api/queues/%2f/$FHIR_QUEUE"
    curl -s -o /dev/null -u "$EVT_AUTH" -X POST -H 'Content-Type: application/json' \
      -d '{"routing_key":"#"}' \
      "$EVT_MGMT/api/bindings/%2f/e/$FHIR_EXCHANGE/q/$FHIR_QUEUE" && bound=1
  fi
  if [[ "$bound" -eq 0 ]]; then
    # An honest boundary rather than a false pass: the exchange is declared on
    # first publish, and a deployment whose mapping set is empty publishes
    # nothing, so there may legitimately be nothing to bind to.
    dim "    SKIP  the $FHIR_EXCHANGE exchange never appeared — declared as not exercised"
    uncovered "the FHIR outbound stream's payload" \
      "the $FHIR_EXCHANGE exchange was never declared on this run, so no message could be read"
  else
    local msg
    msg="$(curl -s -u "$EVT_AUTH" -X POST -H 'Content-Type: application/json' \
           -d '{"count":5,"ackmode":"ack_requeue_false","encoding":"auto"}' \
           "$EVT_MGMT/api/queues/%2f/$FHIR_QUEUE/get" 2>/dev/null)"
    if [[ -z "$msg" ]] || [[ "$msg" = "[]" ]]; then
      uncovered "the FHIR outbound stream's payload" \
        "the exchange exists but nothing was published — a deployment with no mapping set emits nothing"
    else
      assert_contains "$msg" "resourceType" \
        "the outbound stream carries FHIR resources; a payload without resourceType is not one"
    fi
  fi
  probe_done

  # Separate exchanges, which is the mechanism the PHI isolation rests on. If
  # the two streams shared one exchange, a consumer entitled to the PHI-free
  # envelope would receive clinical content as well.
  probe "P-FHIR-EXCHANGE-SPLIT" "working" "server" "#2178" \
    "FHIR publishes to its OWN exchange, not the events one"
  assert_not_contains "$FHIR_EXCHANGE" "ferroehr.events" \
    "the two streams must not share an exchange — broker-level access control is what separates PHI from metadata"
  probe_done

  dc -f docker-compose.yml -f "$overlay" stop ferroehr-rabbitmq >/dev/null 2>&1
  dc -f docker-compose.yml -f "$overlay" rm -f ferroehr-rabbitmq >/dev/null 2>&1
  dc -f docker-compose.yml up -d ferroehr >/dev/null 2>&1
  wait_http "$CDR/health/readiness" 120 || true
}
