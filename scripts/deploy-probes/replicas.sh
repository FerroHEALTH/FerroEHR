#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The two-instance probe family (#5589).
#
# Any number of FerroEHR instances serve one deployment behind a load
# balancer, on one PostgreSQL, with no sticky sessions. This family brings up
# the quickstart with docker-compose.replicas.yml, a second instance on port
# 8081, both signing with one generated OpenPGP key, and observes at the far
# end: a record written through one instance reads through the other, writes
# sent to both at once all land, the access log and the system chain each
# instance appended to verify with their checkpoint and seal, and an instance
# started with another key is named in the other's health.
#
# Sourced by scripts/deploy-probe.sh after signing_pgp.sh, whose key
# generation it reuses; never run directly.

# The second instance's port, shifted off the overlay's default as the
# family's other ports are.
export FERROEHR_REPLICA_PORT="${PROBE_REPLICA_PORT:-18081}"
SECOND_CDR="http://localhost:${FERROEHR_REPLICA_PORT}"
SECOND_API="$SECOND_CDR/ferroehr/rest/openehr/v1"

# An overlay mounting the key at `dir` into the instances `services` and
# switching each to pgp mode, written to `out`.
replica_key_overlay() {
  local dir="$1" out="$2"
  shift 2
  printf 'services:\n' > "$out"
  local service
  for service in "$@"; do
    cat >> "$out" <<YAML
  $service:
    volumes:
      - $dir/signing.asc:/etc/ferroehr/signing.asc:ro
      - $dir/passphrase:/run/secrets/pgp-pass:ro
    environment:
      FERROEHR__RECORD__SIGNING__ENABLED: "true"
      FERROEHR__RECORD__SIGNING__MODE: pgp
      FERROEHR__RECORD__SIGNING__KEY_FILE: /etc/ferroehr/signing.asc
      FERROEHR__RECORD__SIGNING__KEY_PASSPHRASE_FILE: /run/secrets/pgp-pass
YAML
  done
  printf '%s' "$out"
}

# The state of the indicator `name` on the readiness document of `base`.
indicator_state() {
  curl -s "$1/health/readiness" | jq -r --arg name "$2" \
    '.indicators[]? | select(.name == $name) | .state' 2>/dev/null
}

# The id of an EHR created through the API at `api`.
replica_ehr() {
  curl -s -u "$BASIC" -X POST -D - -o /dev/null "$1/ehr" \
    | grep -i '^location' | tr -d '\r' | awk -F/ '{print $NF}'
}

