#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The Docker Compose probe family.
#
# Every probe here observes at the FAR END — a blob in the bucket, a claim in a
# token, the server's own effective configuration — rather than trusting a 2xx.
# Each one that carries `regression-of` reproduces a defect this project shipped:
# it must fail on the unfixed code and pass on the fixed code, which is what
# turns a one-off sweep into a permanent net.
#
# Sourced by scripts/deploy-probe.sh; never run directly.

# The compose stack this family drives. Ports are shifted off the defaults so a
# probe run can coexist with a developer's own stack.
COMPOSE_PROJECT="ferroehr-probe"
export FERROEHR_PORT="${PROBE_CDR_PORT:-18080}"
export FERROEHR_S3_PORT="${PROBE_S3_PORT:-18333}"
# Inert against the base file since #2879 (the quickstart publishes no DB
# port); kept for runs that add the db-publish overlay so a probe stack still
# coexists with a developer's own.
export FERROEHR_DB_PORT="${PROBE_DB_PORT:-15432}"
CDR="http://localhost:${FERROEHR_PORT}"
S3="http://localhost:${FERROEHR_S3_PORT}"
BASIC="ferroehr:ferroehr"
API="$CDR/ferroehr/rest/openehr/v1"

dc() { docker compose -p "$COMPOSE_PROJECT" "$@"; }

compose_down() { dc down -v --remove-orphans >/dev/null 2>&1 || true; }

# Bring the stack up with the multimedia keys taken from the ENVIRONMENT, which
# is the documented recipe — not by editing a file. That is deliberate: the
# recipe not working was itself a defect (#2169), so the harness must use the
# path the book tells an operator to use.
compose_up() {
  local -a services=("$@")
  # --wait is load-bearing: it gates on seaweedfs-init EXITING 0 (the compose
  # file's own contract — the initializer "exits 0 only in a state the probe
  # accepts, and `up --wait` gates on that exit"), which is also the recipe
  # exactly as the compose file documents it. Without it P-MM-BUCKET races
  # the one-shot initializer on a slow runner and measures the race, not the
  # deployment (the CI flake on the #2617 push run).
  dc -f docker-compose.yml --profile s3 up -d --wait "${services[@]}" >/dev/null 2>&1
}

