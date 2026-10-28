#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The logging component against independent ATNA implementations (EHDS Annex
# II 1.4): FerroEHR's ITI-20 feeds and its ITI-81 retrieval, exercised against
# software FerroEHR's authors did not write, each run from a published image.
#
#   ITI-20, FHIR feed   HAPI FHIR (hapiproject/hapi), an open-source FHIR R4
#                       server, is the Audit Record Repository the feed posts
#                       to. It must accept every record, and its validator
#                       ($validate, with IHE BALP 1.1.4 loaded) judges each
#                       stored AuditEvent against base R4 and the BALP profile
#                       the record names.
#   ITI-20, syslog      syslog-ng (balabit/syslog-ng) receives the DICOM feed
#                       over RFC 5425 TLS and parses each RFC 5424 message; each
#                       payload must be well-formed XML that libxml2 (xmllint)
#                       reads, rooted at the PS3.15 A.5.1 AuditMessage with its
#                       three required parts.
#   ITI-81              the same searches are put to FerroEHR's retrieval and
#                       to HAPI's AuditEvent search over the records the feed
#                       delivered; both must answer the same records.
#
# The workload is the load test (tools/loadtest) at a small size. Every
# disagreement is printed; the run exits 1 on any. No specification names a
# method for an interoperability run: our own design.
#
# Usage: scripts/interop/atna.sh <image> <postgres-image> <out-dir>
#   LOADTEST_BIN    the built load test binary (default target/release/loadtest)
#   FERROEHR_PORT   the host port the quickstart publishes (default 8080)
#   ATNA_HAPI_PORT  the host port HAPI is published on (default 8081)
#   ATNA_KEEP=1     leave the stack running for inspection
#   Writes <out-dir>/summary.md, the HAPI validation outcomes, the syslog
#   messages as received, and both sides' search answers.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly ROOT_DIR
readonly PROJECT=ferroehr-atna-interop
readonly COMPOSE_FILE="$ROOT_DIR/docker-compose.yml"
readonly HAPI_IMAGE="docker.io/hapiproject/hapi:v8.12.0-2@sha256:0ff86aac4c7bd13a1ee226444a70263bb46ed1c28ef543c30a44f865b80ca04e"
readonly SYSLOG_IMAGE="docker.io/balabit/syslog-ng:4.11.0@sha256:c652345a143843731037863abc6bfd75da5ddaf618a94ca42147fa2d59fbcf89"
readonly BALP_VERSION=1.1.4
readonly OPERATION_SYSTEM="urn:openehr:its-rest:operation"
readonly USER_PASS="ferroehr:ferroehr"

usage() {
  echo "usage: $0 <image> <postgres-image> <out-dir>" >&2
  exit 2
}

[[ "$#" -eq 3 ]] || usage
image="$1"
pg_image="$2"
out="$(mkdir -p "$3" && cd "$3" && pwd)"
loadtest="${LOADTEST_BIN:-$ROOT_DIR/target/release/loadtest}"
[[ -x "$loadtest" ]] || {
  echo "error: no load test binary at $loadtest (cargo build --release -p loadtest)" >&2
  exit 2
}
port="${FERROEHR_PORT:-8080}"
hapi_port="${ATNA_HAPI_PORT:-8081}"
for p in "$port" "$hapi_port"; do
  [[ "$p" =~ ^[0-9]+$ ]] || {
    echo "error: a port must be a number (got '$p')" >&2
    exit 2
  }
done
for tool in docker jq xmllint openssl curl; do
  command -v "$tool" >/dev/null || {
    echo "error: $tool is not on PATH" >&2
    exit 2
  }
done
readonly base="http://localhost:${port}/ferroehr/rest"
readonly hapi="http://localhost:${hapi_port}/fhir"

# A test instance sends no usage report (the quickstart reads this switch).
export FERROEHR__USAGE_REPORT__ENABLED=false
export COMPOSE_PROJECT_NAME="$PROJECT"
export FERROEHR_IMAGE="$image" FERROEHR_POSTGRES_IMAGE="$pg_image"
mkdir -p "$out/tls" "$out/syslog" "$out/searches" "$out/validation"
chmod 0777 "$out/syslog"

