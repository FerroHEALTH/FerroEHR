#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The hospital's PIX merge and steward revert (#5693).
#
# FerroEHR subscribes to FerroPIX with PMIR ITI-94 and applies its ITI-93
# messages. A data steward merges a duplicate record of a patient in FerroPIX
# and later reverts the merge; each time the second record's EHR follows on
# both instances: after the merge Dr. Jansen reads it under the survivor's
# care relationship, after the revert she is refused again. Everything is
# observed over HTTP.
#
# Sourced by scripts/deploy-probe.sh after hospital.sh, whose helpers it
# reuses; never run directly.

# FerroPIX on the hospital's stack, which every call reaches on loopback.
HP_PIX="http://localhost:${FERROEHR_PIX_PORT}"
# The source domain the quickstart's ADT system feeds.
HP_SOURCE="urn:oid:2.999.1.1"
# The clinical pseudonymisation domain.
HP_CLINICAL="urn:oid:2.999.7.1"

# The FerroPIX release of the PMIR leg, pinned by digest: the first to
# authenticate its ITI-93 sender (FerroPIX #655) and to state every Patient's
# `active` and links (FerroPIX #656).
HP_FERROPIX_IMAGE="ghcr.io/ferrohealth/ferropix:0.0.31@sha256:6681a8e3f923ecc07ceaa3cc7ede679e2c9fd471c25560dad12c82edaafbd6df"

# Writes once per run, under `dir`, a test CA and the certificate both
# instances serve their listener with (`localhost`, `127.0.0.1`): ITI-93 needs
# secure transport (PMIR 1.6.0 §2:3.93.5), so the feed reaches FerroEHR over
# TLS the CA anchors.
hospital_pix_tls() {
  local dir="$1"
  [[ -s "$dir/ferroehr.crt" ]] && return 0
  mkdir -p "$dir"
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 2 \
    -subj "/CN=FerroEHR hospital probe CA" -addext basicConstraints=critical,CA:true \
    -keyout "$dir/ca.key" -out "$dir/ca.crt" 2>/dev/null
  openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -subj "/CN=localhost" \
    -keyout "$dir/ferroehr.key" -out "$dir/ferroehr.csr" 2>/dev/null
  openssl x509 -req -in "$dir/ferroehr.csr" -CA "$dir/ca.crt" -CAkey "$dir/ca.key" \
    -CAcreateserial -days 2 -out "$dir/ferroehr.crt" \
    -extfile <(printf 'subjectAltName=DNS:localhost,IP:127.0.0.1\nextendedKeyUsage=serverAuth\n') 2>/dev/null
  # FerroPIX's production profile reads every secret from a file: its feed
  # client's secret at the quickstart issuer, which signs any.
  printf 'quickstart' > "$dir/feed-secret"
  # NOTE: the containers run as a non-root user; the key is a throwaway of this run.
  chmod 0755 "$dir"
  chmod 0644 "$dir/ca.crt" "$dir/ferroehr.crt" "$dir/ferroehr.key" "$dir/feed-secret"
}

