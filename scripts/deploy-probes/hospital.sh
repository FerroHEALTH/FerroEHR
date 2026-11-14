#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The hospital scenario family (#5610).
#
# One controller runs FerroEHR with Keycloak as its `persons` issuer, the
# quickstart's `machines` issuer for SCIM and its system clients, and two
# instances on one PostgreSQL. Everything is set up through public interfaces
# alone: SCIM for the people, the demographic API for their roles and care
# relationships, ITS-REST for the record and its EHR_ACCESS, and the admin API
# for the ACCESS_POLICY. Every assertion is observed over HTTP, half of them
# through the second instance, so a change made on one must decide the next
# request on the other.
#
# Sourced by scripts/deploy-probe.sh after oidc.sh, signing_pgp.sh and
# replicas.sh, whose helpers it reuses; never run directly.

# The second instance's port; Keycloak takes the replicas family's 18081.
HOSPITAL_REPLICA_PORT="${PROBE_HOSPITAL_REPLICA_PORT:-18082}"
HOSPITAL_KC_TLS_PORT="${PROBE_KC_TLS_PORT:-18443}"
H1="$CDR"
H2="http://localhost:${HOSPITAL_REPLICA_PORT}"
HB="/ferroehr/rest/openehr/v1"
HX="/ferroehr/rest/extensions/v1"
# The issuer the realm signs under, which the instances verify against.
H_REALM_ISSUER="https://keycloak.localhost:8443/auth/realms/ferroehr"
H_MACHINES="http://localhost:8180/quickstart"
H_PATIENTS="http://localhost:8180/patients"
H_ISSUER_HTTP="http://localhost:${FERROEHR_ISSUER_PORT}"
H_TERM="ferroehr-operations"
H_PURPOSE="x-purpose-of-use"

# ── The stack ─────────────────────────────────────────────────────────────────

# The overlay of this family, written to `out`: the realm's issuer, break-glass
# and the purposes on both instances, the second instance trusting the realm
# as the first does, the issuer minting system-client tokens for this
# audience and, with `patients` set, a `patients` issuer for the portal.
hospital_overlay() {
  local out="$1" patients="${2:-}"
  {
    cat <<'YAML'
x-hospital-env: &hospital-env
  FERROEHR__AUTH__ISSUER__1__AUDIENCE: ferroehr-scim,ferroehr-system
  FERROEHR__AUTH__ISSUER__2__ISSUER: https://keycloak.localhost:8443/auth/realms/ferroehr
  FERROEHR__AUTH__ISSUER__2__KIND: persons
  FERROEHR__AUTH__ISSUER__2__PROFILE: keycloak
  FERROEHR__AUTH__ISSUER__2__AUDIENCE: ferroehr
  FERROEHR__AUTH__ISSUER__2__APP_ONLY__0__CLAIM: client_id
  FERROEHR__AUTH__ISSUER__2__APP_ONLY__0__VALUE: ferroehr-service
  FERROEHR__AUTH__ISSUER__2__ROLE_CLAIMS: roles,groups,entitlements,realm_access.roles
  FERROEHR__AUTH__PARTY__1__ISSUER: https://keycloak.localhost:8443/auth/realms/ferroehr
  FERROEHR__AUTH__PARTY__1__IDENTIFIER_ISSUER: urn:ferroehr:keycloak-quickstart-login
  FERROEHR__AUTH__PARTY__1__IDENTIFIER_TYPE: login
  FERROEHR__AUTH__PARTY__1__GROUP_CLAIM: groups
  FERROEHR__AUTH__PARTY__1__GROUP_IDENTIFIER_ISSUER: urn:ferroehr:keycloak-quickstart-groups
  FERROEHR__AUTHZ__RBAC__ROLE_ISSUERS: https://keycloak.localhost:8443/auth/realms/ferroehr
  FERROEHR__AUTH__TLS__TRUST_ROOTS_FILE: /run/ferroehr-dev-tls/ca/ca.crt
  FERROEHR__AUTHZ__RBAC__ENABLED: "true"
  FERROEHR__AUTHZ__RBAC__EMERGENCY_ACCESS_ROLE: EMERGENCY
  FERROEHR__AUDIT__PURPOSE_CODES: TREAT,ETREAT
  FERROEHR__AUDIT__EMERGENCY_PURPOSE_CODES: ETREAT
  FERROEHR__AUTHZ__GRANTS__ALLOWED: commit_composition,query_system_wide,scim_provision,hold_scopes,sign_in_persons,access_service,write_access_policy,write_care_relationship
  FERROEHR__AUTH__SMART__DISCOVERY: "true"
YAML
    if [[ -n "$patients" ]]; then
      cat <<'YAML'
  FERROEHR__AUTH__ISSUER__3__ISSUER: http://localhost:8180/patients
  FERROEHR__AUTH__ISSUER__3__KIND: patients
  FERROEHR__AUTH__ISSUER__3__AUDIENCE: ferroehr-portal
  FERROEHR__AUTH__ISSUER__3__TOKEN_TYPES: JWT
  FERROEHR__AUTH__SMART__PATIENT_ISSUERS: http://localhost:8180/patients
  FERROEHR__AUTH__DESCRIBED_ISSUER: https://keycloak.localhost:8443/auth/realms/ferroehr
YAML
    fi
    cat <<'YAML'
services:
  identity-issuer:
    environment:
      JSON_CONFIG: >-
        {"tokenCallbacks":[{"issuerId":"quickstart",
        "requestMappings":[{"requestParam":"scope","match":"ferroehr/scim",
        "claims":{"sub":"$${clientId}","client_id":"$${clientId}",
        "aud":["ferroehr-scim"],"scope":"$${scope}"}},
        {"requestParam":"audience","match":"ferroehr-system",
        "claims":{"sub":"$${clientId}","client_id":"$${clientId}",
        "aud":["ferroehr-system"],"scope":"$${scope}"}},
        {"requestParam":"grant_type","match":"client_credentials",
        "claims":{"sub":"$${clientId}","client_id":"$${clientId}",
        "aud":["http://localhost:8090/fhir"],"scope":"$${scope}"}}]},
        {"issuerId":"patients",
        "requestMappings":[{"requestParam":"grant_type","match":"client_credentials",
        "claims":{"sub":"$${login}","client_id":"$${clientId}",
        "aud":["ferroehr-portal"],"scope":"$${scope}","ehrId":"$${ehr_id}"}}]}]}
  ferroehr:
    environment: *hospital-env
  ferroehr-2:
    depends_on:
      keycloak:
        condition: service_healthy
    environment: *hospital-env
    volumes:
      - keycloak-tls:/run/ferroehr-dev-tls:ro
YAML
  } > "$out"
  printf '%s' "$out"
}

# The compose files of the stack, with the portal's issuer when `patients` is
# set; the key overlay is written once per run.
hospital_files() {
  local patients="${1:-}"
  HOSPITAL_FILES=(-f docker-compose.yml -f docker-compose.keycloak.yml
    -f docker-compose.replicas.yml -f "$HOSPITAL_KEY_OVERLAY"
    -f "$(hospital_overlay "$PROBE_TMP/hospital${patients:+-patients}.yml" "$patients")")
}

hdc() { dc "${HOSPITAL_FILES[@]}" --profile replicas --profile s3 "$@"; }

# Brings the whole stack up on fresh volumes: Keycloak first, since its
# one-shot writes the CA both instances read at load, then both instances,
# the bucket and the provisioning of the Basic user and the realm's users.
hospital_up() {
  export FERROEHR_REPLICA_PORT="$HOSPITAL_REPLICA_PORT"
  export KEYCLOAK_TLS_PORT="$HOSPITAL_KC_TLS_PORT"
  # The overlay changes the identity issuer that both instances share a
  # network namespace with, so the quickstart goes down first, its volumes
  # with it: the hospital starts on a database of its own.
  hdc down -v --remove-orphans >/dev/null 2>&1 || true
  hdc up -d keycloak >/dev/null 2>&1
  hdc up -d --wait ferroehr ferroehr-2 keycloak seaweedfs seaweedfs-init \
    ferroehr-provision ferroehr-provision-keycloak >/dev/null 2>&1
  wait_http "$KC_REALM/.well-known/openid-configuration" 150 \
    && wait_http "$H1/health/readiness" 120 && wait_http "$H2/health/readiness" 120
}

# Restarts both instances on the compose files `hospital_files` set last.
hospital_restart() {
  hdc up -d --wait --force-recreate --no-deps ferroehr >/dev/null 2>&1
  hdc up -d --wait --force-recreate --no-deps ferroehr-2 >/dev/null 2>&1
  wait_http "$H1/health/readiness" 120 && wait_http "$H2/health/readiness" 120
}

# Takes the stack down with its volumes, so no other family inherits the
# hospital's parties, and brings the quickstart back as the families after
# this one expect it.
hospital_down() {
  hdc down -v --remove-orphans >/dev/null 2>&1 || true
  compose_up ferroehr seaweedfs seaweedfs-init ferroehr-provision
  wait_http "$CDR/health/readiness" 90 || true
}

# ── Calling the instances ─────────────────────────────────────────────────────

# The base URL of instance `n` (1 or 2).
h_base() { if [[ "$1" = 2 ]]; then printf '%s' "$H2"; else printf '%s' "$H1"; fi; }

# A password-grant token of the realm user `user` through the viewer client,
# the documented recipe of the overlay.
kc_token() {
  curl -s -d client_id=ferroehr-viewer -d client_secret=ferroehr-viewer-quickstart-secret \
    -d username="$1" -d password="$1" -d grant_type=password \
    "$KC_REALM/protocol/openid-connect/token" | jq -r '.access_token // empty'
}

# A token of the quickstart issuer for `client`, asked for `audience` with
# `scope`. The issuer names itself after the Host it is reached at, so the
# call names the host the instances know it by.
machine_token() {
  local client="$1" audience="$2" scope="$3"
  curl -s -H 'Host: localhost:8180' -d grant_type=client_credentials \
    -d client_id="$client" -d client_secret=hospital -d audience="$audience" \
    --data-urlencode scope="$scope" "$H_ISSUER_HTTP/quickstart/token" \
    | jq -r '.access_token // empty'
}

# A portal token of the `patients` issuer for the login `login`, its launch
# context the EHR `ehr`.
patient_token() {
  curl -s -H 'Host: localhost:8180' -d grant_type=client_credentials \
    -d client_id=hospital-portal -d client_secret=hospital -d login="$1" -d ehr_id="$2" \
    --data-urlencode scope="openid launch/patient patient/composition-*.rs patient/aql-*.rs" \
    "$H_ISSUER_HTTP/patients/token" | jq -r '.access_token // empty'
}

