#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The EHR-index family of the Kubernetes probe (#5108): each way the chart
# holds the subject-to-EHR cross-reference, installed and asked.
#
#   1. `ferropix.enabled: true`, the default: the chart deploys FerroPIX and
#      points `[ehr_index]` at it.
#   2. `ferropix.enabled: false` with `pixm_url`: another PIX Manager over
#      PIXm (ITI-83). The probe installs FerroPIX as its own release and names
#      its Service, so the path runs without a second product.
#   3. `ferropix.enabled: false` with `provider: local`: FerroEHR on its own.
#
# With neither 2 nor 3 the render is refused. Each live mode creates an EHR
# and finds it again by its subject. Under the IHE modes the subject is named
# in a source domain and the CDR asks the PIX Manager for the clinical
# pseudonym (ITI-83), which is what proves the index answers.
#
# FerroPIX 0.0.18 reads its issuer from the issuer's metadata and accepts a
# plain-http issuer on loopback only (RFC 8414 §2), so the development issuer
# runs beside it in its pod on localhost:8180 and signs every client
# credentials token with that `iss` and FerroPIX's audience. The CDR reaches
# the same issuer through a Service of the probe's own, and a NetworkPolicy of
# the probe's own admits it, beside the one the FerroPIX chart renders.
#
# Sourced by scripts/deploy-probe-k8s.sh after kubernetes.sh; never run
# directly. No openEHR specification governs the index: our own design.

K8S_PIX_RELEASE="ferropix"
K8S_PIX_LABEL="app.kubernetes.io/name=ferropix"
K8S_PIX_ISSUER_SVC="probe-pix-issuer"
K8S_PIX_ISSUER="http://localhost:8180/machines"
K8S_PIX_AUDIENCE="http://localhost:8080/fhir"
K8S_PIX_SOURCE_CLIENT="probe-source"
K8S_PIX_SOURCE_SYSTEM="urn:oid:2.999.1.1"
K8S_PIX_CLINICAL="urn:oid:2.999.7.1"
K8S_PIX_DB_SECRET="probe-ferropix-db"
K8S_PIX_KEY_SECRET="probe-ferropix-data-key"
K8S_PIX_PORT="${PROBE_K8S_PIX_PORT:-18093}"
K8S_PIX_ISSUER_PORT="${PROBE_K8S_PIX_ISSUER_PORT:-18094}"
K8S_PIX_PF_PID=""
K8S_PIX_CHART=""
K8S_INDEX_EHR=""

# FerroPIX's database on the probe's PostgreSQL, created afresh because each run
# writes a new sealing key, and its two Secrets.
k8s_pix_inputs() {
  docker compose -p "$COMPOSE_PROJECT" exec -T ferroehr-postgres \
    psql -v ON_ERROR_STOP=1 -U postgres -d postgres >/dev/null 2>&1 <<'SQL' || return 1
SELECT 'CREATE ROLE ferropix LOGIN PASSWORD ''ferropix'''
  WHERE NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferropix') \gexec
DROP DATABASE IF EXISTS ferropix WITH (FORCE);
CREATE DATABASE ferropix OWNER ferropix;
SQL
  kc delete secret "$K8S_PIX_DB_SECRET" "$K8S_PIX_KEY_SECRET" --ignore-not-found >/dev/null 2>&1
  kc create secret generic "$K8S_PIX_DB_SECRET" \
    --from-literal=url="postgresql://ferropix:ferropix@${K8S_DB_HOST}:${K8S_DB_PORT}/ferropix" \
    >/dev/null 2>&1 || return 1
  kc create secret generic "$K8S_PIX_KEY_SECRET" \
    --from-literal=data-key="$(openssl rand -base64 32)" >/dev/null 2>&1
}

# The FerroPIX values both IHE modes install, at the top level of the
# FerroPIX chart; mode 1 nests them under `ferropix:`.
k8s_pix_values() {
  local config
  config="$(jq -cn --arg iss "$K8S_PIX_ISSUER" --arg aud "$K8S_PIX_AUDIENCE" \
    '{tokenCallbacks: [{issuerId: "machines", requestMappings: [{requestParam: "grant_type",
      match: "client_credentials", claims: {iss: $iss, sub: "${clientId}",
      client_id: "${clientId}", aud: [$aud], scope: "${scope}"}}]}]}')"
  cat <<YAML
database:
  existingSecret: $K8S_PIX_DB_SECRET
dataKey:
  existingSecret: $K8S_PIX_KEY_SECRET
usageReport:
  enabled: false