# ── The syslog repository's TLS identity, from a CA made for this run ─────────
openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj "/CN=atna-interop-ca" \
  -keyout "$out/tls/ca.key" -out "$out/tls/ca.pem" 2>/dev/null
openssl req -newkey rsa:2048 -nodes -subj "/CN=atna-syslog" \
  -keyout "$out/tls/server.key" -out "$out/tls/server.csr" 2>/dev/null
printf 'subjectAltName=DNS:atna-syslog\nextendedKeyUsage=serverAuth\n' >"$out/tls/ext.cnf"
openssl x509 -req -in "$out/tls/server.csr" -CA "$out/tls/ca.pem" -CAkey "$out/tls/ca.key" \
  -CAcreateserial -days 2 -extfile "$out/tls/ext.cnf" -out "$out/tls/server.pem" 2>/dev/null
chmod 0644 "$out/tls/"*

cat >"$out/syslog-ng.conf" <<'CONF'
@version: 4.11
# RFC 5425: TLS on 6514 with octet-counted frames, which syslog-ng's syslog()
# driver reads; it parses each frame as RFC 5424 and marks one it cannot.
source s_atna {
  syslog(
    transport("tls") port(6514)
    tls(key-file("/etc/atna/tls/server.key") cert-file("/etc/atna/tls/server.pem")
        peer-verify(optional-untrusted))
  );
};
destination d_atna {
  file("/var/log/atna/messages.jsonl"
       template("$(format-json PRI=${PRI} PROGRAM=${PROGRAM} MSGID=${MSGID} HOST=${HOST} MESSAGE=${MESSAGE})\n"));
};
log { source(s_atna); destination(d_atna); };
CONF

cat >"$out/interop.compose.yml" <<YAML
services:
  ferroehr:
    environment:
      FERROEHR__AUDIT__ENABLED: "true"
      FERROEHR__AUDIT__FAIL_MODE: closed
      FERROEHR__AUDIT__FHIR_FEED__ENABLED: "true"
      FERROEHR__AUDIT__FHIR_FEED__URL: http://atna-hapi:8080/fhir
      FERROEHR__AUDIT__FHIR_FEED__POLL_INTERVAL_MS: "500"
      FERROEHR__AUDIT__SYSLOG__ENABLED: "true"
      FERROEHR__AUDIT__SYSLOG__HOST: atna-syslog
      FERROEHR__AUDIT__SYSLOG__PORT: "6514"
      FERROEHR__AUDIT__SYSLOG__TLS_CA_FILE: /etc/atna/ca.pem
    volumes:
      - $out/tls/ca.pem:/etc/atna/ca.pem:ro
  atna-hapi:
    image: $HAPI_IMAGE
    environment:
      hapi.fhir.fhir_version: R4
      hapi.fhir.implementationguides.balp.name: ihe.iti.balp
      hapi.fhir.implementationguides.balp.version: "$BALP_VERSION"
      hapi.fhir.implementationguides.balp.installMode: STORE_AND_INSTALL
      hapi.fhir.implementationguides.balp.fetchDependencies: "true"
    ports:
      - "127.0.0.1:$hapi_port:8080"
  atna-syslog:
    image: $SYSLOG_IMAGE
    volumes:
      - $out/syslog-ng.conf:/etc/syslog-ng/syslog-ng.conf:ro
      - $out/tls:/etc/atna/tls:ro
      - $out/syslog:/var/log/atna
YAML

compose() { docker compose -f "$COMPOSE_FILE" -f "$out/interop.compose.yml" "$@"; }
down() { [[ "${ATNA_KEEP:-}" == 1 ]] || compose down -v --remove-orphans >/dev/null 2>&1 || true; }
trap down EXIT
down

# ── Bring up the two repositories, then FerroEHR ─────────────────────────────
compose up -d atna-hapi atna-syslog
echo "==> waiting for HAPI FHIR (it loads IHE BALP $BALP_VERSION first)"
for _ in $(seq 1 120); do
  curl -fsS "$hapi/metadata" >/dev/null 2>&1 && break
  sleep 5
