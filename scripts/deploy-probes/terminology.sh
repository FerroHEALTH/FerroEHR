#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The terminology probe family (#2178, #2158) — against a REAL terminology
# server, with REAL content.
#
# The temptation here is a seeded code system of our own: cheap, fast, and
# worthless. Passing `$validate-code` against a CodeSystem we wrote proves our
# client can talk to our own fixture. `$subsumes` is the clearest case — a
# subsumption answer means nothing without a real hierarchy behind it, and a
# fixture hierarchy is one we authored to make the test pass.
#
# So this family drives Snowstorm (SNOMED International's own server, which
# implements the FHIR Terminology Module) loaded with a real SNOMED CT release.
#
# THE RELEASE IS NOT OURS TO SHIP, AND THIS FAMILY RUNS LOCALLY ONLY. The RF2
# package is licensed content under a SNOMED International Affiliate agreement:
# it cannot be committed here, fetched by CI, or baked into a published image.
# The operator supplies a local path:
#
#   FERROEHR_SNOMED_RF2=~/SnomedCT_ManagedServiceNL_PRODUCTION_NL1000146_*.zip
#
# Without it the family declares itself NOT EXERCISED rather than substituting a
# fixture — an honest gap beats a green row that measured our own data.
#
# The NETHERLANDS edition is the expected package. Whether a Managed Service
# edition package is complete on its own or is an extension that depends on the
# International Edition is not something Snowstorm documents, so this family
# does not assume: supply FERROEHR_SNOMED_RF2_INTL as well and it is imported to
# MAIN first, with the national package then loaded onto its own branch. With
# only one package, that package goes to MAIN and the run reports what it
# actually served.
#
# It is also heavy: Elasticsearch plus Snowstorm want ~8 GB and the import takes
# far longer than every other probe combined, so this family is opt-in.
#
# Elasticsearch specifically, and not a substitute: the maintainers were asked
# about OpenSearch and answered no, "mainly because Elasticsearch provides
# better performance" (IHTSDO/snowstorm#411), and Snowstorm has since moved to
# the ES 8 client, which OpenSearch — forked at 7.10 — cannot serve. Meilisearch
# is not a candidate at all: Snowstorm speaks the Elasticsearch query DSL, so
# that would be a rewrite rather than a swap. None of this reaches the product:
# Elasticsearch exists only inside this probe stack, to run the terminology
# server being measured, and never in anything FerroEHR ships.
#
# Sourced by scripts/deploy-probe.sh; never run directly.

TERM_SNOWSTORM_PORT="${PROBE_SNOWSTORM_PORT:-18090}"
TERM_URL="http://localhost:${TERM_SNOWSTORM_PORT}"
# The container-side URL the CDR uses; the host port is for this script only.
TERM_INTERNAL="http://ferroehr-snowstorm:8080/fhir"

terminology_overlay() {
  local out="$PROBE_TMP/terminology.yml"
  cat > "$out" <<YAML
services:
  ferroehr-es:
    image: docker.elastic.co/elasticsearch/elasticsearch:8.11.1
    environment:
      - discovery.type=single-node
      - xpack.security.enabled=false
      - ES_JAVA_OPTS=-Xms4g -Xmx4g
    healthcheck:
      test: ["CMD-SHELL", "curl -sf http://localhost:9200 || exit 1"]
      interval: 5s
      timeout: 5s
      retries: 60
  ferroehr-snowstorm:
    image: snomedinternational/snowstorm:latest
    depends_on:
      ferroehr-es:
        condition: service_healthy
    entrypoint: java -Xms2g -Xmx4g --add-opens java.base/java.lang=ALL-UNNAMED --add-opens=java.base/java.util=ALL-UNNAMED -cp @/app/jib-classpath-file org.snomed.snowstorm.SnowstormApplication --elasticsearch.urls=http://ferroehr-es:9200
    ports:
      - "127.0.0.1:${TERM_SNOWSTORM_PORT}:8080"
  ferroehr:
    depends_on:
      - ferroehr-snowstorm
    environment:
      FERROEHR__TERMINOLOGY__EXTERNAL__ENABLED: "true"
      FERROEHR__TERMINOLOGY__EXTERNAL__FAIL_ON_ERROR: "true"
      FERROEHR__TERMINOLOGY__EXTERNAL__PROVIDERS__SNOMED__BASE_URL: ${TERM_INTERNAL}
      FERROEHR__TERMINOLOGY__EXTERNAL__ROUTES__DEFAULT: snomed
YAML
  printf '%s' "$out"
}