# A kind node is itself a container, and runc cannot mount sysfs in a pod's
# own user namespace there (#4364), so FerroPIX shares the node's.
hostUsers: true
config:
  profile: production
  public_base_url: "http://localhost:8080"
  server:
    listen: "0.0.0.0:8080"
  auth:
    audience: ["$K8S_PIX_AUDIENCE"]
    issuer:
      - issuer: "$K8S_PIX_ISSUER"
        token_profile: jwt
  trust:
    issuer:
      - issuer: "$K8S_PIX_ISSUER"
        clients: ["$K8S_PIX_SOURCE_CLIENT"]
  domain:
    - system: "$K8S_PIX_SOURCE_SYSTEM"
      name: "Probe hospital"
      kind: local
      source_client_id: $K8S_PIX_SOURCE_CLIENT
  pseudonym_domain:
    - namespace: "$K8S_PIX_CLINICAL"
      name: "Probe clinical domain"
      consumer_client_id: ferroehr
extraContainers:
  - name: issuer
    image: $K8S_MACHINES_IMAGE
    env:
      - name: SERVER_PORT
        value: "8180"
      - name: JSON_CONFIG
        value: '$config'
    ports: [{ containerPort: 8180, name: issuer }]
    securityContext:
      allowPrivilegeEscalation: false
      runAsNonRoot: true
      runAsUser: 65532
      capabilities: { drop: [ALL] }
      seccompProfile: { type: RuntimeDefault }
YAML
}

# The CDR's half of an IHE mode: the clinical domain, and the grant it asks
# the PIX Manager with. $1 is the `[ehr_index]` block's own keys.
k8s_pix_cdr_values() {
  cat <<YAML
secrets:
  ehrIndexClientSecret: probe-not-a-secret
config:
  # A cohort may name its patients by identifier, which is what asks the PIX
  # Manager; with no small-cell floor its one EHR is served, not withheld.
  cohort:
    subjects: true
    small_cell_threshold: 0
  privacy:
    subject_namespaces: ["$K8S_PIX_CLINICAL"]
  ehr_index:
$1
    assigning_authority: "$K8S_PIX_CLINICAL"
    oauth:
      token_endpoint: "http://$K8S_PIX_ISSUER_SVC:8180/machines/token"
      client_id: ferroehr
      scope: system/Patient.rs
YAML
}

# The Service and NetworkPolicy that let the CDR reach the issuer beside
# FerroPIX, whichever release runs it.
k8s_pix_issuer_route() {
  kc apply -f - >/dev/null 2>&1 <<YAML
apiVersion: v1
kind: Service
metadata:
  name: $K8S_PIX_ISSUER_SVC
spec:
  selector: { app.kubernetes.io/name: ferropix }
  ports: [{ port: 8180, targetPort: 8180 }]
---
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: $K8S_PIX_ISSUER_SVC
spec:
  podSelector: { matchLabels: { app.kubernetes.io/name: ferropix } }
  policyTypes: [Ingress]
  ingress:
    - ports: [{ port: 8180, protocol: TCP }]
      from: [{ podSelector: { matchLabels: { app.kubernetes.io/name: ferroehr } } }]
YAML
}

k8s_pix_rollout() {
  local deploy
  deploy="$(kc get deploy -l "$K8S_PIX_LABEL" -o jsonpath='{.items[0].metadata.name}' 2>/dev/null)"
  [[ -n "$deploy" ]] || return 1
  kc rollout status "deploy/$deploy" --timeout="${1:-300s}" >/dev/null 2>&1
}

k8s_pix_pf_start() {
  local pod
  pod="$(kc get pod -l "$K8S_PIX_LABEL" \
    -o jsonpath='{range .items[?(@.status.containerStatuses[0].ready==true)]}{.metadata.name}{"\n"}{end}' \
    2>/dev/null | sed -n 1p)"
  [[ -n "$pod" ]] || return 1
  k8s_pix_pf_stop
  # kubectl itself, not the kc function: a backgrounded function is a subshell,
  # and its pid is not the forward's, so stopping it would leave the forward.
  kubectl -n "$K8S_NS" port-forward "pod/$pod" "${K8S_PIX_PORT}:8080" "${K8S_PIX_ISSUER_PORT}:8180" >/dev/null 2>&1 &
  K8S_PIX_PF_PID=$!
  wait_http "http://127.0.0.1:${K8S_PIX_PORT}/health/liveness" 30
}

k8s_pix_pf_stop() {
  if [[ -n "$K8S_PIX_PF_PID" ]]; then
    disown "$K8S_PIX_PF_PID" 2>/dev/null || true
    kill "$K8S_PIX_PF_PID" >/dev/null 2>&1 || true
    wait "$K8S_PIX_PF_PID" 2>/dev/null || true
  fi
  K8S_PIX_PF_PID=""
}