done
curl -fsS "$hapi/metadata" >/dev/null || {
  echo "::error::HAPI FHIR did not start" >&2
  compose logs --tail 100 atna-hapi >&2 || true
  exit 2
}
curl -fsS "$hapi/StructureDefinition?url=https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.Read&_summary=count" \
  | jq -e '.total >= 1' >/dev/null || {
  echo "::error::HAPI FHIR started without the IHE BALP $BALP_VERSION profiles" >&2
  exit 2
}
if ! compose up -d --wait --wait-timeout 300 ferroehr-postgres ferroehr; then
  echo "::error::the FerroEHR stack did not become healthy" >&2
  compose logs --tail 200 ferroehr >&2 || true
  exit 2
fi

# ── The workload ─────────────────────────────────────────────────────────────
start="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
sleep 1
"$loadtest" --base-url "$base/openehr/v1" --ehrs 2 --compositions-per-ehr 2 \
  --warmup 0 --operations 40 --workers 2 --out "$out/workload.json"
sleep 2
end="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
sleep 1
window="date=ge${start}&date=le${end}"

ours_total() { curl -fsS -u "$USER_PASS" -H 'Accept: application/fhir+json' "$base/fhir/r4/AuditEvent?${window}${1:+&$1}&_count=0" | jq -r .total; }
hapi_total() { curl -fsS -H 'Accept: application/fhir+json' "$hapi/AuditEvent?${window}${1:+&$1}&_summary=count" | jq -r .total; }

echo "==> waiting for the FHIR feed to deliver the window's records"
want="$(ours_total "")"
got=0
for _ in $(seq 1 60); do
  got="$(hapi_total "")"
  [[ "$got" == "$want" ]] && break
  sleep 2
  want="$(ours_total "")"
done

failures=()
fail() { failures+=("$1"); echo "  DISAGREEMENT: $1"; }

# ── ITI-20, FHIR feed: accepted, and valid by HAPI's validator ───────────────
echo "==> ITI-20 FHIR feed: $want records in the window, $got delivered to HAPI"
[[ "$got" == "$want" ]] || fail "the FHIR feed delivered $got of the window's $want records to HAPI"
curl -fsS -H 'Accept: application/fhir+json' "$hapi/AuditEvent?${window}&_count=1000" >"$out/validation/delivered.json"
invalid=0
while IFS= read -r event; do
  id="$(jq -r .id <<<"$event")"
  profile="$(jq -r '.meta.profile[0] // empty' <<<"$event")"
  query=""
  [[ -n "$profile" ]] && query="?profile=$(jq -rn --arg p "$profile" '$p|@uri')"
  curl -sS -H 'Content-Type: application/fhir+json' -H 'Accept: application/fhir+json' \
    -X POST --data "$event" "$hapi/AuditEvent/\$validate${query}" >"$out/validation/$id.json"
  errors="$(jq '[.issue[]? | select(.severity == "error" or .severity == "fatal")] | length' "$out/validation/$id.json")"
  if [[ "$errors" != 0 ]]; then
    invalid=$((invalid + 1))
    fail "AuditEvent $id ($profile) fails HAPI's validator: $(jq -r '[.issue[] | select(.severity == "error" or .severity == "fatal") | .diagnostics] | unique | join(" | ")' "$out/validation/$id.json")"
  fi
done < <(jq -c '.entry[]?.resource | del(.id, .meta.versionId, .meta.lastUpdated, .meta.source) + {id: .id}' "$out/validation/delivered.json")
echo "    $invalid of $got failed validation (base R4 and the BALP profile each names)"