# The overlay of the PMIR leg, written to `out` with the TLS material under
# `dir`: both instances serve TLS under an https public base URL, subscribe
# with the quickstart client `ferroehr` and trust the feed of the client
# `ferropix`; FerroPIX runs on its production profile with PMIR, its operator
# API and the consumer entry that presents its IUA token to FerroEHR over TLS
# the test CA anchors.
hospital_pix_overlay() {
  local out="$1" dir="$2"
  cat > "$out" <<YAML
x-pix-env: &pix-env
  FERROEHR__SERVER__PUBLIC_BASE_URL: https://localhost:8080
  FERROEHR__SERVER__TLS__CERTIFICATE_FILE: /run/pmir-tls/ferroehr.crt
  FERROEHR__SERVER__TLS__KEY_FILE: /run/pmir-tls/ferroehr.key
  FERROEHR__PIX__FEED_ISSUER: http://localhost:8180/quickstart
  FERROEHR__PIX__FEED_CLIENT_ID: ferropix
  FERROEHR__PIX__ENDPOINT__URL: http://localhost:8090/fhir
  FERROEHR__PIX__ENDPOINT__CREDENTIALS__OAUTH__TOKEN_ENDPOINT: http://localhost:8180/quickstart/token
  FERROEHR__PIX__ENDPOINT__CREDENTIALS__OAUTH__CLIENT_ID: ferroehr
  FERROEHR__PIX__ENDPOINT__CREDENTIALS__OAUTH__CLIENT_SECRET: quickstart
  FERROEHR__PIX__ENDPOINT__CREDENTIALS__OAUTH__SCOPE: system/Subscription.cruds
services:
  ferroehr:
    environment: *pix-env
    volumes:
      - $dir:/run/pmir-tls:ro
  ferroehr-2:
    environment: *pix-env
    volumes:
      - $dir:/run/pmir-tls:ro
  ferropix:
    image: $HP_FERROPIX_IMAGE
    volumes:
      - $dir:/run/pmir-tls:ro
configs:
  ferropix-config:
    content: |
      profile = "production"
      public_base_url = "http://localhost:8090"

      [db]
      url_file = "/etc/ferropix/db-url"
      data_key_file = "/var/lib/ferropix-key/data-key"

      [[auth.issuer]]
      issuer = "http://localhost:8180/quickstart"
      kind = "machines"
      audience = ["http://localhost:8090/fhir"]
      token_types = ["JWT"]
      required_claims = ["iss", "exp", "aud", "sub"]
      client_claims = ["client_id", "azp"]

      [[trust.issuer]]
      issuer = "http://localhost:8180/quickstart"
      clients = ["quickstart-source"]

      [[domain]]
      system = "urn:oid:2.999.1.1"
      name = "Quickstart hospital"
      kind = "local"
      source_client_id = "quickstart-source"

      [[pseudonym_domain]]
      namespace = "urn:oid:2.999.7.1"
      name = "FerroEHR clinical domain"
      consumer_client_id = "ferroehr"

      [operator]
      enabled = true

      [pmir]
      enabled = true

      [[pmir.feed.consumers]]
      url = "https://localhost:8080"

      [pmir.feed.consumers.tls]
      trust_roots_file = "/run/pmir-tls/ca.crt"

      [pmir.feed.consumers.credentials.oauth]
      token_endpoint = "http://localhost:8180/quickstart/token"
      client_id = "ferropix"
      client_secret_file = "/run/pmir-tls/feed-secret"
      scope = "ferroehr/pmir"

      [usage_report]
      deployment = "compose"
      state_dir = "/var/lib/ferropix"
YAML
  printf '%s' "$out"
}

# Restarts both instances and FerroPIX on the PMIR leg's overlay, over the
# stack hospital_up brought up: the provisioning one-shots ran over plain http
# before, and from here every call reaches the instances over TLS.
# shellcheck disable=SC2034 # H1 and H2 are read by h_base in hospital.sh
hospital_pix_up() {
  local dir="$PROBE_TMP/pmir-tls"
  hospital_pix_tls "$dir" || return 1
  HOSPITAL_FILES+=(-f "$(hospital_pix_overlay "$PROBE_TMP/hospital-pix.yml" "$dir")")
  export CURL_CA_BUNDLE="$dir/ca.crt"
  H1="https://localhost:${FERROEHR_PORT}"
  H2="https://localhost:${HOSPITAL_REPLICA_PORT}"
  if ! hdc up -d --wait --force-recreate --no-deps ferropix >/dev/null 2>&1; then
    printf '      log of ferropix:\n'
    hdc logs --no-color --tail 40 ferropix 2>&1 | sed 's/^/        /'
    return 1
  fi
  hospital_restart
}

# Puts the plain-http addresses back once the PMIR leg is done.
# shellcheck disable=SC2034 # H1 and H2 are read by h_base in hospital.sh
hospital_pix_down() {
  unset CURL_CA_BUNDLE
  H1="$CDR"
  H2="http://localhost:${HOSPITAL_REPLICA_PORT}"
}

# A client credentials token of the quickstart issuer for `client` with
# `scope`.
hp_token() {
  curl -s -H 'Host: localhost:8180' -d grant_type=client_credentials -d "client_id=$1" \
    -d client_secret=quickstart -d "scope=$2" "$H_ISSUER_HTTP/quickstart/token" \
    | jq -r '.access_token // empty'
}