# A client credentials token from the issuer beside FerroPIX.
k8s_pix_token() { # <client-id> <scope>
  curl -s -m 30 -d grant_type=client_credentials -d "client_id=$1" -d client_secret=probe \
    --data-urlencode "scope=$2" "http://127.0.0.1:${K8S_PIX_ISSUER_PORT}/machines/token" \
    | jq -r '.access_token // empty'
}

# The EHR_STATUS that names <id> in <namespace>.
k8s_index_status() { # <namespace> <id>
  jq -cn --arg ns "$1" --arg id "$2" '{_type: "EHR_STATUS",
    archetype_node_id: "openEHR-EHR-EHR_STATUS.generic.v1",
    archetype_details: {_type: "ARCHETYPED", rm_version: "1.1.0",
      archetype_id: {_type: "ARCHETYPE_ID", value: "openEHR-EHR-EHR_STATUS.generic.v1"}},
    name: {_type: "DV_TEXT", value: "EHR Status"},
    subject: {_type: "PARTY_SELF", external_ref: {_type: "PARTY_REF", namespace: $ns,
      type: "PERSON", id: {_type: "GENERIC_ID", value: $id, scheme: $ns}}},
    is_queryable: true, is_modifiable: true}'
}

# Creates an EHR whose subject is <id> in <namespace>, its id in $K8S_INDEX_EHR;
# a refusal fails the probe and leaves it empty.
k8s_index_create() { # <namespace> <id>
  local code
  code="$(curl -s -u "$K8S_BASIC" -X POST -D "$PROBE_TMP/index-create.headers" \
    -o "$PROBE_TMP/index-create" -w "$K8S_HTTP_CODE" -H 'Content-Type: application/json' \
    --data "$(k8s_index_status "$1" "$2")" "$K8S_API/ehr")"
  K8S_INDEX_EHR="$(grep -i '^location' "$PROBE_TMP/index-create.headers" | tr -d '\r' | awk -F/ '{print $NF}')"
  [[ -n "$K8S_INDEX_EHR" ]] && return 0
  local log
  log="$(kc logs -l app.kubernetes.io/name=ferroehr --tail=200 2>&1)"
  probe_fail "a Location header naming the EHR created for the subject" \
    "$code $(head -c 300 "$PROBE_TMP/index-create")" \
    "$(grep -E '"level":"(ERROR|WARN)"' <<<"$log" | sed -n 1,2p | cut -c1-400)"
  return 1
}

# The local index: GET /ehr by the subject the EHR_STATUS names.
k8s_index_local_roundtrip() {
  local subject="local-$RANDOM$RANDOM" code found
  k8s_index_create probe "$subject" || return 0
  code="$(curl -s -u "$K8S_BASIC" -G -H 'Accept: application/json' -o "$PROBE_TMP/index-lookup" \
    -w "$K8S_HTTP_CODE" --data-urlencode "subject_id=$subject" --data-urlencode "subject_namespace=probe" \
    "$K8S_API/ehr")"
  found="$(jq -r '.ehr_id.value // empty' "$PROBE_TMP/index-lookup" 2>/dev/null)"
  [[ "$found" == "$K8S_INDEX_EHR" ]] && return 0
  probe_fail "GET /ehr by the subject answering the EHR $K8S_INDEX_EHR" \
    "$code $(head -c 300 "$PROBE_TMP/index-lookup")" \
    "the local index finds the EHR whose EHR_STATUS names the subject"
}

# An IHE index: the EHR names the clinical pseudonym, and a cohort naming the
# patient by its source identifier finds it, which takes the CDR's own ITI-83
# ask of the PIX Manager (POST /query/cohort with `subjects`; GET /ehr reads
# the EHR_STATUS columns and translates nothing).
k8s_index_cohort_roundtrip() { # <pseudonym> <source-id>
  local code found
  k8s_index_create "$K8S_PIX_CLINICAL" "$1" || return 0
  code="$(curl -s -u "$K8S_BASIC" -X POST -H 'Content-Type: application/json' \
    -o "$PROBE_TMP/index-lookup" -w "$K8S_HTTP_CODE" \
    --data "$(jq -cn --arg id "$2" --arg ns "$K8S_PIX_SOURCE_SYSTEM" \
      '{q: "SELECT e/ehr_id/value FROM EHR e", subjects: [{id: $id, namespace: $ns}]}')" \
    "${K8S_CDR}/ferroehr/rest/extensions/v1/query/cohort")"
  found="$(jq -r '[.rows[]?[0]] | join(",")' "$PROBE_TMP/index-lookup" 2>/dev/null)"
  [[ "$found" == "$K8S_INDEX_EHR" ]] && return 0
  probe_fail "POST /query/cohort naming the source identifier answering the EHR $K8S_INDEX_EHR alone" \
    "$code $(head -c 300 "$PROBE_TMP/index-lookup")" \
    "the CDR asks the PIX Manager for the pseudonym and finds the EHR that names it"
}

