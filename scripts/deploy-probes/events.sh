#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The events / AMQP outbox probe family (#2178, #2158).
#
# Two claims are made about this integration and neither had been demonstrated
# on a running system:
#
#   PHI-free  — the payload carries identity and provenance only: contribution
#               id, ehr_id, committed_at, and per version (vo_id, kind,
#               sys_version, change_type, template_id). Never clinical content.
#               That is a PRIVACY claim, so the probe reads the actual bytes off
#               the broker rather than trusting the type that built them.
#   never lose — the outbox is written inside the commit transaction and drained
#               afterwards, so a broker outage must not fail a commit and must
#               not drop the event. Testing that means taking the broker away
#               WHILE committing, which no unit test does.
#
# The far end is the broker: messages are read back through RabbitMQ's
# management API, because "the server logged that it published" is exactly the
# kind of evidence #2173 and #2175 proved worthless.
#
# Sourced by scripts/deploy-probe.sh; never run directly.

EVT_MGMT_PORT="${PROBE_AMQP_MGMT_PORT:-18672}"
EVT_MGMT="http://localhost:${EVT_MGMT_PORT}"
EVT_AUTH="guest:guest"
EVT_QUEUE="probe-events"
EVT_EXCHANGE="ferroehr.events"

# A broker + the server pointed at it. Generated rather than committed: nothing
# else in the quickstart needs a broker, and an overlay in the tree would invite
# someone to run it in production.
events_overlay() {
  local out="$PROBE_TMP/events.yml"
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
      FERROEHR__EVENTS__ENABLED: "true"
      FERROEHR__EVENTS__URL: amqp://guest:guest@ferroehr-rabbitmq:5672/%2f
      FERROEHR__EVENTS__EXCHANGE: ${EVT_EXCHANGE}
YAML
  printf '%s' "$out"
}

# Declare a queue bound to the exchange so the probe can read what was
# published. A topic exchange drops a message with no binding, so without this
# the probes below would measure nothing.
#
# The WAIT is the load-bearing part. The server declares the exchange when its
# publisher connects, not at boot, so binding immediately after readiness posts
# to an exchange that does not exist yet — RabbitMQ answers 404, and a binding
# that never existed looks exactly like an integration that never publishes.
# That produced three red rows blaming the server for the probe's own race.
# Returns non-zero if the binding could not be made, so a caller can say so
# rather than measure nothing and call it a finding.
events_bind_queue() {
  local tries=40
  while [[ "$tries" -gt 0 ]]; do
    curl -s -u "$EVT_AUTH" "$EVT_MGMT/api/exchanges/%2f/$EVT_EXCHANGE" \
      | grep -q '"name"' && break
    tries=$((tries - 1))
    sleep 3
  done
  [[ "$tries" -gt 0 ]] || return 1
  curl -s -o /dev/null -u "$EVT_AUTH" -X PUT -H 'Content-Type: application/json' \
    -d '{"durable":true}' "$EVT_MGMT/api/queues/%2f/$EVT_QUEUE"
  local code
  code="$(curl -s -o /dev/null -w '%{http_code}' -u "$EVT_AUTH" -X POST \
          -H 'Content-Type: application/json' -d '{"routing_key":"#"}' \
          "$EVT_MGMT/api/bindings/%2f/e/$EVT_EXCHANGE/q/$EVT_QUEUE")"
  case "$code" in 201|204) return 0 ;; *) return 1 ;; esac
}

# Wait for the management plugin, on a path that does NOT require credentials.
# `wait_http` sends none, and every /api/* route answers 401 without them — so
# waiting on /api/overview reports "never came up" about a broker that is
# serving perfectly. That cost one run.
events_wait_mgmt() {
  local tries="${1:-40}"
  for _ in $(seq 1 "$tries"); do
    [[ "$(curl -s -o /dev/null -w '%{http_code}' -u "$EVT_AUTH" "$EVT_MGMT/api/overview")" = "200" ]] && return 0
    sleep 3
  done
  return 1
}

# Drain up to N messages, echoing the payloads. `ack_requeue_false` consumes
# them so a later probe cannot re-read an earlier probe's message and call it
# fresh delivery.
events_get() {
  curl -s -u "$EVT_AUTH" -X POST -H 'Content-Type: application/json' \
    -d "{\"count\":${1:-10},\"ackmode\":\"ack_requeue_false\",\"encoding\":\"auto\"}" \
    "$EVT_MGMT/api/queues/%2f/$EVT_QUEUE/get" 2>/dev/null
}

# Wait for at least one message rather than sleeping: the outbox drains on its
# own poll interval, which is not this probe's to guess.
events_wait_message() {
  local tries="${1:-30}" body
  for _ in $(seq 1 "$tries"); do
    body="$(events_get 10)"
    if [[ -n "$body" ]] && [[ "$body" != "[]" ]]; then printf '%s' "$body"; return 0; fi
    sleep 3
  done
  printf '%s' "${body:-[]}"
  return 1
}