# The Authorization header value of the caller `who`: `admin` (the Basic
# first administrator), `kc:<user>` (a realm user), `sys:<client>` (a system
# client), `scim` (the realm's provisioning client) or `pt:<login>:<ehr>` (a
# portal login).
h_auth() {
  local who="$1"
  case "$who" in
    admin) printf 'Basic %s' "$(printf 'ferroehr:ferroehr' | base64)" ;;
    kc:*) printf 'Bearer %s' "$(kc_token "${who#kc:}")" ;;
    sys:*) printf 'Bearer %s' "$(machine_token "${who#sys:}" ferroehr-system \
      'system/composition-*.cruds system/aql-*.cruds system/template-*.r')" ;;
    scim) printf 'Bearer %s' "$(curl -s -H 'Host: localhost:8180' -d grant_type=client_credentials \
      -d client_id=quickstart-scim-keycloak -d client_secret=quickstart -d scope=ferroehr/scim \
      "$H_ISSUER_HTTP/quickstart/token" | jq -r '.access_token // empty')" ;;
    pt:*) local rest="${who#pt:}"; printf 'Bearer %s' "$(patient_token "${rest%%:*}" "${rest#*:}")" ;;
    *) printf 'none' ;;
  esac
}

# h_call <instance> <who> <method> <path> [body] [purpose] [content-type]
#
# Sends one request and leaves the status in H_CODE, the body in H_BODY, the
# response headers in H_HEADERS and the request id in H_RID. A path that
# starts with `/scim` or `/ferroehr` is taken as is; any other is below the
# openEHR base.
h_call() {
  local n="$1" who="$2" method="$3" path="$4" body="${5:-}" purpose="${6:-}"
  local type="${7:-application/json}"
  case "$path" in
    /scim*|/ferroehr*) ;;
    *) path="$HB$path" ;;
  esac
  local -a args=(-s -X "$method" -H "Authorization: $(h_auth "$who")"
    -H 'Accept: application/json' -H 'Prefer: return=representation'
    -D "$PROBE_TMP/h-headers" -o "$PROBE_TMP/h-body" -w '%{http_code}')
  [[ -n "$purpose" ]] && args+=(-H "$H_PURPOSE: $purpose")
  [[ -n "${H_IF_MATCH:-}" ]] && args+=(-H "If-Match: $H_IF_MATCH")
  [[ -n "${H_IF_NONE_MATCH:-}" ]] && args+=(-H "If-None-Match: $H_IF_NONE_MATCH")
  [[ -n "$body" ]] && args+=(-H "Content-Type: $type" --data-binary "$body")
  H_CODE="$(curl "${args[@]}" "$(h_base "$n")$path")"
  H_BODY="$(cat "$PROBE_TMP/h-body" 2>/dev/null)"
  H_HEADERS="$(tr -d '\r' < "$PROBE_TMP/h-headers" 2>/dev/null)"
  H_RID="$(grep -i '^x-request-id:' <<<"$H_HEADERS" | awk '{print $2}' | tail -1)"
}

# The versioned-object id an ETag or Location of the last call names.
h_vo() {
  grep -i '^etag:' <<<"$H_HEADERS" | sed -E 's/.*"([0-9a-f-]+)::.*/\1/' | tail -1
}

# The full version uid the ETag of the last call names.
h_version() {
  grep -i '^etag:' <<<"$H_HEADERS" | sed -E 's/^[^"]*"([^"]+)".*/\1/' | tail -1
}

# The refusal code a 403 of the last call carries.
h_refusal() { jq -r '.refusal // empty' <<<"$H_BODY" 2>/dev/null; }

# ── JSON ──────────────────────────────────────────────────────────────────────

# A DV_CODED_TEXT of `terminology` and `code`.
j_coded() {
  jq -nc --arg t "$1" --arg c "$2" '{_type:"DV_CODED_TEXT",value:$c,
    defining_code:{_type:"CODE_PHRASE",terminology_id:{_type:"TERMINOLOGY_ID",value:$t},code_string:$c}}'
}

# A term of FerroEHR's access control groups.
j_term() { j_coded "$H_TERM" "$1"; }

# A PARTY_REF to the party `id` of RM type `type`.
j_party() {
  jq -nc --arg t "$1" --arg i "$2" \
    '{_type:"PARTY_REF",namespace:"demographic",type:$t,id:{_type:"HIER_OBJECT_ID",value:$i}}'
}

# A DV_INTERVAL<DV_DATE> from `lower` to `upper`, unbounded above when
# `upper` is empty.
j_dates() {
  jq -nc --arg l "$1" --arg u "${2:-}" '{_type:"DV_INTERVAL",
    lower:{_type:"DV_DATE",value:$l},lower_unbounded:false,lower_included:true}
    + (if $u == "" then {upper_unbounded:true,upper_included:false}
       else {upper:{_type:"DV_DATE",value:$u},upper_unbounded:false,upper_included:true} end)'
}

# The date `days` days from today in UTC (negative for the past).
h_day() {
  local days="$1"
  [[ "$days" = -* ]] || days="+$days"
  if date -u -v+0d +%F >/dev/null 2>&1; then
    date -u -v"${days}"d +%F
  else
    date -u -d "${days} days" +%F
  fi
}

# A party of RM type `type` named `name` with an archetype of this family,
# with the extra members `extra` merged in.
j_named_party() {
  local type="$1" name="$2" extra="${3:-}"
  [[ -n "$extra" ]] || extra='{}'
  local arch
  arch="openEHR-DEMOGRAPHIC-${type}.hospital_$(tr '[:upper:]' '[:lower:]' <<<"$type").v1"
  jq -nc --arg t "$type" --arg n "$name" --arg a "$arch" --argjson x "$extra" '{_type:$t,
    archetype_node_id:$a,
    archetype_details:{_type:"ARCHETYPED",archetype_id:{_type:"ARCHETYPE_ID",value:$a},rm_version:"1.1.0"},
    name:{_type:"DV_TEXT",value:$n},
    identities:[{_type:"PARTY_IDENTITY",archetype_node_id:"at0001",name:{_type:"DV_TEXT",value:"name"},
      details:{_type:"ITEM_TREE",archetype_node_id:"at0002",name:{_type:"DV_TEXT",value:"name"},
        items:[{_type:"ELEMENT",archetype_node_id:"at0003",name:{_type:"DV_TEXT",value:"name"},
          value:{_type:"DV_TEXT",value:$n}}]}}]} + $x'
}

# A relationship carrying the care code `code` from `source` (of RM type
# `stype`) to the party `target` (of RM type `ttype`) over `validity`.
j_care() {
  local code="$1" stype="$2" source="$3" ttype="$4" target="$5" validity="$6"
  jq -nc --argjson k "$(j_term "$code")" --argjson s "$(j_party "$stype" "$source")" \
    --argjson t "$(j_party "$ttype" "$target")" --argjson v "$validity" '{_type:"PARTY_RELATIONSHIP",
    archetype_node_id:"openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.hospital_care.v1",
    name:{_type:"DV_TEXT",value:"care"},
    details:{_type:"ITEM_TREE",archetype_node_id:"at0002",name:{_type:"DV_TEXT",value:"details"},
      items:[{_type:"ELEMENT",archetype_node_id:"at0003",name:{_type:"DV_TEXT",value:"kind"},value:$k}]},
    time_validity:$v,source:$s,target:$t}'
}

# A credentials element named by the code `name` holding `value`.
j_qualifier() {
  jq -nc --argjson n "$(j_term "$1")" --argjson v "$2" \
    '{_type:"ELEMENT",archetype_node_id:"at0040",name:$n,value:$v}'
}

# The operation element of a capability granting `operation`.
j_operation() {
  jq -nc --argjson v "$(j_term "$1")" \
    '{_type:"ELEMENT",archetype_node_id:"at0022",name:{_type:"DV_TEXT",value:"operation"},value:$v}'
}

# A CAPABILITY holding the credentials elements given as JSON arguments.
j_capability() {
  local items
  items="$(printf '%s\n' "$@" | jq -sc '.')"
  jq -nc --argjson i "$items" '{_type:"CAPABILITY",archetype_node_id:"at0020",
    name:{_type:"DV_TEXT",value:"grant"},
    credentials:{_type:"ITEM_TREE",archetype_node_id:"at0021",name:{_type:"DV_TEXT",value:"terms"},items:$i}}'
}

# A ROLE named `name` the party `performer` of RM type `ptype` performs,
# holding the capabilities given as JSON arguments.
j_role() {
  local name="$1" ptype="$2" performer="$3"
  shift 3
  local caps
  caps="$(printf '%s\n' "$@" | jq -sc '.')"
  j_named_party ROLE "$name" "$(jq -nc --argjson p "$(j_party "$ptype" "$performer")" \
    --argjson c "$caps" '{performer:$p,capabilities:$c}')"
}

# The clearance and purpose elements: `sensitivity_clearance` for each HL7 v3
# Confidentiality code in `clearances` (space separated) and
# `permitted_purpose` for each PurposeOfUse code in `purposes`.
j_facts() {
  local clearances="$1" purposes="$2" code
  for code in $clearances; do
    j_qualifier sensitivity_clearance "$(j_coded 2.16.840.1.113883.5.25 "$code")"
  done
  for code in $purposes; do
    j_qualifier permitted_purpose "$(j_coded 2.16.840.1.113883.1.11.20448 "$code")"
  done
}

# ── Setting the hospital up ───────────────────────────────────────────────────

# The ids the setup records, by name: realm subjects, PERSONs, parties, EHRs
# and compositions. Bash 3 has no associative arrays, so each is a variable
# `HV_<name>`.
hv() { eval "printf '%s' \"\${HV_$1:-}\""; }
hv_set() { eval "HV_$1=\"\$2\""; }

# Fails the running probe with what the last call answered.
h_fail_call() {
  probe_fail "$1" "$H_CODE ${H_BODY:0:400}" "${2:-request id $H_RID}"
}

# A realm admin token of the master realm.
kc_admin() {
  curl -s -d client_id=admin-cli -d username="${KEYCLOAK_ADMIN_USER:-admin}" \
    -d password="${KEYCLOAK_ADMIN_PASSWORD:-admin}" -d grant_type=password \
    "$KC/auth/realms/master/protocol/openid-connect/token" | jq -r '.access_token // empty'
}

# Creates the realm role `role`.
kc_role() {
  curl -s -o /dev/null -X POST -H "Authorization: Bearer $(kc_admin)" \
    -H 'Content-Type: application/json' -d "{\"name\":\"$1\"}" "$KC/auth/admin/realms/ferroehr/roles"
}

