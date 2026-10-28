#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The terminology probe families (#2178, #2158, #3304, #2236), against
# FerroTERM, the terminology server FerroEHR ships with.
#
# Two families live here. `probes_terminology_ferroterm` runs in every default
# pass over the licence-free shaped seed: the overlay off, on, and the server
# down under each fail posture. `probes_terminology` drives the same overlay
# over a REAL SNOMED CT release, because a seeded code system of our own only
# proves our client can talk to our own fixture: `$subsumes` in particular
# means nothing without a published hierarchy behind it.
#
# THE RELEASE IS NOT OURS TO SHIP, AND THE REAL-RELEASE FAMILY RUNS LOCALLY
# ONLY. The RF2 package is licensed content under a SNOMED International
# Affiliate agreement: it cannot be committed here, fetched by CI, or baked into
# a published image. The operator supplies a local path:
#
#   FERROEHR_SNOMED_RF2=~/SnomedCT_ManagedServiceNL_PRODUCTION_NL1000146_*.zip
#
# Without it the family declares itself NOT EXERCISED rather than substituting a
# fixture — an honest gap beats a green row that measured our own data.
#
# The release becomes an index through the overlay's own one-shot
# `ferroterm-build` service (docker-compose.terminology.yml, profile
# `terminology-build`): the archive is mounted read-only, unpacked to a tmpfs,
# and the index lands in the named volume FerroTERM then opens with
# FERROTERM_INDEX=/data/index. That is the path an operator takes, so the family
# measures the shipped arrangement and nothing built for the test.
#
# The NETHERLANDS edition is the expected package. `ferroterm-build` takes one
# edition per build (`--rf2`), and the run reports what it served: the
# subsumption probe looks its concepts up first and declares a gap, never a
# defect, when they are absent.
#
# Sourced by scripts/deploy-probe.sh; never run directly.

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
  local fixture="$1" ehr; ehr="$(term_ft_new_ehr)"
  [[ -n "$ehr" ]] || { printf '000'; return 0; }
  curl -s -u "$BASIC" -o /dev/null -w '%{http_code}' -X POST \
    -H 'Content-Type: application/json' -H 'Prefer: return=minimal' \
    --data-binary "@$fixture" "$API/ehr/$ehr/composition"
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
  local code="$1" system value_set
  system="$(jq -rn --arg s "$TERM_FT_SYSTEM" '$s | @uri')"
  value_set="$(jq -rn --arg s "$TERM_FT_VALUE_SET" '$s | @uri')"
  curl -s -u "$BASIC" -w ' %{http_code}' \
    "$EXT/terminology/${system}/value_set/${value_set}/validate?candidate_code=$code" \
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
  # --no-deps keeps Compose from starting the stopped ferroterm again as the
  # CDR's dependency (which is what answered this state's commits from a live
  # server on the first CI run).
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
  # exported variable reliably reaches the compose interpolation it runs.
  export FERROEHR__TERMINOLOGY__SERVER__FAIL_ON_ERROR=true
  dc -f docker-compose.yml -f "$TERM_FT_OVERLAY" --profile s3 up -d --no-deps --wait ferroehr >/dev/null 2>&1
  unset FERROEHR__TERMINOLOGY__SERVER__FAIL_ON_ERROR
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

# Build the index of the release at $1 through the overlay's one-shot
# `ferroterm-build`. The archive path must be absolute: it is a bind source.
terminology_build() {
  local zip
  zip="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
  FERROTERM_RF2="$zip" dc -f docker-compose.yml -f "$TERM_FT_OVERLAY" \
    --profile terminology-build run --rm ferroterm-build >"$PROBE_TMP/ferroterm-build.log" 2>&1
}

