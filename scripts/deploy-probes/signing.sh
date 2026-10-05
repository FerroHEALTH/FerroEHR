#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The VERSION-signing probe family (#2163).
#
# `[signing]` is on by default in `digest` mode with read-time verification
# `strict`, which means the server recomputes the signature of a version it
# signed on every read and refuses to serve a provably corrupt record. That is a
# strong claim, and it had never been watched work.
#
# The probes that matter are the TAMPER ones: a signing feature with no
# demonstrated detection is decoration. So they reach past the API into the
# stored rows, change a byte of clinical content, and assert it is caught — the
# only way to show the verification is real rather than a field nobody checks.
# There are two stored copies of a version's content and one probe per copy:
# version.body, caught by the read path, and the node rows, caught by the
# admin storage-parity sweep.
#
# Sourced by scripts/deploy-probe.sh; never run directly.

# Run SQL against the composed database as the quickstart's role.
probe_psql() {
  dc exec -T ferroehr-postgres psql -U "${PG_INIT_USER:-ferroehr}" \
    -d "${PG_INIT_DB:-ferroehr}" -tAc "$1" 2>/dev/null
}

probes_signing() {
  bold "VERSION signing (digest mode)"

  # A committed version must carry a signature the server generated, and the
  # read path must serve it. Digest mode is the shipped default, so this is the
  # posture nearly every deployment runs.
  probe "P-SIGN-DIGEST" "working" "server" "-" \
    "a committed VERSION carries a server signature and reads back cleanly"
  local ehr version
  ehr="$(curl -s -u "$BASIC" -X POST -D - -o /dev/null "$API/ehr" \
    | grep -i '^location' | tr -d '\r' | awk -F/ '{print $NF}')"
  if [[ -z "$ehr" ]]; then
    probe_fail "a committed EHR" "no id returned"
    probe_done
    return 0
  fi
  version="$(curl -s -u "$BASIC" "$API/ehr/$ehr/versioned_ehr_status/version")"
  assert_contains "$version" '"signature"' \
    "with signing enabled a served ORIGINAL_VERSION must carry its signature"
  assert_eq "200" "$(http_code -u "$BASIC" "$API/ehr/$ehr/versioned_ehr_status/version")" \
    "an untampered version verifies on read"
  probe_done

  # The one that makes the feature more than decoration. Reaching past the API
  # into the stored rows is the point: an attacker or a corrupt disk does not
  # go through the write path, so neither does this. The tamper target is
  # version.body — the materialized projection every point read serves. The
  # node rows are the storage's OTHER copy, and P-NODE-TAMPER below covers
  # them through the channel that does see them.
  probe "P-SIGN-TAMPER" "broken" "server" "-" \
    "a tampered stored version is REFUSED on read, not served"
  local before after rows
  before="$(http_code -u "$BASIC" "$API/ehr/$ehr/versioned_ehr_status/version")"
  rows="$(probe_psql "UPDATE clinical.version
                         SET body = (jsonb_set((body)::jsonb, '{name,value}', '\"tampered\"'))::text
                       WHERE ehr_id = '$ehr'::uuid AND kind = 'EHR_STATUS';" \
          && probe_psql "SELECT count(*) FROM clinical.version
                          WHERE ehr_id = '$ehr'::uuid
                            AND (body)::jsonb #>> '{name,value}' = 'tampered';")"
  if [[ "${rows:-0}" = "0" ]]; then
    probe_fail "a tampered stored row" "the UPDATE matched nothing" \
      "the probe could not reach the stored content, so detection was never tested"
  else
    after="$(http_code -u "$BASIC" "$API/ehr/$ehr/versioned_ehr_status/version")"
    case "$after" in
      5*) : ;;
      *)  probe_fail "a 5xx integrity refusal" "$after" \
            "verify_on_read is strict by default, so a corrupt record must not be served (was $before before tampering)" ;;
    esac
  fi
  probe_done

  # The storage keeps every version's content twice: version.body, which the
  # probe above covers, and the decomposed node rows the AQL engine queries. A
  # read-time signature check recomputes only the first, so a tampered node row
  # is invisible to it; the admin storage-parity sweep is the channel that sees
  # it. Same reach-past-the-API shape, aimed at the other copy.
  probe "P-NODE-TAMPER" "broken" "server" "-" \
    "a tampered node row is reported by the admin storage-parity sweep"
  local node_ehr node_vo node_rows sweep sweep_code
  node_ehr="$(curl -s -u "$BASIC" -X POST -D - -o /dev/null "$API/ehr" \
    | grep -i '^location' | tr -d '\r' | awk -F/ '{print $NF}')"
  node_vo="$(probe_psql "SELECT vo_id FROM clinical.version
                          WHERE ehr_id = '$node_ehr'::uuid AND kind = 'EHR_STATUS';" \
             | tr -d '[:space:]')"
  if [[ -z "$node_ehr" ]] || [[ -z "$node_vo" ]]; then
    probe_fail "a committed EHR_STATUS version" "ehr='$node_ehr' vo='$node_vo'" \
      "the probe could not locate the stored version, so detection was never tested"
    probe_done
    return 0
  fi
  node_rows="$(probe_psql "UPDATE clinical.node
                              SET data = jsonb_set(data, '{archetype_node_id}', '\"tampered\"')
                            WHERE vo_id = '$node_vo'::uuid AND num = 0;" \
               && probe_psql "SELECT count(*) FROM clinical.node
                               WHERE vo_id = '$node_vo'::uuid
                                 AND data #>> '{archetype_node_id}' = 'tampered';")"
  if [[ "${node_rows:-0}" = "0" ]]; then
    probe_fail "a tampered node row" "the UPDATE matched nothing" \
      "the probe could not reach the AQL index copy, so detection was never tested"
  else
    sweep_code="$(http_code -u "$BASIC" -X POST "$API/admin/integrity/verify")"
    assert_eq "200" "$sweep_code" \
      "the sweep itself must succeed — a finding is reported in the body, not as a failed request"
    sweep="$(curl -s -u "$BASIC" -X POST "$API/admin/integrity/verify")"
    assert_contains "$sweep" "$node_vo" \
      "the sweep must name the tampered versioned object"
    assert_contains "$sweep" '"defect":"content_differs"' \
      "a node row that no longer matches the materialized body is a content_differs mismatch"
  fi
  probe_done
}
