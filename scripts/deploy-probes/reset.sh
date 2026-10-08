#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The original-state family: the book's reset procedure for the Compose
# quickstart (website/book/src/operations-reset.md), run as written.
#
# CRA Annex I Part I(2)(b) asks for "the possibility to reset the product to
# its original state" (docs/law/eu/cra/text.html). A documented procedure
# nobody runs is a claim, so this family runs it and reads the outcome at the
# far end: the volume itself, the API answering for data written before the
# reset, and the server's own effective configuration.
#
# Destructive by design: it deletes the stack's volumes. Run it last.
#
# Sourced by scripts/deploy-probe.sh; never run directly.

# The value of `key` in the `[table]` of a TOML document, or at the top level
# when `table` is empty. Only what `ferroehr config check` prints is parsed:
# `key = value` lines under `[table]` headers.
toml_value() {
  local doc="$1" table="$2" key="$3"
  printf '%s\n' "$doc" | awk -v table="$table" -v key="$key" '
    /^\[/ { current = $0; gsub(/^\[|\]$/, "", current); next }
    current == table && $1 == key && $2 == "=" {
      value = $0; sub(/^[^=]*=[[:space:]]*/, "", value); print value; exit
    }
  '
}

probes_original_state() {
  bold "returning to the original state (the book's Compose reset)"

  local volume="${COMPOSE_PROJECT}_ferroehr-pgdata"
  local hdr="$PROBE_TMP/reset-ehr.h" ehr_id=""

  # Data written before the reset is what the reset must remove, so write some.
  compose_up ferroehr seaweedfs seaweedfs-init
  if wait_http "$CDR/ferroehr/rest/status" 90; then
    curl -s -u "$BASIC" -X POST -D "$hdr" -o /dev/null "$API/ehr" || true
    ehr_id="$(grep -i '^location' "$hdr" 2>/dev/null | tr -d '\r' | awk -F/ '{print $NF}')"
  fi
  if [[ -z "$ehr_id" ]]; then
    uncovered "the Compose reset procedure" \
      "no EHR could be created before the reset, so nothing would show the reset
       removed it; the stack log tail was: $(dc logs --tail 5 ferroehr 2>&1 | tr '\n' ' ')"
    return
  fi

  # Step 1: stop the stack and delete its volumes, with the profile it runs.
  probe "P-RESET-VOLUME" "working" "compose" "#3637" \
    "docker compose down --volumes deletes the database volume"
  dc -f docker-compose.yml --profile s3 down --volumes >/dev/null 2>&1
  if docker volume inspect "$volume" >/dev/null 2>&1; then
    probe_fail "no volume named $volume" "the volume still exists" \
      "the book's step 1 leaves the database behind"
  fi
  probe_done

  # Steps 2 and 3: no overrides, then up. The harness exported the multimedia
  # keys for the other families, so they are removed here, in a subshell that
  # leaves the rest of the run untouched. The usage-report switch stays off: a
  # probe run sends no report, and that one deviation is declared below.
  local up_status=0
  (
    unset FERROEHR__MULTIMEDIA__ENABLED FERROEHR__MULTIMEDIA__ENDPOINT \
      FERROEHR__MULTIMEDIA__BUCKET FERROEHR__MULTIMEDIA__ALLOW_HTTP
    dc -f docker-compose.yml up -d --wait ferroehr >/dev/null 2>&1
  ) || up_status=$?

  probe "P-RESET-BOOT" "working" "compose" "#3637" \
    "the reset stack boots on a fresh volume and migrates it"
  if [[ "$up_status" -ne 0 ]] || ! wait_http "$CDR/health/readiness" 90; then
    probe_fail "a ready CDR after docker compose up -d --wait" "exit $up_status" \
      "$(dc logs --tail 5 ferroehr 2>&1 | tr '\n' ' ')"
    probe_done
    return
  fi
  docker volume inspect "$volume" >/dev/null 2>&1 \
    || probe_fail "a recreated $volume" "no such volume"
  probe_done

  probe "P-RESET-DATA" "working" "database" "#3637" \
    "an EHR written before the reset is gone after it"
  assert_eq "404" "$(http_code -u "$BASIC" "$API/ehr/$ehr_id")" \
    "GET /ehr/$ehr_id must find nothing on the reset database"
  probe_done

  # Step 4: the server's own reading of its configuration.
  probe "P-RESET-CONFIG" "working" "compose" "#3637" \
    "the reset server runs the shipped configuration"
  local effective
  effective="$(dc exec -T ferroehr /usr/local/bin/ferroehr config check 2>&1)"
  assert_eq '"sandbox"' "$(toml_value "$effective" "" deployment_profile)" \
    "the quickstart declares the sandbox profile"
  assert_eq "false" "$(toml_value "$effective" multimedia enabled)" \
    "an exported override from before the reset must not survive it"
  assert_contains "$(curl -s "$CDR/ferroehr/rest/status")" '"profile":"sandbox"' \
    "the public status document reports the shipped profile"
  probe_done

  uncovered "the usage report after a reset" \
    "the shipped quickstart sends it; this run keeps FERROEHR__USAGE_REPORT__ENABLED=false
     so a probe sends nothing, which is the one override the reset leaves in place."
  uncovered "the single-binary reset procedure" \
    "the Helm procedure runs in scripts/deploy-probe-k8s.sh (P-K8S-RESET-*); the
     single binary's DROP DATABASE and restart are the same database step with no
     release around it, and no probe runs a bare binary."
}

# The decommissioning family: the book's erase procedure
# (website/book/src/operations-decommissioning.md), run as written against the
# reset stack. CRA Annex I Part I(2)(m) asks for the possibility "to securely
# and easily remove on a permanent basis all data and settings"
# (docs/law/eu/cra/text.html). Destructive by design: run it after the reset.
probes_decommission() {
  bold "decommissioning (the book's erase command)"

  local hdr="$PROBE_TMP/erase-ehr.h" ehr_id="" out="" status=0 token=""
  curl -s -u "$BASIC" -X POST -D "$hdr" -o /dev/null "$API/ehr" || true
  ehr_id="$(grep -i '^location' "$hdr" 2>/dev/null | tr -d '\r' | awk -F/ '{print $NF}')"
  if [[ -z "$ehr_id" ]]; then
    uncovered "the erase command" \
      "no EHR could be created first, so nothing would show the erase removed it"
    return
  fi

  probe "P-ERASE-DRYRUN" "working" "image" "#3642" \
    "without --confirm the erase is a dry run that names its confirmation and deletes nothing"
  out="$(dc exec -T ferroehr /usr/local/bin/ferroehr db erase 2>&1)" || status=$?
  assert_eq "1" "$status" "a dry run exits 1"
  assert_contains "$out" "to erase, run: ferroehr db erase --confirm" \
    "the dry run prints the confirming command"
  assert_eq "200" "$(http_code -u "$BASIC" "$API/ehr/$ehr_id")" \
    "the EHR survives the dry run"
  token="$(printf '%s\n' "$out" | sed -nE 's/^to erase, run: ferroehr db erase --confirm (.+)$/\1/p' | tr -d '\r')"
  probe_done

  probe "P-ERASE-WRONG" "working" "image" "#3642" \
    "a confirmation naming another instance refuses and deletes nothing"
  status=0
  dc exec -T ferroehr /usr/local/bin/ferroehr db erase --confirm not-this-instance >/dev/null 2>&1 || status=$?
  [[ "$status" -ne 0 ]] || probe_fail "a non-zero exit" "exit 0" \
    "a wrong confirmation must refuse"
  assert_eq "200" "$(http_code -u "$BASIC" "$API/ehr/$ehr_id")" \
    "the EHR survives a refused erase"
  probe_done

  probe "P-ERASE-CONFIRM" "working" "database" "#3642" \
    "the confirmed erase removes the data, and the restarted server starts empty"
  if [[ -z "$token" ]]; then
    probe_fail "a confirmation token in the dry run's output" "none" "$out"
    probe_done
    return
  fi
  status=0
  out="$(dc exec -T ferroehr /usr/local/bin/ferroehr db erase --confirm "$token" 2>&1)" || status=$?
  assert_eq "0" "$status" "the confirmed erase exits 0: $out"
  dc -f docker-compose.yml restart ferroehr >/dev/null 2>&1 || true
  if wait_http "$CDR/health/readiness" 90; then
    assert_eq "404" "$(http_code -u "$BASIC" "$API/ehr/$ehr_id")" \
      "GET /ehr/$ehr_id must find nothing after the erase"
  else
    probe_fail "a ready CDR after the restart" "not ready" \
      "$(dc logs --tail 5 ferroehr 2>&1 | tr '\n' ' ')"
  fi
  probe_done

  uncovered "the erase of multimedia blobs and of a multi-database layout" \
    "this stack runs after the reset with multimedia off and one database; the
     blob deletion and the per-database transactions are covered by the
     decommission integration tests, not observed here."
}