probes_terminology() {
  bold "terminology — FerroTERM over a real SNOMED CT release"

  # NEVER in a default run. A real release is licensed content that lives only
  # on the operator's machine, and building its index is the one step here that
  # takes minutes rather than seconds. This family runs only when it is ASKED
  # for:
  #
  #   PROBE_ONLY=terminology bash scripts/deploy-probe.sh
  #
  # or when FERROEHR_SNOMED_RF2 is set explicitly, which is itself an ask.
  if [[ "${PROBE_ONLY:-}" != "terminology" ]] && [[ -z "${FERROEHR_SNOMED_RF2:-}" ]]; then
    uncovered "terminology against a real release (#2236)" \
      "opt-in: run PROBE_ONLY=terminology, because it needs a licensed SNOMED CT release on this machine"
    return 0
  fi

  local zip
  if ! zip="$(terminology_release)"; then
    uncovered "terminology against a real release (#2236)" \
      "drop a SnomedCT_*.zip in the repository root (it is gitignored) or set FERROEHR_SNOMED_RF2. It is licensed content this repository may not ship and CI may not fetch, and a seeded code system would only test our own fixture"
    return 0
  fi
  dim "    release: $zip"

  probe "P-TERM-UP" "working" "compose" "#2236" \
    "the overlay's digest-pinned FerroTERM starts and answers its capability statement"
  if ! term_ft_up; then
    probe_fail "a healthy FerroTERM from \`up --wait\`" "$(dc -f docker-compose.yml -f "$TERM_FT_OVERLAY" logs --tail 5 ferroterm 2>&1 | tail -3)" \
      "the overlay must bring the terminology server up before any release is built"
    probe_done
    return 0
  fi
  assert_eq "200" "$(http_code "http://localhost:${TERM_FT_PORT}/r4b/metadata")" \
    "the R4B capability statement the CDR's client reads"
  probe_done

  probe "P-TERM-IMPORT" "working" "compose" "#2236" \
    "ferroterm-build turns the mounted release into an index and the server opens it"
  local started elapsed
  started="$(date +%s)"
  if ! terminology_build "$zip"; then
    probe_fail "a completed ferroterm-build" "$(tail -3 "$PROBE_TMP/ferroterm-build.log")" \
      "without an index every operation below would answer from the shaped seed alone and prove nothing"
    probe_done
    return 0
  fi
  elapsed=$(( $(date +%s) - started ))
  dim "    ferroterm-build: ${elapsed} s — $(tail -1 "$PROBE_TMP/ferroterm-build.log")"
  # Exported, not a prefix assignment: `dc` is a shell function, and only an
  # exported variable reliably reaches the compose interpolation it runs.
  export FERROTERM_INDEX=/data/index
  dc -f docker-compose.yml -f "$TERM_FT_OVERLAY" --profile s3 up -d --force-recreate --wait ferroterm >/dev/null 2>&1
  unset FERROTERM_INDEX
  assert_contains "$(curl -s "http://localhost:${TERM_FT_PORT}/r4b/metadata?mode=terminology")" "http://snomed.info/sct" \
    "the SNOMED CT code system must be served once the index is open"
  probe_done

  # The operation a seeded fixture cannot fake: subsumption over a real
  # hierarchy. 73211009 (Diabetes mellitus) subsumes 44054006 (Type 2 diabetes
  # mellitus) in the International Edition.
  probe "P-TERM-SUBSUMES" "working" "server" "#2236" \
    "a real SNOMED hierarchy answers \$subsumes — the check a fixture cannot honestly make"
  # The concepts below are International. A package that carried only national
  # content would not contain them, and reporting that as a subsumption defect
  # would be a lie about the server — so their presence is established first,
  # and their absence is a declared gap.
  if ! grep -q '"Parameters"' <<<"$(curl -s --get --data-urlencode 'system=http://snomed.info/sct' --data-urlencode 'code=73211009' \
       "http://localhost:${TERM_FT_PORT}/r4b/CodeSystem/\$lookup")"; then
    dim "    SKIP  73211009 is not in the loaded content — declared as not exercised"
    uncovered "SNOMED subsumption over a published hierarchy" \
      "the loaded package does not contain the International concepts this probe names"
    probe_done
  else
    local sub
    sub="$(curl -s --get \
      --data-urlencode 'system=http://snomed.info/sct' \
      --data-urlencode 'codeA=73211009' \
      --data-urlencode 'codeB=44054006' \
      "http://localhost:${TERM_FT_PORT}/r4b/CodeSystem/\$subsumes")"
    assert_contains "$sub" '"valueCode":"subsumes"' \
      "a hierarchy we authored ourselves would answer whatever we told it to; this one is the published release"
    probe_done
  fi

  # The far end that matters for the CDR: a code answered THROUGH the CDR's
  # terminology server, not by its in-process bundle, which carries no SNOMED.
  probe "P-TERM-VALIDATE" "working" "server" "#2236" \
    "the CDR answers a real SNOMED code through its terminology server"
  wait_http "$CDR/health/readiness" 90 >/dev/null || true
  local system
  system="$(jq -rn '"http://snomed.info/sct" | @uri')"
  assert_eq "200" "$(http_code -u "$BASIC" "$EXT/terminology/${system}/term/73211009")" \
    "the /terminology/* route reaches FerroTERM for a terminology the bundle does not carry"
  probe_done

  # fail_on_error is the whole safety story: a terminology server that cannot
  # answer must not silently let an unvalidated code through. The shaped seed is
  # still mounted beside the index, so the binding template resolves while the
  # server is up and the refusal below is the posture, not a missing value set.
  probe "P-TERM-BROKEN" "broken" "server" "#2236" \
    "with the terminology server stopped and fail_on_error set, a bound commit fails CLOSED"
  term_ft_template || true
  dc -f docker-compose.yml -f "$TERM_FT_OVERLAY" stop ferroterm >/dev/null 2>&1
  export FERROEHR__TERMINOLOGY__SERVER__FAIL_ON_ERROR=true
  dc -f docker-compose.yml -f "$TERM_FT_OVERLAY" --profile s3 up -d --no-deps --wait ferroehr >/dev/null 2>&1
  unset FERROEHR__TERMINOLOGY__SERVER__FAIL_ON_ERROR
  wait_http "$CDR/health/readiness" 90 >/dev/null || true
  assert_not_contains "$(dc -f docker-compose.yml -f "$TERM_FT_OVERLAY" ps -a --format '{{.Service}} {{.State}}')" "ferroterm running" \
    "the terminology server must be down for this state to mean anything"
  assert_eq "422" "$(term_ft_commit_code "$TERM_FT_MEMBER")" \
    "fail-closed turns an unreachable terminology server into a refusal"
  probe_done

  # Leave the stack as the other families expect it. The index stays in the
  # project's named volume on this machine, so a second run rebuilds over it.
  dc -f docker-compose.yml -f "$TERM_FT_OVERLAY" rm -sf ferroterm >/dev/null 2>&1 || true
  compose_up ferroehr
}