# Feeds the source record `mrn` to FerroPIX with the name `family`, `given`,
# as the hospital's ADT system does (IHE ITI-104).
hp_feed() {
  local mrn="$1" family="$2" given="$3"
  curl -s -o /dev/null -w '%{http_code}' -X PUT -H 'Host: localhost:8090' \
    -H "Authorization: Bearer $(hp_token quickstart-source system/Patient.cud)" \
    -H 'Content-Type: application/fhir+json' \
    "$HP_PIX/fhir/Patient?identifier=$HP_SOURCE%7C$mrn" \
    --data-binary "$(jq -nc --arg s "$HP_SOURCE" --arg m "$mrn" --arg f "$family" --arg g "$given" \
      '{resourceType:"Patient",identifier:[{system:$s,value:$m}],name:[{family:$f,given:[$g]}]}')"
}

# FerroPIX's person id of the source record `mrn`, read over PDQm (ITI-78).
hp_person() {
  curl -s -H 'Host: localhost:8090' -H "Authorization: Bearer $(hp_token ferroehr system/Patient.rs)" \
    -H 'Accept: application/fhir+json' \
    "$HP_PIX/fhir/Patient?identifier=$HP_SOURCE%7C$1" | jq -r '.entry[0].resource.id // empty'
}

# The pseudonym FerroPIX holds for `mrn` in the clinical domain (ITI-83).
hp_pseudonym() {
  curl -s -H 'Host: localhost:8090' -H "Authorization: Bearer $(hp_token ferroehr system/Patient.rs)" \
    -H 'Accept: application/fhir+json' \
    "$HP_PIX/fhir/Patient/\$ihe-pix?sourceIdentifier=$HP_SOURCE%7C$1&targetSystem=$HP_CLINICAL" \
    | jq -r --arg d "$HP_CLINICAL" '[.. | objects | select(.system? == $d) | .value] | first // empty'
}

# A steward call to FerroPIX's operator API: `method` `path` with `body`;
# sets PIX_CODE and PIX_BODY.
hp_steward() {
  local method="$1" path="$2" body="${3:-}" out
  out="$(curl -s -w '\n%{http_code}' -X "$method" -H 'Host: localhost:8090' \
    -H "Authorization: Bearer $(hp_token steward __ferropix.steward)" \
    -H 'Content-Type: application/json' ${body:+--data-binary "$body"} "$HP_PIX$path")"
  PIX_CODE="${out##*$'\n'}"
  PIX_BODY="${out%$'\n'*}"
}

# A patient PERSON of the patient template named `name`, carrying the source
# record `mrn` at the patient identifier path and declaring the clinical
# pseudonym `pseudonym` (#5648, #5634).
j_patient() {
  local name="$1" mrn="$2" pseudonym="$3" archetype="openEHR-DEMOGRAPHIC-PERSON.patient.v1"
  jq -nc --arg a "$archetype" --arg n "$name" --arg s "$HP_SOURCE" --arg m "$mrn" \
    --arg c "$HP_CLINICAL" --arg p "$pseudonym" --argjson k "$(j_term record_subject)" '
    def text(v): {_type:"DV_TEXT",value:v};
    {_type:"PERSON",archetype_node_id:$a,
     archetype_details:{_type:"ARCHETYPED",rm_version:"1.1.0",
       archetype_id:{_type:"ARCHETYPE_ID",value:$a},
       template_id:{_type:"TEMPLATE_ID",value:"openEHR-DEMOGRAPHIC-PERSON.patient.v1.0.0"}},
     name:text($n),
     details:{_type:"ITEM_TREE",archetype_node_id:"id2",name:text("Details"),items:[
       {_type:"ELEMENT",archetype_node_id:"id3",name:text("Patient identifier"),
        value:{_type:"DV_IDENTIFIER",issuer:$s,id:$m,type:"MR"}},
       {_type:"ELEMENT",archetype_node_id:"id36",name:$k,
        value:{_type:"DV_IDENTIFIER",issuer:$c,id:$p,type:"PI"}}]},
     identities:[{_type:"PARTY_IDENTITY",archetype_node_id:"id6",name:text("Legal name"),
       details:{_type:"ITEM_TREE",archetype_node_id:"id7",name:text("Name structure"),
         items:[{_type:"ELEMENT",archetype_node_id:"id8",name:text("Full name"),value:text($n)}]}}]}'
}