# Registers a synthetic patient with FerroPIX as the source, asks it as the
# CDR's client for the clinical pseudonym (ITI-83), and runs the round trip:
# the EHR names the pseudonym, the lookup names the source identifier.
k8s_index_ihe_roundtrip() {
  local red="probe-$RANDOM$RANDOM" src consumer code pseudonym pix="http://127.0.0.1:${K8S_PIX_PORT}/fhir"
  if ! k8s_pix_pf_start; then
    probe_fail "a forward to a ready FerroPIX pod" "no FerroPIX pod answered /health/liveness"
    return 0
  fi
  src="$(k8s_pix_token "$K8S_PIX_SOURCE_CLIENT" system/Patient.cud)"
  consumer="$(k8s_pix_token ferroehr system/Patient.rs)"
  if [[ -z "$src" || -z "$consumer" ]]; then
    probe_fail "two client credentials tokens from the issuer beside FerroPIX" "no access_token"
    k8s_pix_pf_stop
    return 0
  fi
  code="$(curl -s -m 30 -o "$PROBE_TMP/pix-feed" -w "$K8S_HTTP_CODE" -X PUT \
    -H "Authorization: Bearer $src" -H 'Content-Type: application/fhir+json' \
    "$pix/Patient?identifier=$K8S_PIX_SOURCE_SYSTEM%7C$red" \
    --data "{\"resourceType\":\"Patient\",\"identifier\":[{\"system\":\"$K8S_PIX_SOURCE_SYSTEM\",\"value\":\"$red\"}],\"name\":[{\"family\":\"Probe\"}]}")"
  case "$code" in
    200 | 201) ;;
    *)
      probe_fail "the synthetic patient registered (ITI-104, 200 or 201)" \
        "$code $(head -c 200 "$PROBE_TMP/pix-feed")"
      k8s_pix_pf_stop
      return 0
      ;;
  esac
  pseudonym="$(curl -s -m 30 -G -H "Authorization: Bearer $consumer" -H 'Accept: application/fhir+json' \
    --data-urlencode "sourceIdentifier=$K8S_PIX_SOURCE_SYSTEM|$red" \
    --data-urlencode "targetSystem=$K8S_PIX_CLINICAL" "$pix/Patient/\$ihe-pix" \
    | jq -r --arg sys "$K8S_PIX_CLINICAL" '[.parameter[]? | select(.name == "targetIdentifier")
        | .valueIdentifier | select(.system == $sys) | .value] | if length == 1 then .[0] else empty end' 2>/dev/null)"
  k8s_pix_pf_stop
  if [[ -z "$pseudonym" ]]; then
    probe_fail "one pseudonym in $K8S_PIX_CLINICAL for the registered patient (ITI-83)" "none"
    return 0
  fi
  k8s_index_cohort_roundtrip "$pseudonym" "$red"
}

# The release's CDR on one EHR-index mode: install, roll out, forward.
k8s_index_install() { # <values-file> [helm args...]
  local values="$1"
  shift
  k8s_pf_stop
  k8s_install -f "$values" "$@" && k8s_rollout 300s && k8s_pf_start
}