# A composition-free way to commit a large DV_MULTIMEDIA: an EHR_STATUS carrying
# one. No template upload is needed, and it exercises the same versioning path a
# COMPOSITION does — which is exactly where #2197 lived.
#
# Built with `base64` and `printf` rather than a scripting language: #2178 pins
# the harness to bash and Rust.
probe_status_payload() {
  local bytes="${1:-409600}" out="$2"
  local b64
  b64="$(head -c "$bytes" /dev/urandom | base64 | tr -d '\n')"
  printf '%s' '{"_type":"EHR_STATUS",
    "name":{"_type":"DV_TEXT","value":"EHR status"},
    "archetype_node_id":"openEHR-EHR-EHR_STATUS.generic.v1",
    "archetype_details":{"_type":"ARCHETYPED",
      "archetype_id":{"_type":"ARCHETYPE_ID","value":"openEHR-EHR-EHR_STATUS.generic.v1"},
      "rm_version":"1.1.0"},
    "subject":{"_type":"PARTY_SELF"},
    "is_queryable":true,"is_modifiable":true,
    "other_details":{"_type":"ITEM_TREE",
      "name":{"_type":"DV_TEXT","value":"attachments"},
      "archetype_node_id":"at0001",
      "items":[{"_type":"ELEMENT",
        "name":{"_type":"DV_TEXT","value":"scan"},
        "archetype_node_id":"at0002",
        "value":{"_type":"DV_MULTIMEDIA",
          "media_type":{"_type":"CODE_PHRASE",
            "terminology_id":{"_type":"TERMINOLOGY_ID","value":"IANA_media-types"},
            "code_string":"application/octet-stream"},
          "size":' > "$out"
  printf '%s,"data":"%s"' "$bytes" "$b64" >> "$out"
  printf '%s' '}}]}}' >> "$out"
}

# Commit an EHR whose EHR_STATUS carries a 400 KiB DV_MULTIMEDIA; echo its id
# (empty when the commit did not succeed).
probe_commit_media_status() {
  local body="$PROBE_TMP/status.json" hdr="$PROBE_TMP/h.txt"
  probe_status_payload 409600 "$body"
  curl -s -u "$BASIC" -X POST -H 'Content-Type: application/json' \
    --data-binary "@$body" -D "$hdr" -o /dev/null "$API/ehr" || return 0
  grep -i '^location' "$hdr" 2>/dev/null | tr -d '\r' | awk -F/ '{print $NF}'
}

# The same commit, but reporting only its status code — for the states where the
# REFUSAL is the expected outcome.
probe_commit_media_status_code() {
  local body="$PROBE_TMP/status-broken.json"
  probe_status_payload 409600 "$body"
  curl -s -u "$BASIC" -X POST -H 'Content-Type: application/json' \
    --data-binary "@$body" -o /dev/null -w '%{http_code}' "$API/ehr"
}

# ── The families ──────────────────────────────────────────────────────────────

probes_shipped_config_boots() {
  bold "shipped configuration"

  # #2159: validate.sh rendered and linted; nothing ever STARTED what it
  # rendered, and none of the shipped values files produced a bootable server.
  probe "P-BOOT-01" "working" "compose" "#2159" \
    "the quickstart compose file boots to a serving CDR"
  if wait_http "$CDR/ferroehr/rest/status" 90; then
    local status; status="$(curl -s "$CDR/ferroehr/rest/status")"
    assert_contains "$status" '"status"' "the status document is served"
  else
    probe_fail "a serving CDR" "$( dc logs --tail 20 ferroehr 2>&1 | tail -5 )" \
      "the stack never answered /rest/status"
  fi
  probe_done

  probe "P-BOOT-02" "working" "image" "-" \
    "the health family answers without authentication, statuses only"
  assert_eq "200" "$(http_code "$CDR/health/liveness")"
  assert_eq "200" "$(http_code "$CDR/health/readiness")"
  assert_not_contains "$(curl -s "$CDR/health/readiness")" '"detail"' \
    "the public probe names each indicator and its status; the detail is /management/health"
  probe_done
}

probes_deployment_posture() {
  bold "deployment posture"

  # #3226 gave the server a declared posture and #3398/#3323 widened what it
  # measures: the audit pool's cluster, schema preparation per domain database,
  # and the two compatibility defaults. Nothing observed that a stack the
  # project ships reports any of it. Far end: the SERVER's own /rest/status,
  # not the configuration that was supposed to produce it.
  probe "P-POSTURE-01" "working" "image" "#3226" \
    "/rest/status carries the declared deployment posture"
  local status
  status="$(curl -s "$CDR/ferroehr/rest/status")"
  assert_contains "$status" '"deployment"' "the posture block is served"
  assert_contains "$status" '"profile":"sandbox"' \
    "the quickstart declares the sandbox profile"
  assert_not_contains "$status" '"gaps"' \
    "the open separations are weak points: the public document names none of them"
  assert_eq "401" "$(http_code "$CDR/management/status")" \
    "the full posture needs a credential"
  assert_contains "$(curl -s -u "$BASIC" "$CDR/management/status")" '"gaps"' \
    "the authenticated status document carries the open separations"
  probe_done

  # The quickstart runs all four domains on one database and applies its own
  # migrations, so the two separation gaps must be reported. The audit pool is
  # measured like the other three: its cluster is read from its own connection
  # (docs/law/ch/dpo/text-de.html Art. 4 Abs. 5 keeps the log "getrennt vom
  # System, in welchem die Personendaten bearbeitet werden").
  probe "P-POSTURE-02" "working" "image" "#3398" \
    "the co-located quickstart reports its separation gaps"
  assert_contains "$status" '"shared_credential"' \
    "one DSN for four domains is a shared credential"
  assert_contains "$status" '"shared_cluster"' \
    "one cluster for four domains, audit included, is a shared cluster"
  assert_contains "$status" '"migrate_on_runtime_credential"' \
    "the quickstart applies its own migrations on the runtime credential"
  probe_done

  # The shipped defaults are deliberate and disclosed, which is only true if
  # the running server actually discloses them (GDPR Art. 25(2),
  # docs/law/eu/gdpr/text.html).
  probe "P-POSTURE-03" "working" "image" "#3323" \
    "the two compatibility defaults are reported as open gaps"
  assert_contains "$status" '"open_ehr_access_default"' \
    "the open EHR_ACCESS default is disclosed rather than silent"
  assert_contains "$status" '"audit_fails_open"' \
    "the open audit fail mode is disclosed rather than silent"
  probe_done

  uncovered "the production profile's refusal" \
    "declaring deployment_profile = production would stop this stack booting at
     all, which the harness reads as a dead SUT rather than as the refusal it
     is. The refusal itself is unit-covered in config::deployment; what is not
     observed anywhere is a REAL server refusing to start on it."
  uncovered "an audit database in a cluster of its own" \
    "the audit half of the shared-cluster gap is proven here only by the
     co-located reading. Showing the gap CLOSE would need a second PostgreSQL
     cluster in the compose file with [storage.audit] url pointed at it, and a
     second migrated database is a stack change, not a probe."
}

probes_multimedia() {
  bold "multimedia (S3 externalization)"

  # #2169: the documented recipe exports FERROEHR__MULTIMEDIA__* in a shell.
  # Far end: the SERVER's own effective configuration, not the compose file.
  probe "P-MM-ENV" "working" "compose" "#2169" \
    "exported FERROEHR__MULTIMEDIA__* reaches the server"
  local env_doc; env_doc="$(curl -s -u "$BASIC" "$CDR/management/env")"
  assert_contains "$env_doc" '"endpoint":"http://seaweedfs:8333"' \
    "the compose file must pass the multimedia keys through from the shell"
  assert_contains "$env_doc" '"enabled":true'
  probe_done

  # #2168: the gateway ships with no bucket, and an S3 write into a missing one
  # answers 403 (not 404), so it reads as a credentials problem.
  probe "P-MM-BUCKET" "working" "compose" "#2168" \
    "the bucket exists with no manual step"
  assert_eq "200" "$(http_code -I "$S3/openehr-multimedia")" \
    "seaweedfs-init must create the bucket once the gateway is healthy"
  probe_done

  # The far-end observation the issue asks for: the blob is IN THE BUCKET,
  # under its content hash, and the stored record references it.
  probe "P-MM-OFFLOAD" "working" "server" "-" \
    "a large DV_MULTIMEDIA is externalized and the blob lands in the bucket"
  local uri
  PROBE_MEDIA_EHR="$(probe_commit_media_status)"
  local ehr="$PROBE_MEDIA_EHR" key=""
  if [[ -z "$ehr" ]]; then
    probe_fail "an EHR carrying a 400 KiB DV_MULTIMEDIA" "the commit did not return an id"
  else
    uri="$(curl -s -u "$BASIC" "$API/ehr/$ehr/ehr_status" \
      | tr ',' '\n' | grep -o 's3://openehr-multimedia/[0-9a-f]*' | head -1)"
    assert_contains "$uri" "s3://openehr-multimedia/" "the stored record must reference the blob"
    key="${uri##*/}"
    PROBE_MEDIA_KEY="$key"
    if [[ -n "$key" ]]; then
      assert_eq "200" "$(http_code "$S3/openehr-multimedia/$key")" \
        "the blob must be retrievable from the bucket by its content hash"
    fi
  fi
  probe_done

  # #2197: offload runs for every versioned object; re-inlining was wired to the
  # COMPOSITION read alone, so EHR_STATUS content had no way back.
  probe "P-MM-EXPAND" "working" "server" "#2197" \
    "?expand_multimedia=true returns the bytes on an EHR_STATUS read"
  if [[ -n "${ehr:-}" ]]; then
    local expanded data digest
    expanded="$(curl -s -u "$BASIC" "$API/ehr/$ehr/ehr_status?expand_multimedia=true")"
    data="$(printf '%s' "$expanded" \
      | jq -r '.other_details.items[0].value.data // empty' 2>/dev/null)"
    if [[ -z "$data" ]]; then
      probe_fail "a re-inlined DV_MULTIMEDIA.data" "no data member in the served value" \
        "the read must re-inline the blob, not answer with the compact reference"
    else
      # The strongest far-end check available: the bytes that came back must
      # hash to the content-addressed key the record references. That closes the
      # whole loop — committed, externalized under its SHA-256, fetched, and
      # re-inlined byte-identical — rather than trusting that a `data` member
      # appeared.
      digest="$(printf '%s' "$data" | base64 -d 2>/dev/null | shasum -a 256 | cut -d' ' -f1)"
      assert_eq "$key" "$digest" \
        "the re-inlined bytes must hash to the key the record references"
    fi
    # The `uri` deliberately SURVIVES expansion: RM DV_MULTIMEDIA's invariant is
    # `is_inline or is_external`, an OR, so carrying both is valid and keeps the
    # provenance reference. Asserting its absence would test a rule the spec
    # does not have.
  else
    probe_fail "an expandable record" "no EHR was committed"
  fi
  probe_done
}

probes_multimedia_restart() {
  bold "multimedia — persistence across a restart"

  # The case that separates a real object store from a temp directory: the
  # server process goes away and comes back, and the clinical content it
  # externalized is still retrievable byte-for-byte.
  #
  # It re-reads the record committed by P-MM-OFFLOAD rather than committing a
  # fresh one, because the property under test is that THAT content survived —
  # a new commit after the restart would prove only that the feature still
  # works, which is a different and weaker claim.
  probe "P-MM-RESTART" "working" "server" "-" \
    "externalized content survives a server restart, byte-for-byte"
  if [[ -z "${PROBE_MEDIA_EHR:-}" ]] || [[ -z "${PROBE_MEDIA_KEY:-}" ]]; then
    probe_fail "a record committed earlier in this run" "none was recorded" \
      "P-MM-OFFLOAD must run first — this probe deliberately re-reads its record"
    probe_done
    return 0
  fi

  dc restart ferroehr >/dev/null 2>&1
  if ! wait_http "$CDR/health/readiness" 120; then
    probe_fail "the CDR serving again after a restart" "readiness never returned"
    probe_done
    return 0
  fi

  # The blob is still in the store …
  assert_eq "200" "$(http_code "$S3/openehr-multimedia/$PROBE_MEDIA_KEY")" \
    "the object store outlives the server process"

  # … and the API still returns the bytes, verified against the same key.
  local data digest
  data="$(curl -s -u "$BASIC" "$API/ehr/$PROBE_MEDIA_EHR/ehr_status?expand_multimedia=true" \
    | jq -r '.other_details.items[0].value.data // empty' 2>/dev/null)"
  if [[ -z "$data" ]]; then
    probe_fail "the re-inlined bytes after a restart" "no data member in the served value" \
      "content committed before the restart must still be retrievable"
  else
    digest="$(printf '%s' "$data" | base64 -d 2>/dev/null | shasum -a 256 | cut -d' ' -f1)"
    assert_eq "$PROBE_MEDIA_KEY" "$digest" \
      "the bytes returned after the restart must hash to the original key"
  fi
  probe_done
}

probes_multimedia_off() {
  bold "multimedia — OFF (the default state)"

  # The default posture, and the state #2171 hid in: with externalization off a
  # large DV_MULTIMEDIA must be stored INLINE, byte-identical, with no
  # dependency on the object store at all. "Off" is the state most deployments
  # actually run, and it was never driven.
  #
  # Re-ups the CDR with the switch off; everything else is unchanged, so a
  # difference here is the switch and nothing else.
  probe "P-MM-OFF" "off" "server" "-" \
    "with externalization off, a large DV_MULTIMEDIA is stored inline"
  FERROEHR__MULTIMEDIA__ENABLED=false compose_up ferroehr
  if ! wait_http "$CDR/health/readiness" 90; then
    probe_fail "a serving CDR with multimedia off" "readiness never returned" \
      "turning an integration off must not stop the server starting"
    probe_done
    return 0
  fi

  local off_env; off_env="$(curl -s -u "$BASIC" "$CDR/management/env")"
  assert_contains "$off_env" '"enabled":false' "the switch must actually be off for this probe to mean anything"

  local ehr; ehr="$(probe_commit_media_status)"
  if [[ -z "$ehr" ]]; then
    probe_fail "a committed EHR" "the commit returned no id" \
      "an inline commit needs no object store and must succeed"
  else
    local stored; stored="$(curl -s -u "$BASIC" "$API/ehr/$ehr/ehr_status")"
    assert_contains "$stored" '"data"' "with the integration off the bytes stay in the record"
    assert_not_contains "$stored" 's3://' "nothing may be externalized while the switch is off"
  fi
  probe_done

  # Put the stack back the way the rest of the run expects it.
  compose_up ferroehr
  wait_http "$CDR/health/readiness" 90 || true
}

probes_multimedia_broken() {
  bold "multimedia — dependency broken"

  # The state least often tested, and where a system either fails loudly or
  # loses data quietly. With the store gone, a commit that must offload has to
  # be REFUSED, not half-stored.
  probe "P-MM-BROKEN" "broken" "server" "-" \
    "with the object store stopped, a commit that must offload is refused"
  dc stop seaweedfs >/dev/null 2>&1
  local code; code="$(probe_commit_media_status_code)"
  case "$code" in
    5*) : ;;
    *)  probe_fail "a 5xx refusal" "$code" \
          "an unreachable blob store must fail the commit, never store a half record" ;;
  esac
  dc start seaweedfs >/dev/null 2>&1
  wait_http "$S3/" 60 || true
  probe_done
}

probes_health_broken() {
  bold "health — dependency broken"

  # Readiness genuinely going unready. A probe that only ever returns UP is
  # worse than none, and this is the shape that keeps a dead pod in rotation.
  probe "P-HEALTH-BROKEN" "broken" "server" "-" \
    "database stopped: readiness 503, liveness still 200, no restart"
  dc stop ferroehr-postgres >/dev/null 2>&1
  if wait_status "$CDR/health/readiness" "503" 30; then
    assert_eq "200" "$(http_code "$CDR/health/liveness")" \
      "liveness must be process-local — restarting cannot fix a dependency"
    local body; body="$(curl -s "$CDR/health/readiness")"
    assert_contains "$body" '"status":"DOWN"' "readiness must name the failing component"
    assert_contains "$(curl -s -u "$BASIC" "$CDR/management/health")" '"detail"' \
      "the management view must say why the component is down"
  else
    probe_fail "readiness 503 within 60s" "$(curl -s -o /dev/null -w '%{http_code}' "$CDR/health/readiness")" \
      "a readiness probe that never fails cannot remove a pod from rotation"
  fi
  dc start ferroehr-postgres >/dev/null 2>&1
  wait_http "$CDR/health/readiness" 90 || true
  probe_done
}

probes_management() {
  bold "management surface"

  # #2177: the per-endpoint levels are the ONLY authority, and they are
  # INDEPENDENT. The quickstart ships four different levels at once, which is
  # exactly the shape a per-endpoint guard gets wrong:
  #
  #   prometheus = public       env    = admin_only
  #   info       = private      (flamegraph names no level ⇒ off)
  #
  # A guard that applied the most permissive configured level to every route —
  # or the strictest — passes a one-endpoint-at-a-time test and fails here.
  probe "P-MGMT-LEVELS" "working" "server" "#2177" \
    "each management endpoint answers at its own level, anonymous and authenticated"
  assert_eq "200" "$(http_code "$CDR/management/prometheus")" \
    "a public endpoint is served OUTSIDE authentication"
  assert_eq "401" "$(http_code "$CDR/management/info")" \
    "a private endpoint must challenge an anonymous caller"
  assert_eq "401" "$(http_code "$CDR/management/env")" \
    "an admin_only endpoint must challenge an anonymous caller"
  # The quickstart's Basic user carries ADMIN, so both open for it.
  assert_eq "200" "$(http_code -u "$BASIC" "$CDR/management/info")"
  assert_eq "200" "$(http_code -u "$BASIC" "$CDR/management/env")"
  probe_done

  # An endpoint that names no level is not mounted — 404, not 401. Its absence
  # is not a credential problem, and there is no global default that could open
  # it by accident.
  probe "P-MGMT-OFF" "off" "server" "#2177" \
    "an endpoint naming no level is not mounted, even for an admin"
  assert_eq "404" "$(http_code -u "$BASIC" "$CDR/management/flamegraph")" \
    "an unnamed endpoint must answer 404 rather than fall back to a server default"
  probe_done
}

# The management surface on its OWN listener — the second of the two
# configurations #2162 asks for, and the one a production deployment actually
# uses, because it is what keeps ops introspection off the public port.
#
# The property is not "the port answers" but that the surface MOVED: served on
# the management port and NO LONGER on the API port. A probe that only checked
# the new port would pass just as happily on a server that exposed the surface
# on both, which is the exact misconfiguration this option exists to prevent.
probes_management_separate_listener() {
  bold "management surface — its own listener"

  local overlay="$PROBE_TMP/mgmt-port.yml"
  local mport="${PROBE_MGMT_PORT:-19090}"
  cat > "$overlay" <<YAML
services:
  ferroehr:
    ports:
      - "127.0.0.1:${mport}:9090"
    environment:
      FERROEHR__MANAGEMENT__PORT: "9090"
YAML

  probe "P-MGMT-PORT" "working" "server" "#2162" \
    "with management.port set, the surface is served from its own listener"
  dc -f docker-compose.yml -f "$overlay" up -d ferroehr >/dev/null 2>&1
  if ! wait_http "$CDR/health/readiness" 120; then
    probe_fail "a serving CDR with a separate management listener" \
      "$(dc logs --tail 5 ferroehr 2>&1 | tail -3)" \
      "a second listener that fails to bind takes the whole process down at boot"
    probe_done
    dc -f docker-compose.yml up -d ferroehr >/dev/null 2>&1
    wait_http "$CDR/health/readiness" 120 || true
    return 0
  fi
  local mgmt="http://localhost:${mport}"
  if ! wait_http "$mgmt/management/prometheus" 30; then
    probe_fail "the management surface answering on its own port" \
      "$(http_code "$mgmt/management/prometheus")" \
      "management.port is documented to move the surface to a separate listener"
  fi
  probe_done

  # The half that makes the option worth having.
  probe "P-MGMT-PORT-MOVED" "broken" "server" "#2162" \
    "the surface is NO LONGER on the API listener once it has its own port"
  local on_api
  on_api="$(http_code "$CDR/management/prometheus")"
  case "$on_api" in
    404|403|401) : ;;
    200) probe_fail "the API port to stop serving /management" "$on_api" \
           "serving it on both ports defeats the point: the surface is still reachable from the public listener" ;;
    *)   probe_fail "404 from the API port" "$on_api" \
           "the API listener must not serve a surface that has moved" ;;
  esac
  probe_done

  # Back to the shipped posture for anything that follows.
  dc -f docker-compose.yml up -d ferroehr >/dev/null 2>&1
  wait_http "$CDR/health/readiness" 120 || true
}