# ── ITI-20, syslog: received over TLS, RFC 5424, a PS3.15 AuditMessage ────────
syslog_file="$out/syslog/messages.jsonl"
received=0
[[ -f "$syslog_file" ]] && received="$(wc -l <"$syslog_file" | tr -d ' ')"
echo "==> ITI-20 syslog: $received messages received by syslog-ng over TLS"
[[ "$received" -gt 0 ]] || fail "syslog-ng received no message over RFC 5425 TLS"
bad_xml=0
n=0
while IFS= read -r line; do
  n=$((n + 1))
  jq -r .MESSAGE <<<"$line" >"$out/syslog/message-$n.xml"
  if ! xmllint --noout "$out/syslog/message-$n.xml" 2>/dev/null; then
    bad_xml=$((bad_xml + 1))
    fail "syslog message $n is not well-formed XML (or syslog-ng could not parse its RFC 5424 frame): $(head -c 200 "$out/syslog/message-$n.xml")"
    continue
  fi
  for part in EventIdentification ActiveParticipant AuditSourceIdentification; do
    if [[ "$(xmllint --xpath "count(/AuditMessage/$part)" "$out/syslog/message-$n.xml" 2>/dev/null)" == 0 ]]; then
      bad_xml=$((bad_xml + 1))
      fail "syslog message $n has no /AuditMessage/$part (DICOM PS3.15 A.5.1)"
    fi
  done
done < <([[ -f "$syslog_file" ]] && cat "$syslog_file")
echo "    $bad_xml problems in $received messages"

# ── ITI-81: the same searches on FerroEHR and on HAPI ─────────────────────────
echo "==> ITI-81: the same searches on both repositories"
searches=(
  ""
  "action=C"
  "action=R"
  "action=E"
  "outcome=0"
  "type=http://terminology.hl7.org/CodeSystem/audit-event-type|rest"
  "subtype=${OPERATION_SYSTEM}|composition_create"
  "subtype=${OPERATION_SYSTEM}|composition_get"
  "subtype=http://hl7.org/fhir/restful-interaction|read"
  "entity-type=http://terminology.hl7.org/CodeSystem/audit-entity-type|2"
)
# key FILE: each record of a searchset as recorded|action|subtypes, sorted.
key() { jq -r '[.entry[]?.resource | "\(.recorded)|\(.action)|\([.subtype[]? | "\(.system)|\(.code)"] | sort | join(","))"] | sort[]' "$1"; }
i=0
for q in "${searches[@]}"; do
  i=$((i + 1))
  enc="$(jq -rn --arg q "$q" '$q | sub("\\|"; "%7C")')"
  curl -fsS -u "$USER_PASS" -H 'Accept: application/fhir+json' \
    "$base/fhir/r4/AuditEvent?${window}${enc:+&$enc}&_count=1000" >"$out/searches/$i-ferroehr.json"
  curl -fsS -H 'Accept: application/fhir+json' \
    "$hapi/AuditEvent?${window}${enc:+&$enc}&_count=1000" >"$out/searches/$i-hapi.json"
  ours="$(key "$out/searches/$i-ferroehr.json")"
  theirs="$(key "$out/searches/$i-hapi.json")"
  o="$(grep -c . <<<"$ours" || true)"
  t="$(grep -c . <<<"$theirs" || true)"
  if [[ "$ours" == "$theirs" ]]; then
    echo "    ${q:-<window only>}: both answer $o records"
  else
    fail "the search '${q:-<window only>}' answers $o records on FerroEHR and $t on HAPI"
    diff <(printf '%s\n' "$ours") <(printf '%s\n' "$theirs") | head -10 || true
  fi
done

{
  echo "### The logging component against independent ATNA implementations"
  echo
  echo "| Transaction | Independent implementation | Result |"
  echo "|---|---|---|"
  echo "| ITI-20 FHIR feed | HAPI FHIR \`v8.12.0-2\`, validator with IHE BALP $BALP_VERSION | $got of $want records delivered; $invalid failed validation |"
  echo "| ITI-20 syslog (RFC 5425 TLS, RFC 5424, DICOM PS3.15 A.5) | syslog-ng \`4.11.0\`, libxml2 | $received messages received; $bad_xml problems |"
  echo "| ITI-81 retrieval | HAPI FHIR AuditEvent search over the delivered records | ${#searches[@]} searches compared |"
  echo
  if ((${#failures[@]} == 0)); then
    echo "VERDICT: no disagreement"
  else
    echo "Disagreements:"
    echo
    for f in "${failures[@]}"; do echo "- $f"; done
    echo
    echo "VERDICT: ${#failures[@]} disagreements"
  fi
  printf "\nimage: \`%s\`; window %s to %s\n" "$image" "$start" "$end"
} | tee "$out/summary.md"
((${#failures[@]} == 0))