# Creates the realm user `user` (password = user) named `first last` holding
# the realm roles given after; records its subject as HV_sub_<user>.
kc_user() {
  local user="$1" first="$2" last="$3"
  shift 3
  local token id role roles
  token="$(kc_admin)"
  curl -s -o /dev/null -X POST -H "Authorization: Bearer $token" -H 'Content-Type: application/json' \
    -d "{\"username\":\"$user\",\"enabled\":true,\"firstName\":\"$first\",\"lastName\":\"$last\",
      \"email\":\"$user@example.test\",\"emailVerified\":true,
      \"credentials\":[{\"type\":\"password\",\"value\":\"$user\",\"temporary\":false}]}" \
    "$KC/auth/admin/realms/ferroehr/users"
  id="$(curl -s -H "Authorization: Bearer $token" \
    "$KC/auth/admin/realms/ferroehr/users?username=$user&exact=true" | jq -r '.[0].id // empty')"
  roles="[]"
  for role in "$@"; do
    roles="$(jq -c --argjson r "$(curl -s -H "Authorization: Bearer $token" \
      "$KC/auth/admin/realms/ferroehr/roles/$role")" '. + [$r]' <<<"$roles")"
  done
  curl -s -o /dev/null -X POST -H "Authorization: Bearer $token" -H 'Content-Type: application/json' \
    -d "$roles" "$KC/auth/admin/realms/ferroehr/users/$id/role-mappings/realm"
  hv_set "sub_$user" "$id"
}

# A SCIM User for the realm user `user`, through instance `n`; records the
# PERSON as HV_person_<user>.
scim_user() {
  local n="$1" user="$2" formatted="$3"
  h_call "$n" scim POST /scim/v2/Users "$(jq -nc --arg u "$user" --arg f "$formatted" \
    --arg k "$(hv "sub_$user")" '{schemas:["urn:ietf:params:scim:schemas:core:2.0:User",
      "urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:User"],userName:$u,name:{formatted:$f},
      "urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:User":{loginIdentifier:$k}}')" "" \
    application/scim+json
  hv_set "person_$user" "$(jq -r '.id // empty' <<<"$H_BODY")"
  [[ "$H_CODE" = 201 ]]
}

# Creates a demographic party of `kind` (person, group, organisation, agent,
# role) from `body` as the administrator; records its id as HV_<name>.
h_party() {
  local name="$1" kind="$2" body="$3"
  h_call 1 admin POST "/demographic/$kind" "$body"
  hv_set "$name" "$(h_vo)"
  [[ "$H_CODE" = 201 ]]
}

# Adds the relationships `rels` (a JSON array) to the party `id` of `kind`
# through instance `n` as `who`, over its current version.
h_relate() {
  local n="$1" who="$2" kind="$3" id="$4" rels="$5" body version
  h_call "$n" admin GET "/demographic/$kind/$id"
  version="$(jq -r '.uid.value' <<<"$H_BODY")"
  body="$(jq -c --argjson r "$rels" 'del(.uid) | .relationships = ((.relationships // []) + $r)' <<<"$H_BODY")"
  H_IF_MATCH="\"$version\"" h_call "$n" "$who" PUT "/demographic/$kind/$version" "$body"
  [[ "$H_CODE" = 200 || "$H_CODE" = 204 ]]
}

# Replaces the relationships of the party `id` of `kind` by `rels` (a JSON
# array), through instance `n` as `who`.
h_rerelate() {
  local n="$1" who="$2" kind="$3" id="$4" rels="$5" body version
  h_call "$n" admin GET "/demographic/$kind/$id"
  version="$(jq -r '.uid.value' <<<"$H_BODY")"
  body="$(jq -c --argjson r "$rels" 'del(.uid) | .relationships = $r' <<<"$H_BODY")"
  H_IF_MATCH="\"$version\"" h_call "$n" "$who" PUT "/demographic/$kind/$version" "$body"
  [[ "$H_CODE" = 200 || "$H_CODE" = 204 ]]
}

# The templates of the record, each a vendored OPT, by the role it plays.
H_OPTS="corpus/fixtures/service/knowledge/opt"
H_T_ENCOUNTER="minimal_evaluation.en.v1"
H_T_VITALS="Vital Signs Encounter (Composition)"
H_T_MENTAL="RIPPLE - Clinical Notes.v1"
H_T_HIV="IDCR - Laboratory Test Report.v0"
H_T_LAB="minimal_observation.en.v1"
H_T_DIAGNOSIS="IDCR - Problem List.v1"

# An EHR whose EHR_STATUS names the PERSON `person` as its subject; records
# the EHR as HV_ehr_<name>.
h_ehr() {
  local name="$1" person="$2"
  h_call 1 admin POST /ehr "$(jq -nc --argjson s "$(j_party PERSON "$person")" '{_type:"EHR_STATUS",
    archetype_node_id:"openEHR-EHR-EHR_STATUS.generic.v1",name:{_type:"DV_TEXT",value:"EHR status"},
    archetype_details:{_type:"ARCHETYPED",rm_version:"1.1.0",
      archetype_id:{_type:"ARCHETYPE_ID",value:"openEHR-EHR-EHR_STATUS.generic.v1"}},
    subject:{_type:"PARTY_SELF",external_ref:$s},is_queryable:true,is_modifiable:true}')"
  hv_set "ehr_$name" "$(jq -r '.ehr_id.value // empty' <<<"$H_BODY")"
  [[ "$H_CODE" = 201 ]]
}

# The example composition of the template `template`, as the server makes it.
h_example() {
  curl -s -u "$BASIC" -H 'Accept: application/json' \
    "$H1$HB/definition/template/adl1.4/$(jq -rn --arg t "$1" '$t|@uri')/example"
}

# Commits the example of `template` into the EHR `ehr` through instance `n`
# as `who`, declaring `purpose`; records the composition as HV_<name>.
h_commit() {
  local name="$1" n="$2" who="$3" ehr="$4" template="$5" purpose="${6:-TREAT}"
  h_call "$n" "$who" POST "/ehr/$ehr/composition" "$(h_example "$template")" "$purpose"
  hv_set "$name" "$(h_vo)"
  [[ "$H_CODE" = 201 ]]
}

# A system client AGENT carrying `client` under the machines issuer, acting
# for the ORGANISATION `org`, with a ROLE holding the capabilities given
# after; records the AGENT as HV_<name>.
h_client() {
  local name="$1" client="$2" org="$3"
  shift 3
  h_party "$name" agent "$(j_named_party AGENT "$client" "$(jq -nc --arg c "$client" --arg i "$H_MACHINES" \
    '{details:{_type:"ITEM_TREE",archetype_node_id:"at0010",name:{_type:"DV_TEXT",value:"details"},
      items:[{_type:"ELEMENT",archetype_node_id:"at0011",name:{_type:"DV_TEXT",value:"client"},
        value:{_type:"DV_IDENTIFIER",issuer:$i,id:$c,type:"client_id"}}]}}')")" || return 1
  h_relate 1 admin agent "$(hv "$name")" \
    "[$(j_care acts_for AGENT "$(hv "$name")" ORGANISATION "$org" "$(j_dates "$(h_day -365)")")]" || return 1
  h_party "${name}_role" role "$(j_role "$client grants" AGENT "$(hv "$name")" "$@")"
}

# A ROLE of the PERSON of `user` carrying the clearances and purposes given;
# records it as HV_role_<user>.
h_facts_role() {
  local user="$1" clearances="$2" purposes="$3"
  # shellcheck disable=SC2046 # one JSON element per word is the intent
  h_party "role_$user" role "$(j_role "$user caller facts" PERSON "$(hv "person_$user")" \
    "$(j_capability $(j_facts "$clearances" "$purposes"))")"
}