# The pseudonymisation domains, and the ONE place every probe below takes its
# role names, schema names and dump identity from.
#
# Four enumerations were spelled out by hand here before the third domain
# arrived (#3220): the role roll-call, the NOINHERIT set, the roles the
# boundary reads with, and the schemas it reads into. A probe that enumerates a
# role set which no longer matches the database reports green about a boundary
# it is not testing — which is exactly what happened when `linkage` landed. So
# the enumeration exists once and every probe derives from it, and a fourth
# domain is one line here rather than an escape.
#
# One record per domain, `|`-separated:
#
#   1  name     — also the compose backup job's suffix and its dump filename
#                 prefix, and the FERROEHR_BACKUP_<NAME>_DIR the job reads
#   2  roles    — the runtime roles serving it (space-separated)
#   3  schemas  — the schemas the boundary covers: what a role belonging to
#                 ANOTHER domain must not be able to read (space-separated)
#   4  markers  — relations naming this domain in a dump's table of contents
#                 (`;`-separated). The first must be PRESENT in this domain's
#                 own dump; every one must be ABSENT from every other's.
#
# `ext` and `audit` appear in neither list 3 nor list 4 on purpose: they are
# shared infrastructure the clinical dump carries along, not a domain any role
# is barred from.
PROBE_DOMAINS=(
  "clinical|ferroehr_clinical ferroehr_clinical_reader|clinical|clinical version"
  "party|ferroehr_party ferroehr_party_reader|party|party version;party national_identifier"
  "linkage|ferroehr_linkage|linkage|linkage subject_ehr"
)