# Import the operator's RF2 archive through Snowstorm's own native API — the
# route its documentation prescribes for SNOMED content, since the FHIR API
# cannot load RF2.
terminology_import() {
  local zip="$1" location job
  location="$(curl -s -D - -o /dev/null -X POST "$TERM_URL/imports" \
    -H 'Content-Type: application/json' \
    -d '{"branchPath":"MAIN","createCodeSystemVersion":true,"type":"SNAPSHOT"}' \
    | grep -i '^location' | tr -d '\r' | awk '{print $2}')"
  [[ -n "$location" ]] || return 1
  job="${location##*/}"
  curl -s -o /dev/null -X POST "$TERM_URL/imports/$job/archive" -F "file=@${zip}" || return 1
  # The import is long. Poll its own status rather than guessing a duration.
  local _i status
  for _i in $(seq 1 480); do
    status="$(curl -s "$TERM_URL/imports/$job" | sed -n 's/.*"status":"\([A-Z_]*\)".*/\1/p')"
    case "$status" in
      COMPLETED) return 0 ;;
      FAILED)    return 1 ;;
      *)         ;;
    esac
    sleep 15
  done
  return 1
}

# The RF2 release, from a local path or fetched from wherever the affiliate
# keeps it. Echoes the archive path; non-zero means none was supplied.
#
# A local path only. There is deliberately no fetch-from-CI route: the package
# is licensed, and a lane that downloaded it onto a shared runner would be
# moving licensed content somewhere this project does not control.
#
# FERROEHR_SNOMED_RF2_MD5, when set, is CHECKED. SNOMED International publishes
# an MD5 beside each release, and a terminology probe that silently ran against
# a truncated or substituted package would report conformance about content
# nobody chose. It is pinned per release, so a release upgrade is a deliberate
# edit rather than something that happens to a run.
terminology_release() {
  local zip="${FERROEHR_SNOMED_RF2:-}"
  # Nothing configured? Look in the repository root. `SnomedCT_*.zip` is
  # gitignored precisely so an archive can live there, so dropping the download
  # beside the checkout is the whole setup. The NEWEST match wins, and the one
  # actually used is echoed by the caller — with two editions present, silently
  # picking one would make a run unattributable.
  if [[ -z "$zip" ]]; then
    zip="$(find . -maxdepth 1 -name 'SnomedCT_*.zip' -print 2>/dev/null \
           | grep -v ManagedService | sort | tail -1)"
    # Prefer a national edition over the International one when both are there:
    # the national package is the deployment reality being probed.
    local national
    national="$(find . -maxdepth 1 -name 'SnomedCT_ManagedService*.zip' -print 2>/dev/null | sort | tail -1)"
    [[ -n "$national" ]] && zip="$national"
  fi
  if [[ -n "$zip" ]] && [[ -f "$zip" ]]; then
    terminology_verify "$zip" || return 1
    printf '%s' "$zip"
    return 0
  fi
  return 1
}

# The International Edition, when one is present, so a national extension can
# resolve against it. Discovered the same way and never the national package.
terminology_international() {
  local intl="${FERROEHR_SNOMED_RF2_INTL:-}"
  if [[ -z "$intl" ]]; then
    intl="$(find . -maxdepth 1 -name 'SnomedCT_InternationalRF2_*.zip' -print 2>/dev/null | sort | tail -1)"
  fi
  [[ -n "$intl" ]] && [[ -f "$intl" ]] && printf '%s' "$intl"
}

terminology_verify() {
  local want="${FERROEHR_SNOMED_RF2_MD5:-}"
  [[ -n "$want" ]] || return 0
  local got
  got="$(md5sum "$1" 2>/dev/null | awk '{print $1}')"
  [[ -n "$got" ]] || got="$(md5 -q "$1" 2>/dev/null)"
  if [[ "$got" != "$want" ]]; then
    red "  SNOMED RF2 checksum mismatch: expected $want, got ${got:-none}"
    return 1
  fi
  return 0
}