# Creates the people, the parties, the record and the controller's policy;
# each step that does not hold fails the setup probe with what it answered.
hospital_setup() {
  local user
  kc_role CLINICIAN
  kc_role NURSE
  kc_role EMERGENCY
  kc_user anna Anna Jansen CLINICIAN
  kc_user bram Bram "de Vries" NURSE
  kc_user carla Carla Smit CLINICIAN
  kc_user dirk Dirk Bakker CLINICIAN EMERGENCY
  kc_user gina Gina Mulder NURSE
  kc_user hans Hans Hoekstra ADMIN
  for user in anna bram carla dirk gina hans; do
    [[ -n "$(hv "sub_$user")" ]] || probe_fail "a realm user $user" "no subject"
  done
  # SCIM through both instances: the client's view must be one deployment.
  scim_user 1 anna "Dr. Anna Jansen" || h_fail_call "SCIM creates anna"
  scim_user 2 bram "Bram de Vries" || h_fail_call "SCIM creates bram"
  scim_user 1 carla "Dr. Carla Smit" || h_fail_call "SCIM creates carla"
  scim_user 2 dirk "Dr. Dirk Bakker" || h_fail_call "SCIM creates dirk"
  scim_user 1 gina "Gina Mulder" || h_fail_call "SCIM creates gina"
  scim_user 2 hans "Hans Hoekstra" || h_fail_call "SCIM creates hans"

  h_party cardiology group "$(j_named_party GROUP Cardiology)" || h_fail_call "the Cardiology team"
  h_party ward3 group "$(j_named_party GROUP "Ward 3")" || h_fail_call "the Ward 3 team"
  h_party psychiatry group "$(j_named_party GROUP Psychiatry)" || h_fail_call "the Psychiatry team"
  h_party region group "$(j_named_party GROUP "Regional health network")" || h_fail_call "the regional GROUP"
  h_party hospital organisation "$(j_named_party ORGANISATION "Sint Example Ziekenhuis")" \
    || h_fail_call "the controller"
  local org
  for org in lab_org member_org outside_org; do
    h_party "$org" organisation "$(j_named_party ORGANISATION "$org")" || h_fail_call "the $org"
  done
  for org in lab_org member_org; do
    h_relate 1 admin organisation "$(hv "$org")" \
      "[$(j_care member_of ORGANISATION "$(hv "$org")" GROUP "$(hv region)" "$(j_dates "$(h_day -365)")")]" \
      || h_fail_call "the $org joins the regional GROUP"
  done

  # The callers' facts: clearances and purposes on their ROLEs, care teams
  # in their PERSONs, written by the administrator.
  h_facts_role anna "N R" TREAT || h_fail_call "Dr. Jansen's clearances"
  h_facts_role bram "N" TREAT || h_fail_call "Nurse de Vries's clearances"
  h_facts_role carla "N R V" TREAT || h_fail_call "Dr. Smit's clearances"
  h_facts_role dirk "N" "TREAT ETREAT" || h_fail_call "Dr. Bakker's clearances"
  h_facts_role gina "N" TREAT || h_fail_call "Gina's clearances"
  # The controller's administrator, the realm's ferroehr user, writes the
  # policy; the policy route asks for its own grant.
  h_call 1 scim GET "/scim/v2/Users?filter=$(jq -rn '"userName eq \"keycloak-ferroehr\""|@uri')"
  hv_set person_controller "$(jq -r '.Resources[0].id // empty' <<<"$H_BODY")"
  h_party role_controller_policy role "$(j_role "controller administrator" PERSON "$(hv person_controller)" \
    "$(j_capability "$(j_operation write_access_policy)")")" || h_fail_call "the controller's policy grant"
  local since
  since="$(j_dates "$(h_day -30)")"
  h_relate 2 admin person "$(hv person_anna)" \
    "[$(j_care member_of PERSON "$(hv person_anna)" GROUP "$(hv cardiology)" "$since")]" \
    || h_fail_call "Dr. Jansen joins Cardiology"
  h_relate 1 admin person "$(hv person_bram)" \
    "[$(j_care member_of PERSON "$(hv person_bram)" GROUP "$(hv ward3)" "$since")]" \
    || h_fail_call "Nurse de Vries joins Ward 3"
  h_relate 2 admin person "$(hv person_gina)" \
    "[$(j_care member_of PERSON "$(hv person_gina)" GROUP "$(hv ward3)" "$since")]" \
    || h_fail_call "Gina joins Ward 3"
  h_relate 1 admin person "$(hv person_carla)" \
    "[$(j_care member_of PERSON "$(hv person_carla)" GROUP "$(hv psychiatry)" "$since")]" \
    || h_fail_call "Dr. Smit joins Psychiatry"

  # The system clients: the lab acts for the lab, a member of the region;
  # the HIE client for a member provider; the other for a provider outside;
  # the access service for the member provider too.
  local query treat
  query="$(j_operation query_system_wide)"
  treat="$(j_qualifier permitted_purpose "$(j_coded 2.16.840.1.113883.1.11.20448 TREAT)")"
  h_client lab lab-system "$(hv lab_org)" "$(j_capability "$query" "$treat")" \
    "$(j_capability "$(j_operation commit_composition)" \
      "$(j_qualifier template "$(jq -nc --arg t "$H_T_LAB" '{_type:"DV_TEXT",value:$t}')")" \
      "$(j_qualifier origin "$(jq -nc '{_type:"DV_TEXT",value:"relay"}')")" \
      "$(jq -nc --argjson n "$(j_term facility)" --argjson i "$(j_term issuer)" \
        --argjson d "$(j_term identifier)" '{_type:"CLUSTER",archetype_node_id:"at0041",name:$n,
        items:[{_type:"ELEMENT",archetype_node_id:"at0042",name:$i,value:{_type:"DV_TEXT",value:"urn:oid:2.999.7.9"}},
          {_type:"ELEMENT",archetype_node_id:"at0043",name:$d,value:{_type:"DV_TEXT",value:"lab-1"}}]}')")" \
    || h_fail_call "the lab system client"
  h_client hie hie-member "$(hv member_org)" "$(j_capability "$query" "$treat")" \
    || h_fail_call "the regional HIE client"
  h_client outside hie-outside "$(hv outside_org)" "$(j_capability "$query" "$treat")" \
    || h_fail_call "the client outside the region"
  h_client access access-service "$(hv member_org)" \
    "$(j_capability "$query" "$treat")" "$(j_capability "$(j_operation access_service)")" \
    || h_fail_call "the access service"

  # The patients: Eva under the care of three teams since ten days ago,
  # Frank her representative, Gina a patient too, and Piet, anybody else.
  h_party person_eva person "$(j_named_party PERSON "Eva Visser")" || h_fail_call "Eva's PERSON"
  h_party person_piet person "$(j_named_party PERSON "Piet Jansen")" || h_fail_call "Piet's PERSON"
  local admitted team
  admitted="$(j_dates "$(h_day -10)")"
  H_EVA_CARE="[]"
  for team in cardiology ward3 psychiatry; do
    H_EVA_CARE="$(jq -c --argjson r "$(j_care under_care_of PERSON "$(hv person_eva)" GROUP "$(hv "$team")" \
      "$admitted")" '. + [$r]' <<<"$H_EVA_CARE")"
  done
  h_rerelate 2 admin person "$(hv person_eva)" "$H_EVA_CARE" || h_fail_call "Eva's admission"
  h_call 1 admin GET "/demographic/person/$(hv person_anna)"
  local frank
  frank="$(jq -c --arg i "$H_PATIENTS" --argjson r "$(j_care represents PERSON 00000000-0000-0000-0000-000000000000 \
    PERSON "$(hv person_eva)" "$(j_dates "$(h_day -365)")")" 'del(.uid) | del(.relationships)
    | .name.value = "Frank Visser"
    | .details.items = [.details.items[0] | .value = {_type:"DV_IDENTIFIER",issuer:$i,id:"frank",type:"login"}]
    | .identities[0].details.items[0].value.value = "Frank Visser"' <<<"$H_BODY")"
  h_party person_frank person "$frank" || h_fail_call "Frank's PERSON"
  h_rerelate 1 admin person "$(hv person_frank)" "[$(j_care represents PERSON "$(hv person_frank)" \
    PERSON "$(hv person_eva)" "$(j_dates "$(h_day -365)")")]" || h_fail_call "Frank represents Eva"

  h_ehr eva "$(hv person_eva)" || h_fail_call "Eva's EHR"
  h_ehr gina "$(hv person_gina)" || h_fail_call "Gina's EHR"
  h_ehr piet "$(hv person_piet)" || h_fail_call "Piet's EHR"

  local file
  for file in "$H_T_ENCOUNTER" "$H_T_VITALS" "$H_T_MENTAL" "$H_T_HIV" "$H_T_LAB" "$H_T_DIAGNOSIS"; do
    case "$file" in
      minimal_evaluation.en.v1) file=minimal_evaluation ;;
      minimal_observation.en.v1) file=minimal_observation ;;
      *) ;;
    esac
    h_call 1 admin POST /definition/template/adl1.4 "@$H_OPTS/$file.opt" "" application/xml
    [[ "$H_CODE" = 201 || "$H_CODE" = 409 ]] || h_fail_call "the template $file"
  done

  # The record, each part committed by the professional who wrote it.
  local eva gina piet
  eva="$(hv ehr_eva)"
  gina="$(hv ehr_gina)"
  piet="$(hv ehr_piet)"
  h_commit c_encounter 1 kc:anna "$eva" "$H_T_ENCOUNTER" || h_fail_call "the cardiology encounter"
  h_commit c_vitals 2 kc:bram "$eva" "$H_T_VITALS" || h_fail_call "the vital signs"
  h_commit c_mental 1 kc:carla "$eva" "$H_T_MENTAL" || h_fail_call "the mental-health note"
  h_commit c_hiv 2 kc:anna "$eva" "$H_T_HIV" || h_fail_call "the HIV result"
  h_commit c_diagnosis 1 kc:anna "$eva" "$H_T_DIAGNOSIS" || h_fail_call "the diagnosis"
  h_commit c_gina 2 kc:anna "$gina" "$H_T_VITALS" || h_fail_call "Gina's vital signs"
  h_commit c_piet 1 kc:anna "$piet" "$H_T_VITALS" || h_fail_call "Piet's vital signs"
}

# ── The controller's policy and Eva's directives ──────────────────────────────

# A sensitivity of the HL7 v3 Confidentiality codes.
j_sensitivity() { j_coded 2.16.840.1.113883.5.25 "$1"; }

# The controller's ACCESS_POLICY: the mental-health note very restricted, the
# HIV result restricted, a care relationship with a one-day grace and a
# purpose required, and the subject view withholding the roles of a person
# who accessed (EHDS Art. 9(3)).
j_policy() {
  local arch="openEHR-EHR-ACCESS_POLICY.controller.v1"
  jq -nc --arg a "$arch" --argjson c "$(j_party ORGANISATION "$(hv hospital)")" \
    --arg mental "$H_T_MENTAL" --arg hiv "$H_T_HIV" \
    --argjson v "$(j_sensitivity V)" --argjson r "$(j_sensitivity R)" \
    --argjson person "$(j_coded openehr person)" --argjson roles "$(j_term accessor_roles)" \
    --argjson ground "$(j_coded urn:example:national-law art-9-3)" '{_type:"ACCESS_POLICY",
    name:{_type:"DV_TEXT",value:"Sint Example Ziekenhuis access policy"},archetype_node_id:$a,
    archetype_details:{_type:"ARCHETYPED",archetype_id:{_type:"ARCHETYPE_ID",value:$a},rm_version:"1.2.0"},
    controller:$c,
    classifications:[
      {_type:"ACCESS_CLASSIFICATION",name:{_type:"DV_TEXT",value:"Mental health"},archetype_node_id:"at0002",
       data:{_type:"ACCESS_DATA_SCOPE",templates:[$mental]},sensitivities:[$v]},
      {_type:"ACCESS_CLASSIFICATION",name:{_type:"DV_TEXT",value:"HIV results"},archetype_node_id:"at0002",
       data:{_type:"ACCESS_DATA_SCOPE",templates:[$hiv]},sensitivities:[$r]}],
    care_relationship_required:true,
    relationship_grace:{_type:"DV_DURATION",value:"P1D"},
    purpose_required:true,
    log_restrictions:[{_type:"ACCESS_LOG_RESTRICTION",name:{_type:"DV_TEXT",value:"Roles withheld"},
      archetype_node_id:"at0003",accessor_kinds:[$person],withheld:[$roles],legal_grounds:[$ground]}]}'
}

# A DV_INTERVAL<DV_DATE_TIME> from `lower` to `upper` (UTC dates), each a
# whole day; an empty `upper` is unbounded.
j_instants() {
  jq -nc --arg l "$1T00:00:00Z" --arg u "${2:+$2T23:59:59Z}" '{_type:"DV_INTERVAL",
    lower:{_type:"DV_DATE_TIME",value:$l},lower_unbounded:false,lower_included:true}
    + (if $u == "" then {upper_unbounded:true,upper_included:false}
       else {upper:{_type:"DV_DATE_TIME",value:$u},upper_unbounded:false,upper_included:true} end)'
}

# A directive of `kind` and `effect` named `name`, with `extra` merged in.
j_directive() {
  local kind="$1" effect="$2" name="$3" extra="${4:-}"
  [[ -n "$extra" ]] || extra='{}'
  jq -nc --arg n "$name" --argjson k "$(j_term "$kind")" --argjson e "$(j_term "$effect")" \
    --argjson x "$extra" '{_type:"ACCESS_DIRECTIVE",name:{_type:"DV_TEXT",value:$n},
    archetype_node_id:"at0001",kind:$k,effect:$e,emergency_override:true} + $x'
}

# An actor scope naming the party `id` of RM type `type`.
j_naming() { jq -nc --argjson p "$(j_party "$1" "$2")" '[{party:$p}]'; }