# A care relationship of the patient template (its node id32): the patient
# PERSON `source` under the care of the GROUP `team` over `validity`, coded
# under_care_of of the access terminology.
j_patient_care() {
  local source="$1" team="$2" validity="$3"
  jq -nc --argjson k "$(j_term under_care_of)" --argjson s "$(j_party PERSON "$source")" \
    --argjson t "$(j_party GROUP "$team")" --argjson v "$validity" '{_type:"PARTY_RELATIONSHIP",
    archetype_node_id:"id32",name:{_type:"DV_TEXT",value:"Care relationship"},
    details:{_type:"ITEM_TREE",archetype_node_id:"id33",name:{_type:"DV_TEXT",value:"Care details"},
      items:[{_type:"ELEMENT",archetype_node_id:"id34",
        name:{_type:"DV_TEXT",value:"Care relationship code"},value:$k}]},
    time_validity:$v,source:$s,target:$t}'
}

# An EHR on the clinical pseudonym `pseudonym`; its id in `hv ehr_<name>`.
hp_ehr() {
  local name="$1" pseudonym="$2"
  h_call 1 admin POST /ehr "$(jq -nc --arg p "$pseudonym" --arg c "$HP_CLINICAL" '{_type:"EHR_STATUS",
    archetype_node_id:"openEHR-EHR-EHR_STATUS.generic.v1",name:{_type:"DV_TEXT",value:"EHR status"},
    archetype_details:{_type:"ARCHETYPED",rm_version:"1.1.0",
      archetype_id:{_type:"ARCHETYPE_ID",value:"openEHR-EHR-EHR_STATUS.generic.v1"}},
    subject:{_type:"PARTY_SELF",external_ref:{_type:"PARTY_REF",namespace:$c,type:"PERSON",
      id:{_type:"HIER_OBJECT_ID",value:$p}}},is_queryable:true,is_modifiable:true}')"
  hv_set "ehr_$name" "$(jq -r '.ehr_id.value // empty' <<<"$H_BODY")"
  [[ "$H_CODE" = 201 ]]
}

# Reads the composition `comp` of `ehr` through instance `n` as `who` until it
# answers `expected`, at most 60 seconds: the feed is applied off the request
# path. Asserts the last answer.
hp_read_until() {
  local n="$1" who="$2" ehr="$3" comp="$4" expected="$5" what="$6" tries=0
  while (( tries < 30 )); do
    h_call "$n" "$who" GET "/ehr/$ehr/composition/$(hv "$comp")" "" TREAT
    [[ "$H_CODE" = "$expected" ]] && break
    tries=$((tries + 1))
    sleep 2
  done
  assert_eq "$expected" "$H_CODE" "$what"
}

# What both instances and FerroPIX logged of the subscription and the feed,
# for a probe the feed did not reach.
hp_feed_log() {
  local service
  for service in ferroehr ferroehr-2 ferropix; do
    printf '      %s on the PMIR feed:\n' "$service"
    hdc logs --no-color "$service" 2>&1 \
      | grep -iE 'pmir|iti-93|iti-94|subscription|outbox|feed' | tail -25 | sed 's/^/        /'
  done
}

# The PIX merge and the steward revert, on the pixm stack hospital_pixm set
# up: Lisa under Cardiology's care, her duplicate record with an EHR of its
# own, merged into hers and split again.
# Lisa and her duplicate record: fed to FerroPIX, each with its patient
# PERSON and EHR, Lisa under Cardiology's care and the duplicate record holding
# vital signs. Written in hospital_pixm's setup, before the controller's
# ACCESS_POLICY holds every write to its rules.
hospital_pix_patients() {
  local fed mrn pseudonym
  for mrn in lisa-0001 lisa-0002; do
    fed="$(pix_feed "$mrn")"
    [[ "$fed" = 200 || "$fed" = 201 ]] || probe_fail "200 or 201 for the ITI-104 feed of $mrn" "$fed"
  done
  pseudonym="$(hp_pseudonym lisa-0001)"
  h_party person_lisa person "$(j_patient "Lisa Bos" lisa-0001 "$pseudonym")" \
    || h_fail_call "Lisa's patient PERSON"
  hp_ehr lisa "$pseudonym" || h_fail_call "Lisa's EHR"
  h_rerelate 1 admin person "$(hv person_lisa)" \
    "[$(j_patient_care "$(hv person_lisa)" "$(hv cardiology)" "$(j_dates "$(h_day -5)")")]" \
    || h_fail_call "Lisa's admission to Cardiology"
  pseudonym="$(hp_pseudonym lisa-0002)"
  h_party person_lisa_dup person "$(j_patient "L. Bos" lisa-0002 "$pseudonym")" \
    || h_fail_call "the duplicate record's patient PERSON"
  hp_ehr lisa_dup "$pseudonym" || h_fail_call "the duplicate record's EHR"
  h_commit c_lisa_dup 2 kc:anna "$(hv ehr_lisa_dup)" "$H_T_VITALS" \
    || h_fail_call "vital signs in the duplicate record"
}