probes_replicas() {
  bold "Two instances on one PostgreSQL (#5589)"

  if ! command -v gpg >/dev/null 2>&1 || ! command -v jq >/dev/null 2>&1; then
    uncovered "two instances (#5589)" "gpg or jq is not available on this host"
    return 0
  fi
  local dir
  if ! dir="$(pgp_material replicas)" || [[ -z "$dir" ]]; then
    uncovered "two instances (#5589)" "this host's gpg would not generate a probe key"
    return 0
  fi
  local overlay
  overlay="$(replica_key_overlay "$dir" "$PROBE_TMP/replicas-key.yml" ferroehr ferroehr-2)"
  local -a files=(-f docker-compose.yml -f docker-compose.replicas.yml -f "$overlay")

  probe "P-REPLICA-BOOT" "$PROBE_WORKING" "deployment" "#5589" \
    "a second instance on the same database and key boots, and both report the instances agree"
  # The overlay publishes the second instance's port on the identity issuer,
  # so Compose recreates the issuer, and every service in its network
  # namespace must be recreated with it: FerroPIX left in the old namespace is
  # unreachable on localhost, and FerroEHR refuses to boot without it.
  dc "${files[@]}" --profile replicas --profile s3 up -d --wait --always-recreate-deps \
    ferroehr ferroehr-2 >/dev/null 2>&1
  if ! wait_http "$SECOND_CDR/health/readiness" 90 || ! wait_http "$CDR/health/readiness" 90; then
    probe_fail "two serving instances" "$(replica_state "${files[@]}")"
    probe_done
    replicas_down
    return 0
  fi
  assert_eq "up" "$(indicator_state "$SECOND_CDR" instances)" \
    "the second instance compared itself with the first at start"
  probe_done

  probe "P-REPLICA-SHARED" "$PROBE_WORKING" "server" "#5589" \
    "a record written through one instance reads through the other, with no sticky session"
  local ehr
  ehr="$(replica_ehr "$API")"
  if [[ -z "$ehr" ]]; then
    probe_fail "an EHR committed through the first instance" "no id returned"
  else
    assert_eq "200" "$(http_code -u "$BASIC" "$SECOND_API/ehr/$ehr")" \
      "the second instance reads what the first committed"
    assert_eq "200" "$(http_code -u "$BASIC" "$SECOND_API/ehr/$ehr/ehr_status")"
  fi
  probe_done

  probe "P-REPLICA-CONCURRENT" "$PROBE_WORKING" "server" "#5589" \
    "writes sent to both instances at once all commit"
  local codes="$PROBE_TMP/replica-codes"
  : > "$codes"
  local i api
  local -a writers=()
  for i in $(seq 1 20); do
    api="$API"
    [[ $((i % 2)) -eq 0 ]] && api="$SECOND_API"
    curl -s -u "$BASIC" -X POST -o /dev/null -w '%{http_code}\n' "$api/ehr" >> "$codes" &
    writers+=("$!")
  done
  for i in "${writers[@]}"; do
    wait "$i" || true
  done
  assert_eq "20" "$(grep -c '^201$' "$codes")" \
    "every POST /ehr of either instance commits: $(sort "$codes" | uniq -c | tr '\n' ' ')"
  probe_done

  probe "P-REPLICA-CHAINS" "$PROBE_WORKING" "server" "#5589" \
    "the access log and the system chain both instances appended to verify with their checkpoint and seal"
  local out
  # The drain writes each access's entry after the response; give it a moment.
  sleep 3
  dc "${files[@]}" exec -T ferroehr-2 /usr/local/bin/ferroehr access-log checkpoint >/dev/null 2>&1
  out="$(dc "${files[@]}" exec -T ferroehr /usr/local/bin/ferroehr access-log verify 2>&1)"
  assert_contains "$out" "the access log verifies" \
    "a checkpoint sealed by one instance verifies the links both signed"
  dc "${files[@]}" exec -T ferroehr /usr/local/bin/ferroehr audit-chain seal >/dev/null 2>&1
  out="$(dc "${files[@]}" exec -T ferroehr-2 /usr/local/bin/ferroehr audit-chain verify 2>&1)"
  assert_contains "$out" "the system chain verifies" \
    "the system chain verifies against the other instance's seal"
  probe_done

  probe "P-REPLICA-KEY-MISMATCH" "$PROBE_BROKEN" "deployment" "#5589" \
    "an instance started with another signing key is named in health from the start"
  local other
  if ! other="$(pgp_material replicas-other)" || [[ -z "$other" ]]; then
    probe_fail "a second probe key" "gpg refused"
  else
    local mismatch
    mismatch="$(replica_key_overlay "$other" "$PROBE_TMP/replicas-other.yml" ferroehr-2)"
    dc "${files[@]}" -f "$mismatch" --profile replicas --profile s3 up -d --wait \
      ferroehr-2 >/dev/null 2>&1
    wait_http "$SECOND_CDR/health/readiness" 90 || true
    assert_eq "degraded" "$(indicator_state "$SECOND_CDR" instances)" \
      "the instance with another key compares itself with the first at start"
    assert_contains "$(curl -s -u "$BASIC" "$SECOND_CDR/management/health")" "another signing key" \
      "the management view says what differs"
  fi
  probe_done

  replicas_down
}

# What a failed boot leaves to read: each container's state and the last
# lines of each instance's and FerroPIX's log, for the compose `files`.
replica_state() {
  local service
  dc "$@" --profile replicas ps -a --format '{{.Service}} {{.State}} {{.Status}}' 2>&1 | tr '\n' ';'
  for service in ferroehr ferroehr-2 ferropix; do
    printf ' %s: ' "$service"
    dc "$@" --profile replicas logs --no-color --tail 3 "$service" 2>&1 | tr '\n' ' '
  done
}

# Stop the second instance and bring the quickstart stack back as the other
# families expect it: the issuer drops the second port, so it is recreated
# with every service in its namespace again.
replicas_down() {
  dc -f docker-compose.yml -f docker-compose.replicas.yml --profile replicas \
    rm -sf ferroehr-2 >/dev/null 2>&1 || true
  dc -f docker-compose.yml --profile s3 up -d --wait --always-recreate-deps \
    ferroehr seaweedfs seaweedfs-init ferroehr-provision >/dev/null 2>&1
  wait_http "$CDR/health/readiness" 90 || true
}