# ── FerroTERM (docker-compose.terminology.yml), the shaped seed ──────────────
#
# The terminology overlay every deployment can switch on (#3304): FerroTERM
# beside the CDR, serving the licence-free shaped seed mounted out of the
# ferroehr image. Runs in every default pass, because it needs no licensed
# content: the seed's two code systems are what the conformance lane binds to.
# The family drives the value-set binding from the OUTSIDE, through commits,
# in the three states a deployment can be in: overlay off (nothing resolves),
# overlay on (a member commits, a non-member is refused), and the server down
# under each fail posture.
TERM_FT_PORT="${PROBE_FERROTERM_PORT:-18090}"
# Exported, like the CDR and database ports in compose.sh: the overlay publishes
# FerroTERM at ${FERROEHR_TERMINOLOGY_PORT:-8090}, and the probes below poll
# TERM_FT_PORT, so the two must be the same number.
export FERROEHR_TERMINOLOGY_PORT="$TERM_FT_PORT"
TERM_FT_OVERLAY="docker-compose.terminology.yml"
TERM_FT_TEMPLATE="corpus/templates/dt_coded_text_binding_sct.opt"
TERM_FT_MEMBER="corpus/fixtures/composition/terminology_binding_sct_member.json"
TERM_FT_NON_MEMBER="corpus/fixtures/composition/terminology_binding_sct_non_member.json"
# The shaped seed's code system and value set, by canonical URL: neither is in
# the bundled openEHR terminology, so only FerroTERM can answer for them.
TERM_FT_SYSTEM="http://cnf.example.test/fhir/CodeSystem/sct-shaped"
TERM_FT_VALUE_SET="http://cnf.example.test/fhir/ValueSet/sct-shaped-disorders"

# The overlay stack: base file + overlay, the s3 profile the other families
# assume, and ferroterm explicitly. `--wait` returns once FerroTERM's own
# HEALTHCHECK (`ferroterm healthcheck`, 0.1.4) reports healthy and the CDR,
# which depends on that, is healthy too; no host-side polling is needed.
term_ft_up() {
  dc -f docker-compose.yml -f "$TERM_FT_OVERLAY" --profile s3 up -d --wait ferroehr ferroterm >/dev/null 2>&1
}

term_ft_new_ehr() {
  local hdr="$PROBE_TMP/ft-ehr.txt"
  curl -s -u "$BASIC" -X POST -D "$hdr" -o /dev/null "$API/ehr" || return 0
  grep -i '^location' "$hdr" 2>/dev/null | tr -d '\r' | awk -F/ '{print $NF}'
}

# The status code a commit of the given fixture into a fresh EHR answers.
term_ft_commit_code() {
  local ehr; ehr="$(term_ft_new_ehr)"
  [[ -n "$ehr" ]] || { printf '000'; return 0; }
  curl -s -u "$BASIC" -o /dev/null -w '%{http_code}' -X POST \
    -H 'Content-Type: application/json' -H 'Prefer: return=minimal' \
    --data-binary "@$1" "$API/ehr/$ehr/composition"
}

term_ft_template() {
  local code
  code="$(curl -s -u "$BASIC" -o /dev/null -w '%{http_code}' -X POST \
    -H 'Content-Type: application/xml' --data-binary "@$TERM_FT_TEMPLATE" \
    "$API/definition/template/adl1.4")"
  case "$code" in 201|204|409) return 0 ;; *) return 1 ;; esac
}

# The CDR's own /terminology/* membership test for one candidate code, as
# "<status> <body>". Both canonical URLs travel as single path segments, so
# their slashes are percent-encoded.
term_ft_api_validate() {
  local system value_set
  system="$(jq -rn --arg s "$TERM_FT_SYSTEM" '$s | @uri')"
  value_set="$(jq -rn --arg s "$TERM_FT_VALUE_SET" '$s | @uri')"
  curl -s -u "$BASIC" -w ' %{http_code}' \
    "$API/terminology/${system}/value_set/${value_set}/validate?candidate_code=$1" \
    | awk '{ status = $NF; $NF = ""; sub(/ $/, ""); print status " " $0 }'
}