# Field <n> of a domain record.
domain_field() { printf '%s' "$1" | cut -d'|' -f"$2"; }

# Every runtime role in the table, one per line.
domain_all_roles() {
  local record role
  local -a roles
  for record in "${PROBE_DOMAINS[@]}"; do
    read -ra roles <<< "$(domain_field "$record" 2)"
    for role in "${roles[@]}"; do printf '%s\n' "$role"; done
  done
}

# A space-separated list as a SQL IN-list of quoted literals. Every value it
# ever sees is a literal from the table above, so there is nothing here a
# deployment could inject through.
sql_in_list() {
  local -a items
  local item out=""
  read -ra items <<< "$1"
  for item in "${items[@]}"; do out="${out:+$out,}'$item'"; done
  printf '%s' "$out"
}

# The domains whose schemas this stack's database does not actually carry,
# space-separated and empty when all of them are present.
#
# A local run defaults to the PUBLISHED server image, and a release that
# predates a domain migrates none of its schemas — so every probe about the
# boundary would answer about a database with nothing to separate, passing
# vacuously. CI builds the image from source
# (`FERROEHR_IMAGE: ferroehr:deploy-probe`); a run that does not is told what it
# is measuring instead of being allowed to look green.
missing_domains() {
  local record name tables
  for record in "${PROBE_DOMAINS[@]}"; do
    name="$(domain_field "$record" 1)"
    tables="$(dc exec -T ferroehr-postgres psql -qtAX -U "${PG_INIT_USER:-ferroehr}" \
      -d "${PG_INIT_DB:-ferroehr}" -c \
      "SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname IN ($(sql_in_list "$(domain_field "$record" 3)"))
          AND c.relkind = 'r'" 2>/dev/null)"
    [ "${tables:-0}" -gt 0 ] 2>/dev/null || printf '%s, ' "$name"
  done
}

