#!/bin/sh
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Seed the composed FHIR R4 terminology server with the CNF test terminologies.
#
# POSIX sh ON PURPOSE (the one non-bash script in the tree): the terminology
# container runs this via /bin/sh and ships no bash, so bash-only constructs
# ([[ ]] included) must stay out of this file.
#
# Runs as a one-shot init container of the `terminology` compose profile, after
# the server answers its own capability statement. Everything it writes is
# SYNTHETIC test content under the reserved example.test domain (IETF RFC 6761
# §6.2): two code systems and their enumerated value sets, one SNOMED-CT-shaped
# (hierarchical, numeric codes) and one LOINC-shaped (NNNNN-N codes; the
# namespace is deliberately named "lab-shaped": HAPI FHIR special-cases any
# ValueSet canonical URL CONTAINING the substring "loinc" and its url-based
# $expand then answers HAPI-2788 "Unknown ValueSet" for stored, searchable,
# by-id-expandable content — verified empirically against
# hapiproject/hapi:v8.10.0-3, 2026-07-29: loinc-shaped-vitals/loinc-x 404,
# identical content at lab-shaped-vitals/zz-vitals 200. No openEHR spec
# governs any of this — our own test infrastructure). No
# licensed terminology content is distributed here.
#
# IDEMPOTENT BY CONSTRUCTION: every resource is written with `PUT
# /<type>/<id>`, the FHIR "update as create" interaction
# (hl7.org/fhir/R4/http.html#update), so re-running the seeder against a server
# that already holds them is a no-op update rather than a duplicate. The stock
# HAPI image keeps its H2 database in memory, so a terminology-container
# restart drops the seed — re-run this step (or the whole profile) after one.
set -eu

FHIR_BASE="${FHIR_BASE:-http://terminology:8080/fhir}"
SEED_DIR="${SEED_DIR:-/seed}"
# The server boots a JVM + database schema; poll rather than guess a sleep.
READY_ATTEMPTS="${READY_ATTEMPTS:-120}"
READY_INTERVAL="${READY_INTERVAL:-5}"

echo "terminology-seed: waiting for ${FHIR_BASE}/metadata"
attempt=0
until [ "$attempt" -ge "$READY_ATTEMPTS" ]; do
  code=$(curl -s -o /dev/null -w '%{http_code}' "${FHIR_BASE}/metadata" || true)
  if [ "$code" = "200" ]; then
    break
  fi
  attempt=$((attempt + 1))
  sleep "$READY_INTERVAL"
done
if [ "$attempt" -ge "$READY_ATTEMPTS" ]; then
  echo "terminology-seed: ${FHIR_BASE} never became ready" >&2
  exit 1
fi

put() {
  # $1 = resource type, $2 = resource id, $3 = file
  status=$(curl -s -o /tmp/seed-response.json -w '%{http_code}' \
    -X PUT "${FHIR_BASE}/$1/$2" \
    -H 'Content-Type: application/fhir+json' \
    --data-binary "@$3")
  case "$status" in
    200 | 201)
      echo "terminology-seed: $1/$2 -> $status"
      ;;
    *)
      echo "terminology-seed: $1/$2 -> $status" >&2
      cat /tmp/seed-response.json >&2
      exit 1
      ;;
  esac
}

put CodeSystem cnf-sct-shaped "${SEED_DIR}/codesystem-sct-shaped.json"
put CodeSystem cnf-lab-shaped "${SEED_DIR}/codesystem-lab-shaped.json"
put ValueSet cnf-sct-shaped-disorders "${SEED_DIR}/valueset-sct-shaped-disorders.json"
put ValueSet cnf-lab-shaped-vitals "${SEED_DIR}/valueset-lab-shaped-vitals.json"

# Prove the two operations the CDR's FHIR provider actually calls answer for
# the seeded content BEFORE the CDR starts: a value-set membership test
# (ValueSet/$validate-code) and an expansion (ValueSet/$expand). Both need
# HAPI's Hibernate Search index, which the compose service enables; failing
# here is far cheaper to read than a terminology error inside a conformance
# run.
verify() {
  # $1 = human label, $2 = url
  status=$(curl -s -o /tmp/seed-verify.json -w '%{http_code}' "$2")
  if [ "$status" != "200" ]; then
    echo "terminology-seed: $1 -> $status" >&2
    cat /tmp/seed-verify.json >&2
    exit 1
  fi
  echo "terminology-seed: $1 -> 200"
}

# BOTH operations on BOTH namespaces: the CDR's provider calls exactly these
# two shapes, and HAPI resolves them through different internal paths — a
# ValueSet that answers one can still 404 the other (the "loinc" quirk below),
# so a single sample proves nothing.
# shellcheck disable=SC2016 # $validate-code and $expand are FHIR operation names, not shell expansions
verify '$validate-code (sct-shaped member)' \
  "${FHIR_BASE}/ValueSet/\$validate-code?url=http://cnf.example.test/fhir/ValueSet/sct-shaped-disorders&system=http://cnf.example.test/fhir/CodeSystem/sct-shaped&code=1000002"
# shellcheck disable=SC2016 # $expand is a FHIR operation name, not a shell expansion
verify '$expand (sct-shaped disorders)' \
  "${FHIR_BASE}/ValueSet/\$expand?url=http://cnf.example.test/fhir/ValueSet/sct-shaped-disorders"
# shellcheck disable=SC2016 # $validate-code is a FHIR operation name, not a shell expansion
verify '$validate-code (lab-shaped member)' \
  "${FHIR_BASE}/ValueSet/\$validate-code?url=http://cnf.example.test/fhir/ValueSet/lab-shaped-vitals&system=http://cnf.example.test/fhir/CodeSystem/lab-shaped&code=99991-1"
# shellcheck disable=SC2016 # $expand is a FHIR operation name, not a shell expansion
verify '$expand (lab-shaped vitals)' \
  "${FHIR_BASE}/ValueSet/\$expand?url=http://cnf.example.test/fhir/ValueSet/lab-shaped-vitals"

echo "terminology-seed: done"