# Eva's directives: Gina restricted, mental health denied except to Dr. Smit,
# the regional consent for treatment from `consent_from` to `consent_to`,
# and Frank's delegation; with `opt_out` set,
# her opt-out from the access services too.
j_eva_settings() {
  local consent_from="$1" consent_to="$2" opt_out="${3:-}"
  local -a directives=(
    "$(j_directive restriction deny "Gina restricted" "{\"applies_to\":$(j_naming PERSON "$(hv person_gina)")}")"
    "$(j_directive consent deny "Mental health, except Dr. Smit" "$(jq -nc --arg t "$H_T_MENTAL" \
      --argjson x "$(j_directive consent permit "Dr. Smit" "{\"applies_to\":$(j_naming PERSON "$(hv person_carla)")}")" \
      '{data:[{templates:[$t]}],exceptions:[$x]}')")"
    "$(j_directive consent permit "Regional sharing" "$(jq -nc --argjson g "$(j_naming GROUP "$(hv region)")" \
      --argjson p "$(j_coded 2.16.840.1.113883.1.11.20448 TREAT)" \
      --argjson v "$(j_instants "$consent_from" "$consent_to")" \
      '{applies_to:$g,purposes:[$p],time_validity:$v,emergency_override:false,
        identifiers:[{_type:"DV_IDENTIFIER",issuer:"urn:oid:2.999.7.1",id:"regional-opt-in",type:"BPPC"}]}')")"
    "$(j_directive delegation permit "Frank represents Eva" "$(jq -nc --argjson p "$(j_party PERSON "$(hv person_frank)")" \
      --argjson r "$(j_term representative)" '{applies_to:[{party:$p,relationship:$r}],emergency_override:false}')")"
  )
  [[ -n "$opt_out" ]] && directives+=("$(j_directive opt_out deny "Opt-out from the access services")")
  printf '%s\n' "${directives[@]}" | jq -sc --argjson d "$(j_term restricted)" \
    '{_type:"ACCESS_DIRECTIVE_SETTINGS",default_access:$d,gate_keepers:[{_type:"PARTY_SELF"}],directives:.}'
}

# Commits `settings` as the EHR_ACCESS settings of `ehr` through instance
# `n` as `who`, her own gate-keeper, in a CONTRIBUTION over the current
# version (RM EHR IM, EHR_ACCESS "versioned via the normal mechanism").
h_settings() {
  local n="$1" who="$2" ehr="$3" settings="$4" current
  h_call "$n" "$who" GET "$HX/ehr/$ehr/ehr_access"
  current="$(jq -r '.uid.value // empty' <<<"$H_BODY")"
  [[ -n "$current" ]] || current="$(h_version)"
  h_call "$n" "$who" POST "/ehr/$ehr/contribution" "$(jq -nc --arg p "$current" --argjson s "$settings" \
    --argjson m "$(j_coded openehr 251)" --argjson c "$(j_coded openehr 532)" '{_type:"CONTRIBUTION",
    versions:[{_type:"ORIGINAL_VERSION",preceding_version_uid:{_type:"OBJECT_VERSION_ID",value:$p},
      lifecycle_state:$c,commit_audit:{_type:"AUDIT_DETAILS",change_type:$m,committer:{_type:"PARTY_SELF"}},
      data:{_type:"EHR_ACCESS",archetype_node_id:"openEHR-EHR-EHR_ACCESS.access_directives.v1",
        archetype_details:{_type:"ARCHETYPED",rm_version:"1.2.0",
          archetype_id:{_type:"ARCHETYPE_ID",value:"openEHR-EHR-EHR_ACCESS.access_directives.v1"}},
        name:{_type:"DV_TEXT",value:"EHR Access"},settings:$s}}],
    audit:{_type:"AUDIT_DETAILS",change_type:$m,committer:{_type:"PARTY_SELF"}}}')"
  [[ "$H_CODE" = 201 || "$H_CODE" = 200 || "$H_CODE" = 204 ]]
}

# ── Assertions ────────────────────────────────────────────────────────────────

# The access-log expectations, one per scenario request:
# `<request id>|<result>|<reason or ->|<emergency true/false>|<what>`.
H_EXPECT=()

# Notes the access-log entry the last call must have written.
h_expect() {
  local result="$1" reason="${2:--}" emergency="${3:-false}" what="$4"
  [[ -n "$H_RID" ]] && H_EXPECT+=("$H_RID|$result|$reason|$emergency|$what")
}

# A read of the composition `c` of the EHR `ehr` through instance `n` as `who`
# declaring `purpose`, asserting the status `code` and, for a 403, the
# refusal `refusal`; the access-log entry it must write is noted.
h_read() {
  local n="$1" who="$2" ehr="$3" c="$4" purpose="$5" code="$6" refusal="${7:-}" what="$8"
  # An access declaring an emergency purpose carries the mark, granted or
  # refused (EHDS Art. 11(5)).
  local emergency=false
  [[ "$purpose" = ETREAT ]] && emergency=true
  h_call "$n" "$who" GET "/ehr/$ehr/composition/$(hv "$c")" "" "$purpose"
  assert_eq "$code" "$H_CODE" "$what (instance $n): ${H_BODY:0:300}"
  case "$code" in
    200) h_expect granted - "$emergency" "$what" ;;
    403) assert_eq "$refusal" "$(h_refusal)" "$what: the refusal code"
      h_expect refused "$refusal" "$emergency" "$what" ;;
    404) assert_not_contains "$(tr '[:upper:]' '[:lower:]' <<<"$H_BODY")" "restrict" \
        "$what: a 404 says nothing of a restriction"
      assert_not_contains "$H_BODY" "refused-" "$what: a 404 names no refusal"
      h_expect refused "$refusal" "$emergency" "$what" ;;
    *) ;;
  esac
}

# The composition ids an AQL query over the EHR `ehr` returns to `who`
# through instance `n`, sorted, one per line; `extra` is appended to the
# statement (LIMIT and OFFSET).
h_rows() {
  local n="$1" who="$2" ehr="$3" purpose="$4" extra="${5:-}"
  h_call "$n" "$who" POST "/query/aql?ehr_id=$ehr" "$(jq -nc --arg e "$ehr" --arg x "$extra" \
    '{q:("SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c WHERE e/ehr_id/value = $ehr ORDER BY c/uid/value" + $x),
      query_parameters:{ehr:$e}}')" "$purpose"
  jq -r '.rows[]?[0] | split("::")[0]' <<<"$H_BODY" 2>/dev/null
}

# The COUNT the same query answers.
h_count() {
  local n="$1" who="$2" ehr="$3" purpose="$4"
  h_call "$n" "$who" POST "/query/aql?ehr_id=$ehr" "$(jq -nc --arg e "$ehr" \
    '{q:"SELECT COUNT(c/uid/value) FROM EHR e CONTAINS COMPOSITION c WHERE e/ehr_id/value = $ehr",
      query_parameters:{ehr:$e}}')" "$purpose"
  jq -r '.rows[0][0] // empty' <<<"$H_BODY" 2>/dev/null
}

# Asserts that `who` through instance `n` queries exactly the compositions
# named (by setup name) in `expected`, that COUNT agrees, and that pages of
# one row walk the same set.
h_query_agrees() {
  local n="$1" who="$2" ehr="$3" purpose="$4" expected="$5" what="$6"
  local want got count paged i row name
  want="$(for name in $expected; do hv "$name"; printf '\n'; done | sed '/^$/d' | sort)"
  got="$(h_rows "$n" "$who" "$ehr" "$purpose")"
  assert_eq "$want" "$got" "$what: the rows of $who"
  count="$(h_count "$n" "$who" "$ehr" "$purpose")"
  assert_eq "$(sed '/^$/d' <<<"$want" | wc -l | tr -d ' ')" "${count:-none}" "$what: the COUNT of $who"
  paged=""
  for i in $(seq 0 9); do
    row="$(h_rows "$n" "$who" "$ehr" "$purpose" " LIMIT 1 OFFSET $i")"
    [[ -z "$row" ]] && break
    paged+="$row"$'\n'
  done
  assert_eq "$got" "$(sed '/^$/d' <<<"$paged")" "$what: pages of one row of $who"
}

# Asserts each noted access-log expectation: exactly one entry for its
# request, with the result, the refusal and the emergency mark noted.
h_check_log() {
  local row rid result reason emergency what entries
  for row in "${H_EXPECT[@]}"; do
    IFS='|' read -r rid result reason emergency what <<<"$row"
    h_call 1 kc:hans GET "$HX/admin/access-log?request_id=$rid"
    entries="$(jq -c '[.entries[]? | {result: .outcome.result.defining_code.code_string,
      reason: (.outcome.reason.defining_code.code_string // "-"), emergency: (.is_emergency // false)}]' \
      <<<"$H_BODY" 2>/dev/null)"
    assert_eq "[{\"result\":\"$result\",\"reason\":\"$reason\",\"emergency\":$emergency}]" "$entries" \
      "$what: the access-log entries of request $rid"
  done
}

# ── The family ────────────────────────────────────────────────────────────────

probes_hospital() {
  bold "A hospital scenario on two instances (#5610)"
  if ! command -v gpg >/dev/null 2>&1 || ! command -v jq >/dev/null 2>&1; then
    uncovered "the hospital scenario (#5610)" "gpg or jq is not available on this host"
    return 0
  fi
  local dir
  if ! dir="$(pgp_material hospital)" || [[ -z "$dir" ]]; then
    uncovered "the hospital scenario (#5610)" "this host's gpg would not generate a probe key"
    return 0
  fi
  HOSPITAL_KEY_OVERLAY="$(replica_key_overlay "$dir" "$PROBE_TMP/hospital-key.yml" ferroehr ferroehr-2)"
  # The hospital keeps a local patient index: the EHR_STATUS names the
  # patient's PERSON, whose care relationships the evaluator reads.
  export FERROEHR_EHR_INDEX=local
  hospital_files

  probe "P-HOSP-BOOT" "$PROBE_WORKING" "deployment" "#5610" \
    "two instances with Keycloak as the persons issuer and the machines issuer boot"
  if ! hospital_up; then
    probe_fail "two serving instances and a realm" "$(replica_state "${HOSPITAL_FILES[@]}")"
    probe_done
    hospital_down
    unset FERROEHR_EHR_INDEX
    return 0
  fi
  probe_done

  hospital_discovery
  probe "P-HOSP-SETUP" "$PROBE_WORKING" "server" "#5610" \
    "SCIM, the demographic API and ITS-REST set the people, the parties and the record up"
  hospital_setup
  probe_done

  hospital_portal_boot || { hospital_down; unset FERROEHR_EHR_INDEX; return 0; }
  hospital_policy
  hospital_directives
  hospital_rbac
  hospital_abac
  hospital_systems
  hospital_queries
  hospital_care_and_consent
  hospital_scim_lifecycle
  hospital_portal
  hospital_log
  hospital_pixm

  hospital_down
  unset FERROEHR_EHR_INDEX
}

# FerroHEALTH-backlog #34: the discovery document describes the persons
# issuer, never the machines issuer beside it.
hospital_discovery() {
  probe "P-HOSP-DISCOVERY" "$PROBE_WORKING" "server" "#5425" \
    "/.well-known/smart-configuration describes the Keycloak persons issuer, never the machines one"
  local n doc
  for n in 1 2; do
    doc="$(curl -s "$(h_base "$n")/ferroehr/rest/.well-known/smart-configuration")"
    assert_eq "$H_REALM_ISSUER" "$(jq -r '.issuer // empty' <<<"$doc")" "instance $n: the issuer"
    assert_contains "$(jq -r '.token_endpoint // empty' <<<"$doc")" "$H_REALM_ISSUER/" \
      "instance $n: the token endpoint is the realm's"
    assert_not_contains "$doc" "8180" "instance $n: nothing of the machines issuer"
  done
  probe_done
}