# The one sentence every domain probe prints when it declines to measure.
domains_absent_reason() {
  printf '%s' "the stack's database carries no relations in the ${1%, } domain(s), so this
     measures nothing about the boundary. A local run defaults to the published
     server image (\`FERROEHR_IMAGE\`), which predates the domain split; CI builds
     it from source."
}

probes_domain_roles() {
  bold "pseudonymisation domain roles"

  local absent
  absent="$(missing_domains)"
  if [ -n "$absent" ]; then
    uncovered "the pseudonymisation domain roles" "$(domains_absent_reason "$absent")"
    return
  fi

  # #3179: the book documents the NOINHERIT runtime roles and a boot-time
  # self-check over their grants. Nothing observed that a stack the project
  # ships actually reaches that posture — the migrations create the roles only
  # when the migrator holds CREATEROLE, and skip them with a NOTICE otherwise,
  # which is silent. Far end: the DATABASE's own catalogue, not the compose
  # file that was supposed to arrange it.
  probe "P-ROLE-EXIST" "working" "compose" "#3179" \
    "every domain role exists in the database the stack booted against"
  local roles role
  roles="$(dc exec -T ferroehr-postgres psql -qtAX -U ferroehr -d ferroehr -c \
    "SELECT rolname FROM pg_roles WHERE rolname LIKE 'ferroehr_%' ORDER BY rolname" 2>/dev/null)"
  while read -r role; do
    assert_contains "$roles" "$role" "the compose init provisions $role"
  done <<< "$(domain_all_roles)"
  probe_done

  # A role that could inherit the other domain's grants would make the boundary
  # a naming convention (PostgreSQL 18, CREATE ROLE).
  probe "P-ROLE-NOINHERIT" "working" "compose" "#3179" \
    "each domain role is NOINHERIT"
  local inheriting
  inheriting="$(dc exec -T ferroehr-postgres psql -qtAX -U ferroehr -d ferroehr -c \
    "SELECT rolname FROM pg_roles
      WHERE rolname IN ($(sql_in_list "$(domain_all_roles | tr '\n' ' ')"))
        AND rolinherit" 2>/dev/null)"
  if [ -n "$inheriting" ]; then
    probe_fail "every domain role NOINHERIT" "$inheriting" \
      "an inheriting domain role can pick up another domain's grants through membership"
  fi
  probe_done

  # The property the server refuses to boot without. Asking the database
  # directly is the far end: the boot check passing proves the server's opinion,
  # this proves the catalogue's.
  #
  # One query per domain: that domain's roles against every OTHER domain's
  # schemas, which is the same barrier table the server enforces at boot. The
  # roles are taken through `pg_roles` so an absent role is skipped rather than
  # raising — P-ROLE-EXIST above is what keeps that from turning into silence.
  probe "P-ROLE-BOUNDARY" "working" "database" "#3179" \
    "no domain's roles can read another domain's relations"
  local record name others other_record breach
  for record in "${PROBE_DOMAINS[@]}"; do
    name="$(domain_field "$record" 1)"
    others=""
    for other_record in "${PROBE_DOMAINS[@]}"; do
      if [ "$(domain_field "$other_record" 1)" = "$name" ]; then continue; fi
      others="${others:+$others }$(domain_field "$other_record" 3)"
    done
    breach="$(dc exec -T ferroehr-postgres psql -qtAX -U ferroehr -d ferroehr -c \
      "SELECT r.rolname || ' -> ' || n.nspname || '.' || c.relname
         FROM pg_class c
         JOIN pg_namespace n ON n.oid = c.relnamespace
         CROSS JOIN (SELECT rolname FROM pg_roles
                      WHERE rolname IN ($(sql_in_list "$(domain_field "$record" 2)"))) AS r
        WHERE n.nspname IN ($(sql_in_list "$others"))
          AND c.relkind IN ('r','p','v','m','f')
          AND has_table_privilege(r.rolname, c.oid, 'SELECT')
        LIMIT 5" 2>/dev/null)"
    if [ -n "$breach" ]; then
      probe_fail "no $name role reaching another domain" "$breach" \
        "the boundary is the whole point of the split; a readable relation defeats it"
    fi
  done
  probe_done

  # And the honest half: this stack runs ONE login credential, so the schema
  # separation is demonstrated and the credential separation is not. Recorded
  # as a measurement rather than left to the reader to assume either way.
  probe "P-ROLE-ONE-CREDENTIAL" "working" "compose" "#3179" \
    "the compose stack is single-credential by design, and says so"
  local party_dsn
  party_dsn="$(curl -s -u "$BASIC" "$CDR/management/env" | grep -o '"party":{[^}]*}' || true)"
  case "$party_dsn" in
    ''|*'"url":null'*|*'"url":""'*) : ;;
    *) probe_fail "no separate party DSN in the demo stack" "$party_dsn" \
         "if the stack grew one, this probe's premise is stale and the note in the init script is wrong" ;;
  esac
  probe_done

  uncovered "the CREDENTIAL separation AT DEPLOYMENT LEVEL" \
    "this stack still runs one login role that is a member of every domain, so what is
     measured above is the SCHEMA separation and the grant boundary. The in-process half
     is covered elsewhere as of #3222: app/ferroehr-rest/tests/it/credential_separation.rs
     assembles the server on one login role per domain against a real PostgreSQL, serves
     both domains through the wire, and asserts that each of the server's own pools is
     refused the other domain's relations with SQLSTATE 42501. What remains uncovered by
     any probe is a real DEPLOYMENT on two DSNs: a container booting with
     FERROEHR__STORAGE__PARTY__URL_FILE mounted, schema preparation by a role that is
     neither runtime credential, and the chart's database.party.existingSecret
     and database.linkage.existingSecret wiring the three. The server opens four
     pools (db::connect_domains over [storage.clinical], [storage.party],
     [storage.linkage] and [storage.audit], each defaulting to [db] url) and the
     boundary suite exercises all three credentials in-process; no probe here boots a
     container on three DSNs."
  uncovered "role provisioning on a managed database" \
    "the compose init creates the domain roles as the bootstrap superuser. A managed
     PostgreSQL where the migrator holds no CREATEROLE takes the documented manual
     step instead, and nothing here runs that path."
}

# Create a subdirectory of <dir> per domain and point every backup job at it,
# under the FERROEHR_BACKUP_<NAME>_DIR names the compose file reads, so one
# `run` per domain writes where this harness can find it and no directory name
# is spelled twice.
export_backup_dirs() {
  local record name
  for record in "${PROBE_DOMAINS[@]}"; do
    name="$(domain_field "$record" 1)"
    mkdir -p "$1/$name"
    export "FERROEHR_BACKUP_$(printf '%s' "$name" | tr '[:lower:]' '[:upper:]')_DIR=$1/$name"
  done
}

probes_backup_restore() {
  bold "per-domain logical backups and the restore self-check"

  local absent
  absent="$(missing_domains)"
  if [ -n "$absent" ]; then
    uncovered "per-domain logical backups" "$(domains_absent_reason "$absent")"
    return
  fi

  # #3157: the book documents per-schema dumps into separate targets and a
  # restore that the boot self-check gates. Far end: the DUMP FILES the shipped
  # compose profile actually writes, and the server's own verdict on a database
  # restored from them — not the compose file that was supposed to arrange it.
  local dumps="$PROBE_TMP/backup"
  export_backup_dirs "$dumps"

  probe "P-BACKUP-SPLIT" "working" "compose" "#3157" \
    "the backup profile writes one dump per pseudonymisation domain"
  # The recipe's own output is KEPT: a dump job that fails is the finding, and
  # discarding its stderr would report the missing file without the reason.
  # Compose's own progress and orphan-container chatter is dropped: it is
  # longer than the diagnostic underneath it, and a truncated note that shows
  # the chatter instead of the error is worse than no note.
  local dump_log="" dump_status=0
  local record name status raw
  for record in "${PROBE_DOMAINS[@]}"; do
    name="$(domain_field "$record" 1)"
    # The job's EXIT STATUS is the assertion, not the presence of a file:
    # `pg_dump --file` creates its output before it connects, so a dump that
    # dies part-way leaves an artefact behind. Checking only for a file lets a
    # truncated archive through and throws the reason away — which is exactly
    # what happened here (#3157), and cost several runs.
    # The status is taken from the RUN, before any filtering: a pipeline
    # reports its last stage, which would be the noise filter's.
    raw="$(dc -f docker-compose.yml --profile backup run --rm --quiet-pull \
        "ferroehr-backup-$name" 2>&1)"
    status=$?
    [ "$status" -eq 0 ] || dump_status=$status
    dump_log="${dump_log}[$name exit=$status] $(printf '%s' "$raw" \
      | grep -vE '^time="|^ *Container |orphan containers' | tr '\n' ' ')"
  done
  if [ "$dump_status" -ne 0 ]; then
    probe_fail "every dump job exits 0" "exit $dump_status" \
      "the recipe reported: $(printf '%s' "$dump_log" | tail -c 400)"
    probe_done
    return
  fi
  # Parallel to PROBE_DOMAINS by index: the dump each domain's job wrote.
  local -a dump_files=()
  local found missing=""
  for record in "${PROBE_DOMAINS[@]}"; do
    name="$(domain_field "$record" 1)"
    found="$(find "$dumps/$name" -name "$name-*.dump" -print -quit 2>/dev/null)"
    dump_files+=("$found")
    [ -n "$found" ] || missing="${missing:+$missing }$name"
  done
  if [ -n "$missing" ]; then
    # A dump that produced no file says nothing about WHY on its own, and the
    # answer is usually about the target rather than about postgres: who the
    # job runs as, and what the mount looks like from inside it.
    local target_state
    target_state="$(dc -f docker-compose.yml --profile backup run --rm --quiet-pull \
        --entrypoint /bin/sh "ferroehr-backup-${missing%% *}" -c \
        'id; ls -ldn /backup; grep " /backup " /proc/mounts; touch /backup/.probe-write 2>&1' 2>&1 \
      | grep -vE '^time="|^ *Container |orphan containers' | tr '\n' ' ')"
    probe_fail "one dump file in each domain's target directory" \
      "no file for: $missing" \
      "recipe: ${dump_log:0:300} || target from inside the job: ${target_state:0:400} \
|| on the host: $(ls -ldn "$dumps/${missing%% *}" 2>&1)"
    probe_done
    return
  fi
  probe_done

  # The property the split exists for: no artefact carries another domain. A
  # dump that named two of them would re-join what the schema separation keeps
  # apart (GDPR Art. 4(5)), and it would do so silently. Checked PAIRWISE, so
  # the third domain is covered against both of the others rather than only
  # against the one it was added beside.
  probe "P-BACKUP-DISJOINT" "working" "compose" "#3157" \
    "no dump carries another domain's relations"
  local -a tocs=()
  local i j marker
  local -a markers
  for i in "${!PROBE_DOMAINS[@]}"; do
    name="$(domain_field "${PROBE_DOMAINS[$i]}" 1)"
    tocs+=("$(docker run --rm -v "$dumps/$name:/backup:ro" \
      "${FERROEHR_POSTGRES_IMAGE:-ghcr.io/ferrohealth/ferroehr-postgres:4.1.1}" \
      pg_restore --list "/backup/$(basename "${dump_files[$i]}")" 2>/dev/null)")
  done
  for i in "${!PROBE_DOMAINS[@]}"; do
    name="$(domain_field "${PROBE_DOMAINS[$i]}" 1)"
    IFS=';' read -ra markers <<< "$(domain_field "${PROBE_DOMAINS[$i]}" 4)"
    assert_contains "${tocs[$i]}" "${markers[0]}" \
      "the $name dump must carry ${markers[0]}; the dump job reported: ${dump_log:0:300}"
    for j in "${!PROBE_DOMAINS[@]}"; do
      if [ "$i" -eq "$j" ]; then continue; fi
      for marker in "${markers[@]}"; do
        assert_not_contains "${tocs[$j]}" "$marker" \
          "a $(domain_field "${PROBE_DOMAINS[$j]}" 1) backup carrying $marker defeats the split"
      done
    done
  done
  probe_done

  # The restore, judged by the server rather than by the restore's own exit
  # code: `ferroehr db verify` is the boot self-check (the migrations this
  # build carries, then the domain-isolation gate over the runtime roles).
  probe "P-BACKUP-RESTORE" "working" "database" "#3157" \
    "a database restored from every domain's dump passes the boot self-check"
  local restored=ferroehr_restored
  # The database is created by the BOOTSTRAP superuser and owned by the
  # application role, which is what a restore runbook does: the application
  # role holds no CREATEDB, and a database owned by `postgres` would refuse
  # the restore its schemas. Errors are kept — a suppressed CREATE DATABASE
  # turns into a confusing "database does not exist" three steps later.
  local create_log
  create_log="$(dc exec -T ferroehr-postgres psql -qtAX -U postgres \
    -d "${PG_INIT_DB:-ferroehr}" \
    -c "DROP DATABASE IF EXISTS $restored" \
    -c "CREATE DATABASE $restored OWNER ${PG_INIT_USER:-ferroehr}" 2>&1)"
  # The DSN is taken from the SERVER CONTAINER's own environment and its
  # database name swapped, rather than rebuilt from the compose defaults: the
  # restore and the verify then use the credential the stack actually runs,
  # and no credential literal lives in this harness.
  local restored_dsn verify_out
  restored_dsn="$(docker inspect --format '{{range .Config.Env}}{{println .}}{{end}}' \
    "$(dc ps -q ferroehr)" 2>/dev/null | sed -n 's/^FERROEHR__DB__URL=//p')"
  if [ -z "$restored_dsn" ]; then
    probe_fail "the server container's FERROEHR__DB__URL" "(empty)" \
      "without it this probe would build a DSN of its own and measure that instead"
    probe_done
    return
  fi
  restored_dsn="${restored_dsn%/*}/$restored"
  # The restore runs the way an operator's would: a client container on the
  # stack's network with the dump directory mounted, addressed by DSN. It is
  # the same `docker run` shape the table-of-contents read above uses, because
  # that one demonstrably reaches these files.
  #
  # The dumps go in TABLE ORDER, which is why `clinical` is first: the other
  # domains' relations depend on the `ext` helpers, and that schema travels
  # with the clinical dump.
  local restore_script="id; ls -ln" restore_log network
  for record in "${PROBE_DOMAINS[@]}"; do
    restore_script="$restore_script /dumps/$(domain_field "$record" 1)"
  done
  restore_script="$restore_script;"
  for i in "${!PROBE_DOMAINS[@]}"; do
    name="$(domain_field "${PROBE_DOMAINS[$i]}" 1)"
    restore_script="$restore_script
      pg_restore --dbname='$restored_dsn' --no-owner '/dumps/$name/$(basename "${dump_files[$i]}")';"
  done
  network="$(docker inspect \
    --format '{{range $net, $_ := .NetworkSettings.Networks}}{{$net}}{{end}}' \
    "$(dc ps -q ferroehr-postgres)" 2>/dev/null)"
  restore_log="$(docker run --rm --network "$network" -v "$dumps:/dumps:ro" \
    "${FERROEHR_POSTGRES_IMAGE:-ghcr.io/ferrohealth/ferroehr-postgres:4.1.1}" \
    sh -c "$restore_script" 2>&1)"
  if verify_out="$(dc exec -T -e FERROEHR__DB__URL="$restored_dsn" -e FERROEHR__DB__MIGRATE=verify \
      ferroehr /usr/local/bin/ferroehr db verify 2>&1)"; then
    :
  else
    probe_fail "the restored database passes \`ferroehr db verify\`" \
      "${verify_out:0:400}" \
      "create: ${create_log:0:150} || network: '${network}' || restore: $(printf '%s' \
        "$restore_log" | tr '\n' ' ' | tail -c 600)"
  fi
  probe_done

  # `ferroehr db verify` reads migration bookkeeping and grants; it never looks
  # at a constraint. So a restore can satisfy it while the map came back
  # STRUCTURALLY wrong, and the linkage domain is where that actually happens:
  # a `--schema` dump carries no extension (PostgreSQL 18, pg_dump §Notes),
  # subject_ehr's temporal UNIQUE … WITHOUT OVERLAPS is a GiST index over
  # btree_gist operator classes, and pg_restore IGNORES a failed statement by
  # default. Measured 2026-09-11 on 18.6: without `--extension=btree_gist` the
  # table comes back with its rows and without the key that admits one open
  # mapping per party. Far end: the restored catalogue itself.
  probe "P-BACKUP-MAP-RESTORED" "working" "database" "#3220" \
    "the restored linkage map carries its temporal key"
  local map_shape
  map_shape="$(dc exec -T ferroehr-postgres psql -qtAX -U postgres -d "$restored" -c \
    "SELECT coalesce((SELECT pg_get_constraintdef(oid) FROM pg_constraint
                       WHERE conrelid = 'linkage.subject_ehr'::regclass
                         AND contype = 'u'), 'no temporal key')" 2>&1)"
  assert_contains "$map_shape" "WITHOUT OVERLAPS" \
    "a subject_ehr restored without its temporal key admits two open mappings for one \
party, and nothing downstream would notice"
  probe_done

  # And the half that makes the probe above mean something: the self-check has
  # to REFUSE a restore whose grants came back wrong. A runbook that re-applies
  # access with a blanket GRANT is the realistic way that happens.
  probe "P-BACKUP-GRANTS-REFUSED" "broken" "database" "#3157" \
    "the self-check refuses a restore whose grants cross the domain boundary"
  dc exec -T ferroehr-postgres psql -qtAX -U "${PG_INIT_USER:-ferroehr}" -d "$restored" -c \
    "GRANT USAGE ON SCHEMA party TO ferroehr_clinical_reader;
     GRANT SELECT ON ALL TABLES IN SCHEMA party TO ferroehr_clinical_reader" >/dev/null 2>&1
  local breach_out
  if breach_out="$(dc exec -T -e FERROEHR__DB__URL="$restored_dsn" \
      -e FERROEHR__DB__MIGRATE=verify \
      ferroehr /usr/local/bin/ferroehr db verify 2>&1)"; then
    probe_fail "\`ferroehr db verify\` refusing a cross-domain grant" \
      "it accepted a database where ferroehr_clinical_reader can read party tables" \
      "the boot gate is then decorative, and a careless restore ships a collapsed boundary"
  else
    # A refusal for ANY other reason would make this probe pass without
    # measuring the gate at all — the vacuity this harness exists to avoid.
    assert_contains "$breach_out" "ferroehr_clinical_reader" \
      "the refusal must name the role that reached across, not merely be a refusal"
    assert_contains "$breach_out" "party." \
      "the refusal must name the schema it reached into"
  fi
  probe_done

  dc exec -T ferroehr-postgres psql -qtAX -U "${PG_INIT_USER:-ferroehr}" \
    -d "${PG_INIT_DB:-ferroehr}" -c "DROP DATABASE IF EXISTS $restored" >/dev/null 2>&1

  uncovered "point-in-time recovery" \
    "PITR stays instance-wide by design (the domains share one cluster), so
     nothing here exercises a WAL archive or a recovery target. What is measured
     is the LOGICAL per-domain dump and the restore's grant posture."
  uncovered "an off-host backup target" \
    "every dump lands in a directory on the machine running the stack. Whether a
     deployment's targets are genuinely separately access-controlled — different
     owners, different buckets, different credentials — is an operator property this
     harness cannot observe."
}