hospital_pix_merge() {
  probe "P-HOSP-PIX-SETUP" "$PROBE_WORKING" "server" "#5693" \
    "Lisa and her duplicate record, fed to FerroPIX, each with its PERSON and EHR"
  local fed mrn
  # The ADT system corrects both records' names now that each PERSON carries
  # its record's identifier: FerroPIX announces each change, and FerroEHR
  # binds each identity to its PERSON before the merge arrives.
  for mrn in lisa-0001 lisa-0002; do
    fed="$(hp_feed "$mrn" Bos Lisa)"
    [[ "$fed" = 200 || "$fed" = 201 ]] || probe_fail "200 or 201 for the corrected feed of $mrn" "$fed"
  done
  h_client pix ferropix "$(hv hospital)" "$(j_capability "$(j_operation write_care_relationship)")" \
    || h_fail_call "FerroPIX's feed client, which writes record subjects"
  hv_set pix_lisa "$(hp_person lisa-0001)"
  hv_set pix_lisa_dup "$(hp_person lisa-0002)"
  [[ -n "$(hv pix_lisa)" && -n "$(hv pix_lisa_dup)" ]] || probe_fail "both FerroPIX persons" "missing"
  hp_read_until 1 kc:anna "$(hv ehr_lisa_dup)" c_lisa_dup 403 \
    "Dr. Jansen has no care relationship to the duplicate record's patient yet"
  probe_done

  probe "P-HOSP-PIX-MERGE" "$PROBE_WORKING" "server" "#5693" \
    "a steward's merge in FerroPIX reaches the duplicate record's EHR on both instances"
  hp_steward GET "/operator/merges/preview?survivor=$(hv pix_lisa)&merged=$(hv pix_lisa_dup)"
  [[ "$PIX_CODE" = 200 ]] || probe_fail "200 for the merge preview" "$PIX_CODE ${PIX_BODY:0:300}"
  hp_steward POST /operator/merges "$(jq -c --arg r "the same patient" \
    '{survivor:.survivor.id,survivor_version:.survivor.version,merged:.merged.id,
      merged_version:.merged.version,reason:$r}' <<<"$PIX_BODY")"
  [[ "$PIX_CODE" = 200 || "$PIX_CODE" = 201 ]] || probe_fail "the steward's merge" "$PIX_CODE ${PIX_BODY:0:300}"
  hv_set pix_merge "$(jq -r '.merge // empty' <<<"$PIX_BODY")"
  hp_read_until 2 kc:anna "$(hv ehr_lisa_dup)" c_lisa_dup 200 \
    "after the merge Dr. Jansen reads the duplicate record under Lisa's care relationship"
  hp_read_until 1 kc:anna "$(hv ehr_lisa_dup)" c_lisa_dup 200 \
    "the merge holds on the other instance"
  [[ "$H_CODE" = 200 ]] || hp_feed_log
  probe_done

  probe "P-HOSP-PIX-REVERT" "$PROBE_WORKING" "server" "#5693" \
    "a steward's revert in FerroPIX splits the duplicate record off again on both instances"
  hp_steward GET "/operator/merges/preview?revert=$(hv pix_merge)"
  [[ "$PIX_CODE" = 200 ]] || probe_fail "200 for the revert preview" "$PIX_CODE ${PIX_BODY:0:300}"
  hp_steward POST "/operator/merges/$(hv pix_merge)/revert" "$(jq -c --arg r "two patients" \
    '{survivor_version:.survivor.version,reason:$r}' <<<"$PIX_BODY")"
  [[ "$PIX_CODE" = 200 || "$PIX_CODE" = 201 ]] || probe_fail "the steward's revert" "$PIX_CODE ${PIX_BODY:0:300}"
  hp_read_until 1 kc:anna "$(hv ehr_lisa_dup)" c_lisa_dup 403 \
    "after the revert Dr. Jansen is refused the duplicate record again"
  hp_read_until 2 kc:anna "$(hv ehr_lisa_dup)" c_lisa_dup 403 \
    "the revert holds on the other instance"
  probe_done
}