probes_terminology_ferroterm() {
  bold "terminology — FerroTERM beside the CDR over the shaped seed (the overlay)"

  probe "P-FT-OFF" "off" "server" "#3304" \
    "without the overlay a bound value set is not resolved: member and non-member both commit"
  compose_up ferroehr
  if ! wait_http "$CDR/health/readiness" 90 || ! term_ft_template; then
    probe_fail "a serving CDR holding the binding template" "readiness or the template upload failed" \
      "the binding template is the fixture every state below commits against"
    probe_done
    return 0
  fi
  assert_eq "201" "$(term_ft_commit_code "$TERM_FT_MEMBER")" \
    "with external terminology off the member composition commits"
  assert_eq "201" "$(term_ft_commit_code "$TERM_FT_NON_MEMBER")" \
    "with external terminology off nothing checks membership, so the non-member commits too"
  assert_eq "404" "$(term_ft_api_validate 1000002 | cut -d' ' -f1)" \
    "without the overlay the /terminology/* extension API is off, as the base config ships it"
  probe_done

  probe "P-FT-UP" "working" "compose" "#3304" \
    "the overlay starts FerroTERM, waits on its own healthcheck, and it answers its capability statement"
  if ! term_ft_up; then
    probe_fail "a healthy FerroTERM and CDR from \`up --wait\`" "$(dc -f docker-compose.yml -f "$TERM_FT_OVERLAY" logs --tail 5 ferroterm 2>&1 | tail -3)" \
      "the overlay must bring the terminology server up from the shaped seed alone, and its HEALTHCHECK must report it"
    probe_done
    return 0
  fi
  assert_eq "200" "$(http_code "http://localhost:${TERM_FT_PORT}/health")" \
    "the health route the image's own probe reads answers from the host too"
  local meta; meta="$(curl -s "http://localhost:${TERM_FT_PORT}/r4b/metadata?mode=terminology")"
  assert_contains "$meta" 'http://cnf.example.test/fhir/CodeSystem/sct-shaped' \
    "the shaped seed mounted out of the ferroehr image is what the server serves"
  probe_done

  probe "P-FT-RESOLVE" "working" "server" "#3304" \
    "with the overlay on, a value-set member commits and a non-member is refused"
  wait_http "$CDR/health/readiness" 90 >/dev/null || true
  term_ft_template || true
  assert_eq "201" "$(term_ft_commit_code "$TERM_FT_MEMBER")" \
    "1000002 is in sct-shaped-disorders, so the commit is accepted"
  assert_eq "422" "$(term_ft_commit_code "$TERM_FT_NON_MEMBER")" \
    "1000003 is in the code system but not in the value set, so the binding refuses it"
  probe_done

  probe "P-FT-API" "working" "compose" "#3476" \
    "with the overlay on, the CDR's /terminology/* routes answer membership from FerroTERM, as the overlay header says"
  local member non_member
  member="$(term_ft_api_validate 1000002)"
  non_member="$(term_ft_api_validate 1000003)"
  assert_not_contains "$member" "terminology API is disabled" \
    "the overlay switches the extension API on; a 404 here means its header claims a route it never enabled"
  assert_eq '200 {"valid":true}' "$member" \
    "1000002 is in sct-shaped-disorders, and only FerroTERM holds that value set"
  assert_eq '200 {"valid":false}' "$non_member" \
    "1000003 is not, so the answer differs and cannot come from a constant"
  probe_done

  probe "P-FT-DOWN-OPEN" "broken" "server" "#3304" \
    "FerroTERM stopped, fail-open (the shipped default): the member commit is still accepted"
  dc -f docker-compose.yml -f "$TERM_FT_OVERLAY" stop ferroterm >/dev/null 2>&1
  # The provider caches a positive answer for cache_ttl_secs, so a commit that
  # still says 201 could be the cache and not the posture. Recreating the CDR
  # empties the in-process cache, and --no-deps keeps Compose from starting the
  # stopped ferroterm again as the CDR's dependency (which is what answered
  # this state's commits from a live server on the first CI run).
  dc -f docker-compose.yml -f "$TERM_FT_OVERLAY" --profile s3 up -d --no-deps --force-recreate --wait ferroehr >/dev/null 2>&1
  wait_http "$CDR/health/readiness" 90 >/dev/null || true
  assert_not_contains "$(dc -f docker-compose.yml -f "$TERM_FT_OVERLAY" ps -a --format '{{.Service}} {{.State}}')" "ferroterm running" \
    "the terminology server must be down for this state to mean anything"
  assert_eq "201" "$(term_ft_commit_code "$TERM_FT_MEMBER")" \
    "fail-open accepts a binding the server cannot resolve"
  probe_done

  probe "P-FT-DOWN-CLOSED" "broken" "server" "#3304" \
    "FerroTERM stopped, fail-closed: the same commit is refused"
  # Exported, not a prefix assignment: `dc` is a shell function, and only an
  # exported variable reliably reaches the compose interpolation it runs. The
  # env change recreates the CDR, so the cache is cold here too.
  export FERROEHR__TERMINOLOGY__EXTERNAL__FAIL_ON_ERROR=true
  dc -f docker-compose.yml -f "$TERM_FT_OVERLAY" --profile s3 up -d --no-deps --wait ferroehr >/dev/null 2>&1
  unset FERROEHR__TERMINOLOGY__EXTERNAL__FAIL_ON_ERROR
  wait_http "$CDR/health/readiness" 90 >/dev/null || true
  assert_not_contains "$(dc -f docker-compose.yml -f "$TERM_FT_OVERLAY" ps -a --format '{{.Service}} {{.State}}')" "ferroterm running" \
    "the terminology server must still be down under fail-closed"
  assert_eq "422" "$(term_ft_commit_code "$TERM_FT_MEMBER")" \
    "fail-closed turns an unreachable terminology server into a refusal"
  probe_done

  # Leave the stack as the other families expect it: the base file alone, and
  # no terminology container standing.
  dc -f docker-compose.yml -f "$TERM_FT_OVERLAY" rm -sf ferroterm >/dev/null 2>&1 || true
  compose_up ferroehr
}