probes_k8s_ehr_index() {
  bold "the EHR index, each mode the chart supports"
  local charts=(deploy/helm/ferroehr/charts/ferropix-*.tgz)
  K8S_PIX_CHART=""
  [[ -f "${charts[0]}" ]] && K8S_PIX_CHART="${charts[0]}"

  # The BROKEN state: FerroPIX off and no other identity source named.
  probe "P-K8S-INDEX-NEITHER" "$PROBE_BROKEN" "chart" "#5108" \
    "ferropix.enabled=false with neither pixm_url nor provider=local is refused at render, naming both"
  local guard
  guard="$(helm template "$K8S_RELEASE" deploy/helm/ferroehr -n "$K8S_NS" \
             -f "$K8S_AUTH_VALUES" --set config.ehr_index.provider=null 2>&1)"
  assert_contains "$guard" "config.ehr_index.pixm_url" \
    "the other PIX Manager is one way forward and the refusal must name its key"
  assert_contains "$guard" "config.ehr_index.provider=local" \
    "the in-instance index is the other and must be named too"
  probe_done

  # Mode 3 is the release every other family measures.
  probe "P-K8S-INDEX-LOCAL" "$PROBE_WORKING" "server" "#5108" \
    "provider: local with no FerroPIX: an EHR is found again by its subject"
  if k8s_index_install /dev/null; then
    k8s_index_local_roundtrip
  else
    probe_fail "the local-index release serving" "$K8S_HELM_REFUSED or no rollout"
  fi
  probe_done

  if [[ -z "$K8S_PIX_CHART" ]]; then
    uncovered "the two IHE modes (P-K8S-INDEX-PIXM, P-K8S-INDEX-FERROPIX)" \
      "the FerroPIX dependency chart is not under deploy/helm/ferroehr/charts/; run helm dependency build deploy/helm/ferroehr"
    return 0
  fi
  if ! k8s_pix_inputs; then
    uncovered "the two IHE modes (P-K8S-INDEX-PIXM, P-K8S-INDEX-FERROPIX)" \
      "FerroPIX's database or Secrets could not be created on the probe's PostgreSQL"
    return 0
  fi
  k8s_pix_issuer_route

  # Mode 2: FerroPIX as a release of its own, named by pixm_url.
  probe "P-K8S-INDEX-PIXM" "$PROBE_WORKING" "server" "#5108" \
    "ferropix.enabled=false with pixm_url naming a PIX Manager's Service: an EHR is found by its source identifier"
  k8s_pix_values > "$PROBE_TMP/pix-values.yaml"
  k8s_pix_cdr_values "    provider: ihe
    pixm_url: \"http://$K8S_PIX_RELEASE:8080/fhir\"" > "$PROBE_TMP/index-pixm.yaml"
  if ! helm upgrade --install "$K8S_PIX_RELEASE" "$K8S_PIX_CHART" -n "$K8S_NS" \
       -f "$PROBE_TMP/pix-values.yaml" >/dev/null 2>&1 || ! k8s_pix_rollout; then
    probe_fail "the FerroPIX release rolled out" \
      "$(kc logs -l "$K8S_PIX_LABEL" --tail=6 --all-containers 2>&1 | tail -6)"
  elif ! k8s_index_install "$PROBE_TMP/index-pixm.yaml"; then
    probe_fail "the CDR serving with pixm_url" \
      "$(kc logs -l app.kubernetes.io/name=ferroehr --tail=6 2>&1 | tail -6)"
  else
    k8s_index_ihe_roundtrip
  fi
  probe_done
  helm uninstall "$K8S_PIX_RELEASE" -n "$K8S_NS" --wait >/dev/null 2>&1 || true

  # Mode 1, the default: the chart deploys FerroPIX itself.
  probe "P-K8S-INDEX-FERROPIX" "$PROBE_WORKING" "server" "#5108" \
    "ferropix.enabled=true, the default: the CDR asks the FerroPIX the chart deploys, and an EHR is found by its source identifier"
  {
    echo "ferropix:"
    echo "  enabled: true"
    k8s_pix_values | sed 's/^/  /'
    k8s_pix_cdr_values ""
  } > "$PROBE_TMP/index-ferropix.yaml"
  # The chart refuses any provider beside ferropix.enabled, and a null over
  # the Basic-auth values' `provider: local` keeps the key, so this install
  # takes those values without their ehr_index block.
  awk '/^  ehr_index:/{skip=1; next} skip && /^  [^ ]|^[^ ]/{skip=0} !skip' \
    "$K8S_AUTH_VALUES" > "$PROBE_TMP/auth-no-index.yaml"
  if ! K8S_AUTH_VALUES="$PROBE_TMP/auth-no-index.yaml" k8s_index_install "$PROBE_TMP/index-ferropix.yaml" \
     || ! k8s_pix_rollout; then
    probe_fail "the CDR and the FerroPIX it deploys serving" \
      "$(kc logs -l "$K8S_PIX_LABEL" --tail=4 --all-containers 2>&1 | tail -4
         kc logs -l app.kubernetes.io/name=ferroehr --tail=4 2>&1 | tail -4)"
  else
    k8s_index_ihe_roundtrip
  fi
  probe_done

  # Back to the release every other family measures.
  k8s_index_install /dev/null >/dev/null 2>&1 || true
  kc delete service,networkpolicy "$K8S_PIX_ISSUER_SVC" --ignore-not-found >/dev/null 2>&1
}