hospital_rbac() {
  local eva
  eva="$(hv ehr_eva)"
  # Tracked by #5621: no controller policy says which templates a role may
  # write, and #5624: template limits bind system callers only.
  probe "P-HOSP-RBAC-NURSE" "$PROBE_WORKING" "server" "#5621 #5624" \
    "a nurse reads the vital signs and cannot commit the diagnosis template her role may not write"
  h_read 2 kc:bram "$eva" c_vitals TREAT 200 "" "Nurse de Vries reads the vital signs"
  h_call 1 kc:bram POST "/ehr/$eva/composition" "$(h_example "$H_T_DIAGNOSIS")" TREAT
  assert_eq 403 "$H_CODE" "a NURSE commits the diagnosis template (#5621): ${H_BODY:0:300}"
  # A commit that was stored is named, so the probes after this one judge
  # the record as it stands.
  [[ "$H_CODE" = 201 ]] && hv_set c_nurse_dx "$(h_vo)"
  probe_done
}

hospital_abac() {
  local eva
  eva="$(hv ehr_eva)"
  probe "P-HOSP-ABAC-CARE" "$PROBE_WORKING" "server" "#5610" \
    "Dr. Jansen reads the cardiology data she treats Eva for, with a purpose, on both instances"
  h_read 1 kc:anna "$eva" c_encounter TREAT 200 "" "Dr. Jansen reads the encounter"
  h_read 2 kc:anna "$eva" c_vitals TREAT 200 "" "Dr. Jansen reads the vital signs"
  h_read 1 kc:anna "$eva" c_encounter "" 403 refused-purpose-missing "Dr. Jansen without a purpose"
  h_read 2 kc:anna "$eva" c_encounter ETREAT 403 refused-purpose-not-permitted \
    "Dr. Jansen with a purpose she may not declare"
  probe_done

  probe "P-HOSP-ABAC-CLEARANCE" "$PROBE_WORKING" "server" "#5610" \
    "Nurse de Vries reads the vital signs but not the HIV result she is not cleared for"
  h_read 1 kc:bram "$eva" c_vitals TREAT 200 "" "Nurse de Vries reads the vital signs"
  h_read 2 kc:bram "$eva" c_hiv TREAT 403 refused-sensitivity "Nurse de Vries reads the HIV result"
  h_read 1 kc:anna "$eva" c_hiv TREAT 200 "" "Dr. Jansen, cleared, reads the HIV result"
  probe_done

  probe "P-HOSP-ABAC-RESTRICTION" "$PROBE_WORKING" "server" "#5633 #5636" \
    "Gina is refused everything of Eva's with a 404 that says nothing of the restriction"
  h_read 1 kc:gina "$eva" c_vitals TREAT 404 refused-restricted "Gina reads the vital signs"
  h_read 2 kc:gina "$eva" c_encounter TREAT 404 refused-restricted "Gina reads the encounter"
  h_call 1 kc:gina GET "/ehr/$eva" "" TREAT
  assert_eq 404 "$H_CODE" "Gina reads Eva's EHR"
  h_expect refused refused-restricted false "Gina reads Eva's EHR"
  h_call 2 kc:gina GET "$HX/ehr/$eva/ehr_access" "" TREAT
  assert_eq 404 "$H_CODE" "Gina reads Eva's EHR_ACCESS"
  assert_eq "" "$(h_rows 1 kc:gina "$eva" TREAT)" "Gina's query of Eva's EHR returns nothing"
  probe_done

  probe "P-HOSP-ABAC-MENTAL" "$PROBE_WORKING" "server" "#5633 #5636" \
    "the mental-health note is refused to Dr. Jansen and readable by Dr. Smit"
  h_read 2 kc:anna "$eva" c_mental TREAT 403 refused-consent-denied "Dr. Jansen reads the note"
  h_read 1 kc:carla "$eva" c_mental TREAT 200 "" "Dr. Smit reads the note"
  probe_done

  probe "P-HOSP-ABAC-BREAKGLASS" "$PROBE_WORKING" "server" "#5627" \
    "Dr. Bakker reads the restricted HIV result only with the emergency role and purpose, marked emergency"
  h_read 1 kc:dirk "$eva" c_hiv TREAT 403 refused-sensitivity "Dr. Bakker without an emergency"
  h_read 2 kc:dirk "$eva" c_hiv ETREAT 200 "" "Dr. Bakker declares an emergency"
  h_read 1 kc:bram "$eva" c_hiv ETREAT 403 refused-purpose-not-permitted \
    "a nurse declaring an emergency without the role"
  probe_done
}

hospital_systems() {
  local eva
  eva="$(hv ehr_eva)"
  probe "P-HOSP-LAB-COMMIT" "$PROBE_WORKING" "server" "#5610" \
    "the lab system commits a lab result into Eva's record under its commit_composition grant"
  # A relayed person act names the professional by an identifier (EHDS
  # Art. 13(4)): the technician who validated the result.
  # The result names the lab as its facility, which the lab's grant covers.
  h_call 2 sys:lab-system POST "/ehr/$eva/composition" "$(h_example "$H_T_LAB" | jq -c '.composer =
    {_type:"PARTY_IDENTIFIED",name:"Lab technician",identifiers:[{_type:"DV_IDENTIFIER",
      issuer:"urn:oid:2.999.7.9",id:"lab-technician-1",type:"employee"}]}
    | if .context then .context.health_care_facility = {_type:"PARTY_IDENTIFIED",name:"Laboratory",
        identifiers:[{_type:"DV_IDENTIFIER",issuer:"urn:oid:2.999.7.9",id:"lab-1",type:"facility"}]} else . end')" TREAT
  hv_set c_lab "$(h_vo)"
  if [[ "$H_CODE" = 201 ]]; then
    h_expect granted - false "the lab system's commit"
  else
    h_fail_call "201 for the lab system's result"
  fi
  probe_done

  probe "P-HOSP-HIE-CONSENT" "$PROBE_WORKING" "server" "#5633 #5636" \
    "the regional HIE client reads under Eva's consent; a client outside the region is refused"
  h_read 1 sys:hie-member "$eva" c_vitals TREAT 200 "" "the member provider's client"
  h_read 2 sys:hie-outside "$eva" c_vitals TREAT 403 refused-consent-required "the outside client"
  probe_done
}

hospital_queries() {
  local eva lab=""
  eva="$(hv ehr_eva)"
  [[ -n "$(hv c_lab)" ]] && lab=c_lab
  probe "P-HOSP-AQL" "$PROBE_WORKING" "server" "#5633 #5636" \
    "AQL rows, COUNT and paging agree with the reads for every caller, on both instances"
  h_query_agrees 1 kc:anna "$eva" TREAT "c_encounter c_vitals c_hiv c_diagnosis c_nurse_dx $lab" "Dr. Jansen"
  h_query_agrees 2 kc:bram "$eva" TREAT "c_encounter c_vitals c_diagnosis c_nurse_dx $lab" "Nurse de Vries"
  h_query_agrees 1 kc:carla "$eva" TREAT "c_encounter c_vitals c_mental c_hiv c_diagnosis c_nurse_dx $lab" "Dr. Smit"
  h_query_agrees 2 kc:gina "$eva" TREAT "" "Gina"
  h_query_agrees 1 kc:dirk "$eva" ETREAT "c_encounter c_vitals c_mental c_hiv c_diagnosis c_nurse_dx $lab" \
    "Dr. Bakker in an emergency"
  h_query_agrees 2 kc:dirk "$eva" TREAT "" "Dr. Bakker without one"
  h_query_agrees 1 sys:hie-member "$eva" TREAT "c_encounter c_vitals c_diagnosis c_nurse_dx $lab" "the member's client"
  h_query_agrees 2 sys:hie-outside "$eva" TREAT "" "the outside client"
  probe_done
}

hospital_care_and_consent() {
  local eva
  eva="$(hv ehr_eva)"
  probe "P-HOSP-OPT-OUT" "$PROBE_WORKING" "server" "#5633 #5636" \
    "the opt-out hides Eva from the access services but not from her clinicians; its withdrawal restores access"
  h_settings 1 "pt:eva:$eva" "$eva" "$(j_eva_settings "$(h_day 0)" "$(h_day 365)" opt-out)" \
    || h_fail_call "Eva records her opt-out"
  h_read 2 sys:access-service "$eva" c_vitals TREAT 404 refused-opt-out "the access service after the opt-out"
  h_read 2 kc:anna "$eva" c_vitals TREAT 200 "" "Dr. Jansen after the opt-out"
  h_read 1 sys:hie-member "$eva" c_vitals TREAT 200 "" "the HIE client, no access service, after the opt-out"
  h_settings 2 "pt:eva:$eva" "$eva" "$(j_eva_settings "$(h_day 0)" "$(h_day 365)")" \
    || h_fail_call "Eva withdraws her opt-out"
  h_read 1 sys:access-service "$eva" c_vitals TREAT 200 "" "the access service after the withdrawal"
  probe_done

  probe "P-HOSP-CONSENT-ENDED" "$PROBE_WORKING" "server" "#5633 #5636" \
    "after the regional consent's validity ends the HIE client is refused"
  h_settings 1 "pt:eva:$eva" "$eva" "$(j_eva_settings "$(h_day -400)" "$(h_day -35)")" \
    || h_fail_call "the consent with its validity ended"
  h_read 2 sys:hie-member "$eva" c_vitals TREAT 403 refused-consent-required "the HIE client after the consent ended"
  h_settings 2 "pt:eva:$eva" "$eva" "$(j_eva_settings "$(h_day 0)" "$(h_day 365)")" \
    || h_fail_call "the consent restored"
  h_read 1 sys:hie-member "$eva" c_vitals TREAT 200 "" "the HIE client under the renewed consent"
  probe_done

  probe "P-HOSP-GRACE" "$PROBE_WORKING" "server" "#5610" \
    "a discharge decides on the other instance: within the grace Dr. Jansen reads, after it she is refused"
  local team rels
  rels="$(jq -c --arg c "$(hv cardiology)" --argjson v "$(j_dates "$(h_day -10)" "$(h_day 0)")" \
    'map(if .target.id.value == $c then .time_validity = $v else . end)' <<<"$H_EVA_CARE")"
  h_rerelate 1 admin person "$(hv person_eva)" "$rels" || h_fail_call "Eva's discharge today"
  h_read 2 kc:anna "$eva" c_encounter TREAT 200 "" "Dr. Jansen within the grace"
  rels="$(jq -c --arg c "$(hv cardiology)" --argjson v "$(j_dates "$(h_day -10)" "$(h_day -3)")" \
    'map(if .target.id.value == $c then .time_validity = $v else . end)' <<<"$H_EVA_CARE")"
  h_rerelate 2 admin person "$(hv person_eva)" "$rels" || h_fail_call "Eva's discharge three days ago"
  h_read 1 kc:anna "$eva" c_encounter TREAT 403 refused-no-care-relationship "Dr. Jansen after the grace"
  h_rerelate 1 admin person "$(hv person_eva)" "$H_EVA_CARE" || h_fail_call "Eva readmitted"
  h_read 2 kc:anna "$eva" c_encounter TREAT 200 "" "Dr. Jansen after the readmission"
  probe_done

  probe "P-HOSP-ROLE-REMOVED" "$PROBE_WORKING" "server" "#5627" \
    "a clearance removed on one instance stops working on the next request on the other"
  h_call 1 admin GET "/demographic/role/$(hv role_anna)"
  local version
  version="$(jq -r '.uid.value // empty' <<<"$H_BODY")"
  h_call 1 admin DELETE "/demographic/role/$version"
  [[ "$H_CODE" = 204 ]] || h_fail_call "204 for the removal of Dr. Jansen's clearances"
  h_read 2 kc:anna "$eva" c_hiv TREAT 403 refused-sensitivity "Dr. Jansen after her clearance went"
  h_facts_role anna "N R" TREAT || h_fail_call "Dr. Jansen's clearances again"
  h_read 1 kc:anna "$eva" c_hiv TREAT 200 "" "Dr. Jansen cleared again"
  probe_done
}