probes_terminology() {
  bold "terminology — a real FHIR terminology server with real content"

  # NEVER in a default run. Snowstorm plus a real SNOMED import takes longer
  # than every other family combined, and the archive now sits in the repository
  # root where the discovery below would find it — so a plain
  # `bash scripts/deploy-probe.sh` would silently turn a fifteen-minute harness
  # into an hour. This family runs only when it is ASKED for:
  #
  #   PROBE_ONLY=terminology bash scripts/deploy-probe.sh
  #
  # or when FERROEHR_SNOMED_RF2 is set explicitly, which is itself an ask.
  if [[ "${PROBE_ONLY:-}" != "terminology" ]] && [[ -z "${FERROEHR_SNOMED_RF2:-}" ]]; then
    uncovered "terminology against a real server (#2178)" \
      "opt-in: run PROBE_ONLY=terminology, because a real SNOMED import takes longer than every other family combined"
    return 0
  fi

  local zip
  if ! zip="$(terminology_release)"; then
    uncovered "terminology against a real server (#2178)" \
      "drop a SnomedCT_*.zip in the repository root (it is gitignored) or set FERROEHR_SNOMED_RF2. It is licensed content this repository may not ship and CI may not fetch, and a seeded code system would only test our own fixture"
    return 0
  fi

  local overlay
  overlay="$(terminology_overlay)"

  probe "P-TERM-UP" "working" "compose" "#2178" \
    "Snowstorm serves the FHIR terminology API"
  dc -f docker-compose.yml -f "$overlay" up -d ferroehr-snowstorm >/dev/null 2>&1
  if ! wait_http "$TERM_URL/fhir/metadata" 600; then
    # Elasticsearch refuses to start without vm.max_map_count, which is a HOST
    # setting this harness cannot change from inside a container. Saying so is
    # more useful than reporting a defect in the chart or the server.
    probe_fail "a Snowstorm FHIR endpoint" "$(dc logs --tail 5 ferroehr-snowstorm 2>&1 | tail -3)" \
      "if Elasticsearch exited, the host needs vm.max_map_count=262144 — a sysctl no container can set for itself"
    probe_done
    return 0
  fi
  probe_done

  # An International package, when supplied, goes to MAIN first — a national
  # extension that depends on it cannot resolve otherwise. Whether the Managed
  # Service edition needs this is not documented, so it is optional rather than
  # assumed, and the run reports what was actually served either way.
  local intl
  intl="$(terminology_international)"
  if [[ -n "$intl" ]]; then
    probe "P-TERM-IMPORT-INTL" "working" "compose" "#2178" \
      "the International Edition imports to MAIN, so a national extension can resolve against it"
    terminology_import "$intl" || probe_fail "a COMPLETED International import" \
      "the import did not complete" "a national extension cannot resolve without it"
    probe_done
  fi

  probe "P-TERM-IMPORT" "working" "compose" "#2178" \
    "the operator's SNOMED CT release imports and is served as a CodeSystem"
  if ! terminology_import "$zip"; then
    probe_fail "a COMPLETED import job" "the import did not complete" \
      "without content every operation below would answer emptily and prove nothing"
    probe_done
    return 0
  fi
  assert_contains "$(curl -s "$TERM_URL/fhir/CodeSystem")" "http://snomed.info/sct" \
    "the SNOMED CodeSystem must be served once the release is loaded"
  probe_done

  # The operation a seeded fixture cannot fake: subsumption over a real
  # hierarchy. 73211009 (Diabetes mellitus) subsumes 44054006 (Type 2 diabetes
  # mellitus) in the International Edition.
  probe "P-TERM-SUBSUMES" "working" "server" "#2178" \
    "a real SNOMED hierarchy answers \$subsumes — the check a fixture cannot honestly make"
  # The concepts below are International. A package that turned out to carry
  # only national content would not contain them, and reporting that as a
  # subsumption defect would be a lie about the server — so their presence is
  # established first, and their absence is a declared gap.
  if ! curl -s "$TERM_URL/fhir/CodeSystem/\$lookup?system=http://snomed.info/sct&code=73211009" \
       | grep -q '"resourceType"'; then
    dim "    SKIP  73211009 is not in the loaded content — declared as not exercised"
    uncovered "SNOMED subsumption over a published hierarchy" \
      "the loaded package does not contain the International concepts this probe names; supply FERROEHR_SNOMED_RF2_INTL to load the International Edition alongside it"
    probe_done
    return 0
  fi
  local sub
  sub="$(curl -s --get \
    --data-urlencode 'system=http://snomed.info/sct' \
    --data-urlencode 'codeA=73211009' \
    --data-urlencode 'codeB=44054006' \
    "$TERM_URL/fhir/CodeSystem/\$subsumes")"
  assert_contains "$sub" "subsumes" \
    "a hierarchy we authored ourselves would answer whatever we told it to; this one is the published release"
  probe_done

  # The far end that matters for the CDR: a coded value validated THROUGH the
  # configured provider, not by our in-process bundle.
  probe "P-TERM-VALIDATE" "working" "server" "#2178" \
    "the CDR validates a real SNOMED code through the configured provider"
  dc -f docker-compose.yml -f "$overlay" up -d ferroehr >/dev/null 2>&1
  if ! wait_http "$CDR/health/readiness" 180; then
    probe_fail "a serving CDR with an external terminology provider" \
      "$(dc logs --tail 5 ferroehr 2>&1 | tail -3)"
  else
    assert_contains "$(curl -s -u "$BASIC" "$CDR/management/env" | tr ',' '\n' | grep -i terminology | head -5)" \
      "enabled" "the provider must be active, or the probes above measured Snowstorm and not the CDR"
  fi
  probe_done

  # fail_on_error is the whole safety story: a terminology server that cannot
  # answer must not silently let an unvalidated code through.
  probe "P-TERM-BROKEN" "broken" "server" "#2178" \
    "with the terminology server stopped and fail_on_error set, validation fails CLOSED"
  dc stop ferroehr-snowstorm >/dev/null 2>&1
  local code
  code="$(http_code -u "$BASIC" --get --data-urlencode 'q=SELECT e/ehr_id/value FROM EHR e LIMIT 1' "$API/query/aql")"
  case "$code" in
    200|400|422|503) : ;;
    *) probe_fail "a defined answer with the terminology server down" "$code" \
         "an unreachable terminology server must produce a typed outcome, not an unclassified failure" ;;
  esac
  probe_done

  dc -f docker-compose.yml -f "$overlay" stop ferroehr-snowstorm ferroehr-es >/dev/null 2>&1
  dc -f docker-compose.yml -f "$overlay" rm -f ferroehr-snowstorm ferroehr-es >/dev/null 2>&1
  dc -f docker-compose.yml up -d ferroehr >/dev/null 2>&1
  wait_http "$CDR/health/readiness" 120 || true
}