probes_events() {
  bold "events — the AMQP outbox"

  local overlay
  overlay="$(events_overlay)"

  probe "P-EVT-BOOT" "working" "compose" "#2178" \
    "the server boots against a broker with events enabled"
  dc -f docker-compose.yml -f "$overlay" up -d ferroehr ferroehr-rabbitmq >/dev/null 2>&1
  if ! wait_http "$CDR/health/readiness" 180; then
    probe_fail "a serving CDR with events enabled" \
      "$(dc logs --tail 5 ferroehr 2>&1 | tail -3)" \
      "a broker the server cannot reach must not stop it serving; a failure here is the wiring"
    probe_done
    dc -f docker-compose.yml up -d ferroehr >/dev/null 2>&1
    wait_http "$CDR/health/readiness" 120 || true
    return 0
  fi
  # A warm-up commit BEFORE binding. The exchange is declared by the publisher
  # on its first publish, not when the process starts — verified directly: right
  # after readiness the broker lists no `ferroehr.events`, and it appears once
  # something has been committed. Binding first therefore posts to an exchange
  # that does not exist, RabbitMQ answers 404, and every later probe reads an
  # empty queue and blames the server.
  curl -s -o /dev/null -u "$BASIC" -X POST "$API/ehr"
  events_wait_mgmt || true
  if ! events_bind_queue; then
    probe_fail "a queue bound to $EVT_EXCHANGE" "the exchange never appeared, or the binding was refused" \
      "without a binding a topic exchange drops every message and the probes below would measure nothing"
  fi
  probe_done

  # A commit, then the message on the broker. Reading it back is the point:
  # the server logging a publish proved nothing in #2173 or #2175 either.
  probe "P-EVT-PUBLISH" "working" "server" "#2178" \
    "a commit produces a message on the exchange, read back from the broker"
  local ehr body
  ehr="$(curl -s -u "$BASIC" -X POST -D - -o /dev/null "$API/ehr" \
         | grep -i '^location' | tr -d '\r' | awk -F/ '{print $NF}')"
  if [[ -z "$ehr" ]]; then
    probe_fail "a committed EHR" "no id returned"
    probe_done
  else
    if ! body="$(events_wait_message)"; then
      probe_fail "at least one message on $EVT_QUEUE" "queue empty after 90s" \
        "the outbox row is written in the commit transaction, so an empty queue is the drain or the broker wiring"
    else
      assert_contains "$body" "$ehr" \
        "the envelope must identify the EHR the commit touched, or a consumer cannot attribute it"
    fi
    probe_done

    # The privacy claim, checked against the bytes actually on the wire.
    probe "P-EVT-NO-PHI" "working" "server" "#2178" \
      "the published envelope carries identity and provenance only — no clinical content"
    if [[ -z "${body:-}" ]] || [[ "${body:-[]}" = "[]" ]]; then
      probe_fail "a message to inspect" "none captured" \
        "without the payload the PHI-free claim cannot be checked at all"
    else
      # The envelope's own vocabulary must be there...
      assert_contains "$body" "contribution_id" \
        "an envelope without its contribution id is not the documented shape"
      # ...and the shapes that would mean clinical content leaked must not.
      # These are RM type discriminators: their presence in a payload that is
      # supposed to be metadata means a composition rode along.
      for leaked in '_type\":\"COMPOSITION' 'DV_TEXT' 'archetype_details' 'other_details'; do
        assert_not_contains "$body" "$leaked" \
          "clinical content in the event stream defeats the PHI-free claim the integration is built on"
      done
    fi
    probe_done
  fi

  # The outbox exists for exactly this: the broker goes away and commits must
  # neither fail nor lose the event.
  probe "P-EVT-BROKER-DOWN" "broken" "server" "#2178" \
    "with the broker stopped a commit still succeeds — the outbox absorbs it"
  dc stop ferroehr-rabbitmq >/dev/null 2>&1
  local queued
  queued="$(curl -s -u "$BASIC" -X POST -D - -o /dev/null "$API/ehr" \
            | grep -i '^location' | tr -d '\r' | awk -F/ '{print $NF}')"
  if [[ -z "$queued" ]]; then
    probe_fail "a commit that succeeds with the broker down" "no EHR id returned" \
      "commits must never block on the broker; that is what the outbox is for"
  fi
  probe_done

  # ...and the event is not lost. This is the half that a unit test cannot
  # reach, and the reason the family exists.
  probe "P-EVT-DRAIN" "working" "server" "#2178" \
    "when the broker returns, the event committed during the outage is delivered"
  dc start ferroehr-rabbitmq >/dev/null 2>&1
  events_wait_mgmt || true
  events_bind_queue || true
  if [[ -n "$queued" ]]; then
    local after
    if ! after="$(events_wait_message 40)"; then
      probe_fail "the queued event delivered after recovery" "queue still empty after 120s" \
        "an outbox that does not drain on reconnect loses every event of the outage"
    else
      assert_contains "$after" "$queued" \
        "the event committed while the broker was down must arrive, or the outbox guarantee is not real"
    fi
  fi
  probe_done

  # Back to the shipped posture: events off, no broker.
  dc -f docker-compose.yml -f "$overlay" stop ferroehr-rabbitmq >/dev/null 2>&1
  dc -f docker-compose.yml -f "$overlay" rm -f ferroehr-rabbitmq >/dev/null 2>&1
  dc -f docker-compose.yml up -d ferroehr >/dev/null 2>&1
  wait_http "$CDR/health/readiness" 120 || true

  probe "P-EVT-OFF" "off" "server" "#2178" \
    "with events off the server serves normally and attempts no broker connection"
  assert_eq "200" "$(http_code "$CDR/health/readiness")" \
    "events are opt-in; the default deployment must be unaffected by their absence"
  assert_not_contains "$(dc logs --tail 40 ferroehr 2>&1)" "amqp://" \
    "a disabled integration must not be dialling a broker"
  probe_done
}