# A SCIM PATCH of the User `id` through instance `n` with `operations`.
scim_patch() {
  h_call "$1" scim PATCH "/scim/v2/Users/$2" "$(jq -nc --argjson o "$3" \
    '{schemas:["urn:ietf:params:scim:api:messages:2.0:PatchOp"],Operations:$o}')" "" application/scim+json
}

hospital_scim_lifecycle() {
  local eva bram
  eva="$(hv ehr_eva)"
  bram="$(hv person_bram)"
  probe "P-HOSP-SCIM-LIFECYCLE" "$PROBE_WORKING" "server" "#5628" \
    "SCIM update, deactivation, reactivation and deletion decide the next request on the other instance"
  scim_patch 1 "$bram" '[{"op":"replace","path":"name.formatted","value":"Bram de Vries RN"}]'
  assert_eq 200 "$H_CODE" "SCIM updates Nurse de Vries: ${H_BODY:0:200}"
  h_read 2 kc:bram "$eva" c_vitals TREAT 200 "" "Nurse de Vries after the update"
  scim_patch 2 "$bram" '[{"op":"replace","path":"active","value":false}]'
  assert_eq 200 "$H_CODE" "SCIM deactivates Nurse de Vries"
  h_call 1 kc:bram GET "/ehr/$eva/composition/$(hv c_vitals)" "" TREAT
  assert_eq 403 "$H_CODE" "a deactivated nurse reads the vital signs: ${H_BODY:0:200}"
  h_call 2 kc:bram GET "/ehr/$(hv ehr_piet)" "" TREAT
  assert_eq 403 "$H_CODE" "a deactivated nurse is refused everywhere"
  scim_patch 1 "$bram" '[{"op":"replace","path":"active","value":true}]'
  assert_eq 200 "$H_CODE" "SCIM reactivates Nurse de Vries"
  h_read 2 kc:bram "$eva" c_vitals TREAT 200 "" "Nurse de Vries after the reactivation"
  h_call 1 scim DELETE "/scim/v2/Users/$bram"
  assert_eq 204 "$H_CODE" "SCIM deletes Nurse de Vries"
  h_call 2 kc:bram GET "/ehr/$eva/composition/$(hv c_vitals)" "" TREAT
  assert_eq 403 "$H_CODE" "a deleted nurse reads the vital signs: ${H_BODY:0:200}"
  probe_done

  # Tracked by #5617: a SCIM Group is no care team, so its membership
  # decides nothing.
  probe "P-HOSP-SCIM-GROUPS" "$PROBE_WORKING" "server" "#5617" \
    "a SCIM Group membership change decides access on the other instance"
  h_call 1 scim POST /scim/v2/Groups "$(jq -nc --arg m "$(hv person_dirk)" \
    '{schemas:["urn:ietf:params:scim:schemas:core:2.0:Group"],displayName:"Emergency department",
      externalId:"emergency-department",members:[{value:$m}]}')" "" application/scim+json
  assert_eq 201 "$H_CODE" "SCIM creates the emergency department with Dr. Bakker: ${H_BODY:0:300}"
  local group
  group="$(jq -r '.id // empty' <<<"$H_BODY")"
  if [[ -n "$group" ]]; then
    H_EVA_CARE="$(jq -c --argjson r "$(j_care under_care_of PERSON "$(hv person_eva)" GROUP "$group" \
      "$(j_dates "$(h_day -1)")")" '. + [$r]' <<<"$H_EVA_CARE")"
    h_rerelate 2 admin person "$(hv person_eva)" "$H_EVA_CARE" || h_fail_call "Eva admitted to the emergency department"
  fi
  h_read 1 kc:dirk "$eva" c_vitals TREAT 200 "" "Dr. Bakker, a member through SCIM, reads the vital signs"
  h_call 2 scim PATCH "/scim/v2/Groups/$group" "$(jq -nc --arg m "$(hv person_dirk)" \
    '{schemas:["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
      Operations:[{op:"remove",path:("members[value eq \"" + $m + "\"]")}]}')" "" application/scim+json
  # NOTE: RFC 7644 §3.5.2 lets a PATCH answer 200 with the resource or 204 without it.
  [[ "$H_CODE" = 204 ]] && H_CODE=200
  assert_eq 200 "$H_CODE" "SCIM removes Dr. Bakker from the Group on the other instance: \
${H_BODY:0:300}"
  h_read 1 kc:dirk "$eva" c_vitals TREAT 403 refused-no-care-relationship \
    "Dr. Bakker after SCIM removed him from the team"
  probe_done

  # Tracked by #5625: a SCIM User carries no roles.
  probe "P-HOSP-SCIM-ROLE" "$PROBE_WORKING" "server" "#5625" \
    "a role removed through SCIM stops working on the next request on the other instance"
  scim_patch 1 "$(hv person_anna)" '[{"op":"remove","path":"roles[value eq \"CLINICIAN\"]"}]'
  assert_eq 200 "$H_CODE" "SCIM removes Dr. Jansen's clinician role: ${H_BODY:0:300}"
  h_call 2 kc:anna GET "/ehr/$eva/composition/$(hv c_encounter)" "" TREAT
  assert_eq 403 "$H_CODE" "Dr. Jansen reads the encounter after her clinician role went"
  probe_done
}

# Restarts both instances with the patient portal's `patients` issuer beside
# the realm, so Eva and Frank sign in; the discovery document still names
# the realm.
hospital_portal_boot() {
  probe "P-HOSP-PORTAL-BOOT" "$PROBE_WORKING" "deployment" "#5610" \
    "both instances restart with the patient portal's patients issuer beside the realm"
  hospital_files patients
  if ! hospital_restart; then
    probe_fail "two serving instances with the portal issuer" "$(replica_state "${HOSPITAL_FILES[@]}")"
    probe_done
    return 1
  fi
  probe_done
  hospital_discovery
}

# The controller's administrator writes the ACCESS_POLICY through the admin
# API; Hans, the privacy officer, reads it.
hospital_policy() {
  probe "P-HOSP-POLICY" "$PROBE_WORKING" "server" "#5610" \
    "the controller's administrator writes the ACCESS_POLICY and the privacy officer reads it"
  H_IF_NONE_MATCH='*' h_call 2 kc:ferroehr PUT "$HX/admin/access_policy" "$(j_policy)"
  [[ "$H_CODE" = 201 ]] || h_fail_call "201 for the controller's ACCESS_POLICY"
  h_call 1 kc:hans GET "$HX/admin/access_policy"
  assert_eq 200 "$H_CODE" "Hans reads the policy on the other instance: ${H_BODY:0:200}"
  assert_eq true "$(jq -r '.care_relationship_required // .document.care_relationship_required // empty' \
    <<<"$H_BODY")" "the policy read back requires a care relationship"
  probe_done
}

# Eva, her own gate-keeper, records her directives through a CONTRIBUTION
# with her portal login.
hospital_directives() {
  probe "P-HOSP-DIRECTIVES" "$PROBE_WORKING" "server" "#5633 #5636" \
    "Eva records her directives in her EHR_ACCESS through a CONTRIBUTION with her portal login"
  local eva
  eva="$(hv ehr_eva)"
  h_settings 1 "pt:eva:$eva" "$eva" "$(j_eva_settings "$(h_day 0)" "$(h_day 365)")" \
    || h_fail_call "Eva commits her EHR_ACCESS"
  h_call 2 "pt:eva:$eva" GET "$HX/ehr/$eva/ehr_access"
  assert_eq 4 "$(jq '.settings.directives | length' <<<"$H_BODY" 2>/dev/null)" \
    "Eva reads her four directives back on the other instance"
  probe_done
}

hospital_portal() {
  local eva gina lab=""
  eva="$(hv ehr_eva)"
  gina="$(hv ehr_gina)"
  [[ -n "$(hv c_lab)" ]] && lab=c_lab
  probe "P-HOSP-SUBJECT" "$PROBE_WORKING" "server" "#5633" \
    "Eva reads her whole record and nobody else's"
  local c
  for c in c_encounter c_vitals c_mental c_hiv c_diagnosis; do
    h_read 1 "pt:eva:$eva" "$eva" "$c" "" 200 "" "Eva reads her $c"
  done
  h_query_agrees 2 "pt:eva:$eva" "$eva" "" \
    "c_encounter c_vitals c_mental c_hiv c_diagnosis c_nurse_dx $lab" "Eva"
  h_read 2 "pt:eva:$eva" "$gina" c_gina "" 403 refused-not-subject "Eva reads Gina's record"
  probe_done

  probe "P-HOSP-REPRESENTATIVE" "$PROBE_WORKING" "server" "#5633 #5613" \
    "Frank reads Eva's record as her representative, held to her directives, and not Gina's"
  h_read 1 "pt:frank:$eva" "$eva" c_vitals "" 200 "" "Frank reads Eva's vital signs"
  h_read 2 "pt:frank:$eva" "$eva" c_mental "" 403 refused-consent-denied "Frank reads Eva's mental-health note"
  h_read 1 "pt:frank:$eva" "$gina" c_gina "" 403 refused-not-subject "Frank reads Gina's record"
  probe_done
}

hospital_log() {
  local eva
  eva="$(hv ehr_eva)"
  # The drain writes each entry after its response.
  sleep 3
  probe "P-HOSP-LOG-ENTRIES" "$PROBE_WORKING" "server" "#5626 #5627 #5633 #5636" \
    "every access and refusal of the scenario wrote exactly one access-log entry with its outcome"
  h_check_log
  probe_done

  probe "P-HOSP-LOG-VIEWS" "$PROBE_WORKING" "server" "#5633" \
    "Eva reads her own log without the withheld roles; Hans reads every entry in full; a clinician reads none"
  h_call 2 "pt:eva:$eva" GET "$HX/ehr/$eva/access_log?fetch=1000"
  assert_eq 200 "$H_CODE" "Eva reads her access log: ${H_BODY:0:200}"
  local mine
  mine="$H_BODY"
  assert_contains "$(jq -r '[.entries[]?.accessor.kind.value] | unique | join(",")' <<<"$mine")" "person" \
    "Eva's log names the professionals who read her record"
  assert_eq 0 "$(jq '[.entries[]? | select(.accessor.kind.value == "person") | .accessor.roles // empty] | length' \
    <<<"$mine")" "Eva's view withholds the roles of a person (EHDS Art. 9(3))"
  assert_eq 0 "$(jq '[.entries[]? | select(.link != null or .credential != null)] | length' <<<"$mine")" \
    "Eva's view carries no link and no credential"
  h_call 1 kc:hans GET "$HX/admin/access-log?ehr_id=$eva&fetch=1000"
  assert_eq 200 "$H_CODE" "Hans reads the log"
  assert_contains "$(jq -c '[.entries[]? | select(.accessor.kind.value == "person") | .accessor.roles // empty][0]' \
    <<<"$H_BODY")" "CLINICIAN" "Hans's full view carries the roles"
  h_call 2 "pt:eva:$eva" GET "$HX/ehr/$(hv ehr_gina)/access_log"
  assert_eq 404 "$H_CODE" "Eva reads Gina's log"
  h_call 1 kc:anna GET "$HX/admin/access-log?ehr_id=$eva" "" TREAT
  assert_eq 403 "$H_CODE" "Dr. Jansen reads the review"
  probe_done

  probe "P-HOSP-LOG-CHAINS" "$PROBE_WORKING" "server" "#5610" \
    "the access log and the system chain both instances appended to verify"
  local n out
  for n in 1 2; do
    h_call "$n" kc:hans POST "$HX/admin/access-log/verify"
    assert_eq 200 "$H_CODE" "instance $n verifies the access log: ${H_BODY:0:300}"
  done
  hdc exec -T ferroehr /usr/local/bin/ferroehr audit-chain seal >/dev/null 2>&1
  out="$(hdc exec -T ferroehr-2 /usr/local/bin/ferroehr audit-chain verify 2>&1)"
  assert_contains "$out" "the system chain verifies" "$(replica_findings "$out")"
  probe_done
}

# ── The same care relationships under the default pixm subject index ─────────

# Feeds the patient `mrn` of the quickstart's source domain to FerroPIX, as
# the hospital's ADT system does (IHE ITI-104).
pix_feed() {
  local mrn="$1" token
  token="$(curl -s -H 'Host: localhost:8180' -d grant_type=client_credentials \
    -d client_id=quickstart-source -d client_secret=hospital -d scope=system/Patient.cud \
    "$H_ISSUER_HTTP/quickstart/token" | jq -r '.access_token // empty')"
  curl -s -o /dev/null -w '%{http_code}' -X PUT -H 'Host: localhost:8090' \
    -H "Authorization: Bearer $token" -H 'Content-Type: application/fhir+json' \
    "http://localhost:${FERROEHR_PIX_PORT}/fhir/Patient?identifier=urn:oid:2.999.1.1%7C$mrn" \
    --data-binary "{\"resourceType\":\"Patient\",\"identifier\":[{\"system\":\"urn:oid:2.999.1.1\",\"value\":\"$mrn\"}],\"name\":[{\"family\":\"Visser\",\"given\":[\"Eva\"]}]}"
}

# The care-relationship and representation assertions again on a stack with
# the quickstart's default subject index, where the EHR_STATUS carries the
# FerroPIX pseudonym. Tracked by #5634.
hospital_pixm() {
  unset FERROEHR_EHR_INDEX
  hospital_files patients
  probe "P-HOSP-PIXM-BOOT" "$PROBE_WORKING" "deployment" "#5610" \
    "the stack boots again with the default pixm subject index and the portal issuer"
  if ! hospital_up; then
    probe_fail "two serving instances on pixm" "$(replica_state "${HOSPITAL_FILES[@]}")"
    probe_done
    return 0
  fi
  probe_done

  probe "P-HOSP-PIXM-SETUP" "$PROBE_WORKING" "server" "#5610" \
    "Dr. Jansen, Cardiology, Eva under its care, Frank her representative and Eva's EHR through FerroPIX"
  kc_role CLINICIAN
  kc_user anna Anna Jansen CLINICIAN
  scim_user 1 anna "Dr. Anna Jansen" || h_fail_call "SCIM creates anna"
  h_party cardiology group "$(j_named_party GROUP Cardiology)" || h_fail_call "the Cardiology team"
  h_party hospital organisation "$(j_named_party ORGANISATION "Sint Example Ziekenhuis")" || h_fail_call "the controller"
  h_facts_role anna "N R" TREAT || h_fail_call "Dr. Jansen's clearances"
  h_call 1 scim GET "/scim/v2/Users?filter=$(jq -rn '"userName eq \"keycloak-ferroehr\""|@uri')"
  hv_set person_controller "$(jq -r '.Resources[0].id // empty' <<<"$H_BODY")"
  h_party role_controller_policy role "$(j_role "controller administrator" PERSON "$(hv person_controller)" \
    "$(j_capability "$(j_operation write_access_policy)")")" || h_fail_call "the controller's policy grant"
  h_relate 2 admin person "$(hv person_anna)" \
    "[$(j_care member_of PERSON "$(hv person_anna)" GROUP "$(hv cardiology)" "$(j_dates "$(h_day -30)")")]" \
    || h_fail_call "Dr. Jansen joins Cardiology"
  h_party person_eva person "$(j_named_party PERSON "Eva Visser")" || h_fail_call "Eva's PERSON"
  H_EVA_CARE="[$(j_care under_care_of PERSON "$(hv person_eva)" GROUP "$(hv cardiology)" "$(j_dates "$(h_day -10)")")]"
  h_rerelate 1 admin person "$(hv person_eva)" "$H_EVA_CARE" || h_fail_call "Eva's admission"
  h_call 1 admin GET "/demographic/person/$(hv person_anna)"
  h_party person_frank person "$(jq -c --arg i "$H_PATIENTS" 'del(.uid) | del(.relationships)
    | .name.value = "Frank Visser"
    | .details.items = [.details.items[0] | .value = {_type:"DV_IDENTIFIER",issuer:$i,id:"frank",type:"login"}]
    | .identities[0].details.items[0].value.value = "Frank Visser"' <<<"$H_BODY")" || h_fail_call "Frank's PERSON"
  h_rerelate 2 admin person "$(hv person_frank)" "[$(j_care represents PERSON "$(hv person_frank)" \
    PERSON "$(hv person_eva)" "$(j_dates "$(h_day -365)")")]" || h_fail_call "Frank represents Eva"
  local fed
  fed="$(pix_feed eva-0001)"
  [[ "$fed" = 200 || "$fed" = 201 ]] || probe_fail "200 or 201 for the ITI-104 feed of Eva" "$fed"
  # The hospital asks FerroPIX for Eva's pseudonym in the clinical domain
  # (IHE ITI-83), which her EHR_STATUS names, as deploy/hosted's seed does.
  local consumer pseudonym
  consumer="$(curl -s -H 'Host: localhost:8180' -d grant_type=client_credentials -d client_id=ferroehr \
    -d client_secret=quickstart -d scope=system/Patient.rs "$H_ISSUER_HTTP/quickstart/token" \
    | jq -r '.access_token // empty')"
  pseudonym="$(curl -s -H 'Host: localhost:8090' -H "Authorization: Bearer $consumer" \
    -H 'Accept: application/fhir+json' \
    "http://localhost:${FERROEHR_PIX_PORT}/fhir/Patient/\$ihe-pix?sourceIdentifier=urn:oid:2.999.1.1%7Ceva-0001&targetSystem=urn:oid:2.999.7.1" \
    | jq -r '[.. | objects | select(.system? == "urn:oid:2.999.7.1") | .value] | first // empty')"
  [[ -n "$pseudonym" ]] || probe_fail "Eva's pseudonym from FerroPIX" "none"
  h_call 1 admin POST /ehr "$(jq -nc --arg p "$pseudonym" '{_type:"EHR_STATUS",
    archetype_node_id:"openEHR-EHR-EHR_STATUS.generic.v1",name:{_type:"DV_TEXT",value:"EHR status"},
    archetype_details:{_type:"ARCHETYPED",rm_version:"1.1.0",
      archetype_id:{_type:"ARCHETYPE_ID",value:"openEHR-EHR-EHR_STATUS.generic.v1"}},
    subject:{_type:"PARTY_SELF",external_ref:{_type:"PARTY_REF",namespace:"urn:oid:2.999.7.1",type:"PERSON",
      id:{_type:"HIER_OBJECT_ID",value:$p}}},
    is_queryable:true,is_modifiable:true}')"
  hv_set ehr_eva "$(jq -r '.ehr_id.value // empty' <<<"$H_BODY")"
  [[ "$H_CODE" = 201 ]] || h_fail_call "Eva's EHR, its subject resolved through FerroPIX"
  h_call 1 admin POST /definition/template/adl1.4 "@$H_OPTS/Vital Signs Encounter (Composition).opt" "" application/xml
  local eva
  eva="$(hv ehr_eva)"
  h_commit c_vitals 2 kc:anna "$eva" "$H_T_VITALS" || h_fail_call "the vital signs"
  H_IF_NONE_MATCH='*' h_call 1 kc:ferroehr PUT "$HX/admin/access_policy" "$(j_policy)"
  [[ "$H_CODE" = 201 ]] || h_fail_call "the controller's ACCESS_POLICY"
  probe_done

  probe "P-HOSP-PIXM-CARE" "$PROBE_WORKING" "server" "#5634" \
    "under pixm, Dr. Jansen reads the record of the patient she treats, on both instances"
  h_read 2 kc:anna "$eva" c_vitals TREAT 200 "" "Dr. Jansen under pixm (#5634)"
  h_read 1 kc:anna "$eva" c_vitals TREAT 200 "" "Dr. Jansen under pixm on the other instance (#5634)"
  probe_done

  probe "P-HOSP-PIXM-REPRESENTATIVE" "$PROBE_WORKING" "server" "#5634 #5633" \
    "under pixm, Frank reads Eva's record as her representative"
  h_read 1 "pt:frank:$eva" "$eva" c_vitals "" 200 "" "Frank under pixm (#5634, #5633)"
  probe_done
}
