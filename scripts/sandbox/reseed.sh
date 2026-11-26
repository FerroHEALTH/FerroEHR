#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# The one seed of FerroEHR's synthetic demo dataset (#2710, #3045, #6663): the
# hosted sandbox every night and the browser journeys' stack
# (scripts/ui-e2e.sh). It drives the PUBLIC API only, the same surface
# visitors use, so the seed itself proves the deployment and every write
# leaves the access record a client's does. This file is a WALKER: what gets
# loaded is scripts/sandbox/seed/manifest.json plus the bodies beside it, so
# adding content is editing data. The manifest's `notes` describe the dataset
# and the tokens substituted below.
#
# Every request retries: right after the nightly wipe, instances booted BEFORE
# the wipe can keep serving until they recycle, and a request landing on one
# answers 5xx (observed live, #2710). Retrying for a few minutes rides that
# window out. Anything else is fatal: a wrong status prints ::error:: and the
# run exits non-zero.
#
# Re-runs: the definition surfaces are 409-tolerant and the EHRs resolve by
# subject, so the seed is safe to repeat. A marker EHR written last records
# completion, and a second run sees it and exits without touching anything.
#
# Environment: SANDBOX_BASE (default https://sandbox.ferroehr.eu),
# SANDBOX_USER / SANDBOX_PASS (the administrator that seeds, default the
# public demo credentials), SANDBOX_CLINICIAN (default ferroehr-clinician, a
# professional who reads outside a care relationship and attests),
# SANDBOX_PROXY (default ferroehr-proxy, whose PERSON the seed writes as a
# patient's representative), each with its `_PASS` (default ferroehr), an
# empty SANDBOX_CLINICIAN skipping the clinician's steps, SANDBOX_IDS_OUT (a
# file the seeded EHR ids and tokens are written to), and SANDBOX_IDENTITIES:
# the file the box's `identities` verb answered (deploy/hosted/ferropix-seed.sh). Under an IHE patient identity source it maps
# each manifest subject to the pseudonym FerroPIX minted for it, and every
# EHR_STATUS names that pseudonym in the clinical domain (#4242). Unset, or
# answering `provider local` (a CDR beside no FerroPIX), the subjects name
# themselves.
# SANDBOX_TERMINOLOGY=without-loinc: a run against a CDR whose terminology
# server serves no LOINC, which moves the archetype-library pin (step 3).
#
# `--render-ehrs <dir>` sends nothing: it writes every EHR_STATUS body the seed
# would send into <dir>, one file per subject, so a check can hold them to the
# sandbox's privacy.subject_namespaces (scripts/checks/sandbox-seed-subjects.sh).
set -Eeuo pipefail

RENDER_DIR=""
case "${1:-}" in
  "") ;;
  --render-ehrs)
    RENDER_DIR="${2:-}"
    [[ -n "$RENDER_DIR" && -d "$RENDER_DIR" ]] || {
      echo "::error::--render-ehrs takes an existing directory" >&2
      exit 2
    }
    ;;
  *)
    echo "::error::unknown argument '$1'; the one flag is --render-ehrs <dir>" >&2
    exit 2
    ;;
esac

BASE="${SANDBOX_BASE:-https://sandbox.ferroehr.eu}/ferroehr/rest/openehr/v1"
# FerroEHR's own extensions sit beside the openEHR API (#4330).
EXT="${SANDBOX_BASE:-https://sandbox.ferroehr.eu}/ferroehr/rest/extensions/v1"
SEED_LOGIN="${SANDBOX_USER:-ferroehr}"
AUTH="$SEED_LOGIN:${SANDBOX_PASS:-ferroehr}"
# The professional who reads outside a care relationship and attests, and the
# representative who reads the subject view of a patient's access log: two
# more logins of the deployment.
CLINICIAN="${SANDBOX_CLINICIAN-ferroehr-clinician}"
CLINICIAN_PASS="${SANDBOX_CLINICIAN_PASS:-ferroehr}"
PROXY="${SANDBOX_PROXY:-ferroehr-proxy}"
PROXY_PASS="${SANDBOX_PROXY_PASS:-ferroehr}"
IDS_OUT="${SANDBOX_IDS_OUT:-}"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SEED_DIR="$ROOT_DIR/scripts/sandbox/seed"
MANIFEST="$SEED_DIR/manifest.json"
TPL_DIR="$ROOT_DIR/corpus/templates/ckm"
ATTEMPTS=12
RETRY_DELAY=15

command -v jq > /dev/null || {
  echo "::error::jq is required to walk $MANIFEST" >&2
  exit 1
}
[[ -f "$MANIFEST" ]] || {
  echo "::error::missing manifest $MANIFEST" >&2
  exit 1
}

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
HDR="$WORK/headers"
BODY="$WORK/body"
CT_JSON='Content-Type: application/json'
CT_PLAIN='Content-Type: text/plain'

# ── the manifest ─────────────────────────────────────────────────────────────

# Read the manifest with a jq filter, raw output.
mf() { local filter="$1"; jq -r "$filter" "$MANIFEST"; }

# Every EHR entry of the manifest, one compact JSON object per line.
manifest_ehrs() { jq -c '.ehrs[]' "$MANIFEST"; }

SUBJECT_NS="$(mf '.subject_namespace')"

# The identity source the deployment resolves subjects through: `local` (the
# subject names itself) or `ihe` (the pseudonym FerroPIX minted names it).
IDENTITIES="${SANDBOX_IDENTITIES:-}"
PROVIDER=local
if [[ -n "$IDENTITIES" ]]; then
  [[ -s "$IDENTITIES" ]] || {
    echo "::error::SANDBOX_IDENTITIES names $IDENTITIES, which is missing or empty" >&2
    exit 1
  }
  PROVIDER="$(awk '$1 == "provider" { print $2 }' "$IDENTITIES")"
  case "$PROVIDER" in
    local) ;;
    ihe)
      SUBJECT_NS="$(awk '$1 == "pseudonym_namespace" { print $2 }' "$IDENTITIES")"
      [[ -n "$SUBJECT_NS" ]] || {
        echo "::error::$IDENTITIES names no pseudonym_namespace" >&2
        exit 1
      }
      ;;
    *)
      echo "::error::$IDENTITIES names provider '$PROVIDER'; want local or ihe" >&2
      exit 1
      ;;
  esac
fi

# The identifier EHR_STATUS carries for a manifest subject: the subject itself,
# or under `ihe` its pseudonym, which must be in the identities file.
subject_id() {
  local subject="$1" id
  if [[ "$PROVIDER" == local ]]; then
    printf '%s' "$subject"
    return
  fi
  id="$(awk -v s="$subject" '$1 == "subject" && $2 == s { print $3 }' "$IDENTITIES")"
  [[ -n "$id" ]] || {
    echo "::error::$IDENTITIES carries no pseudonym for $subject" >&2
    exit 1
  }
  printf '%s' "$id"
}

# ── the wire ─────────────────────────────────────────────────────────────────

# One request, retried through stale instances, as the seeding login or as
# the login `REQ_AUTH` names. `$1` = a label, `$2` = the
# method, `$3` = the URL; the rest go to curl verbatim. Prints the final status
# code; the response headers land in $HDR and the body in $BODY.
request() {
  local label="$1" method="$2" url="$3"
  shift 3
  local attempt code
  for attempt in $(seq 1 "$ATTEMPTS"); do
    # curl prints 000 itself when it reaches no server; appending another would make 000000 and skip the retry.
    code=$(curl -sS -m 300 -u "${REQ_AUTH:-$AUTH}" -D "$HDR" -o "$BODY" -w '%{http_code}' \
      -X "$method" "$url" "$@") || code=000
    case "$code" in
      5?? | 000)
        echo "  $label answered $code (attempt $attempt/$ATTEMPTS); a stale instance or a dropped connection, retrying" >&2
        sleep "$RETRY_DELAY"
        ;;
      *)
        printf '%s' "$code"
        return 0
        ;;
    esac
  done
  printf '%s' "$code"
}

# Fail loud unless `$2` is one of the accepted codes in `$3…`.
expect() {
  local label="$1" code="$2" want
  shift 2
  for want in "$@"; do
    [[ "$code" == "$want" ]] && return 0
  done
  echo "::error::$label answered $code (accepted: $*)" >&2
  head -c 600 "$BODY" >&2
  echo >&2
  exit 1
}

# The last value of response header `$1`, case-insensitively.
header_value() {
  local name="$1"
  tr -d '\r' < "$HDR" | awk -v want="$name" '
    BEGIN { want = tolower(want) ":" }
    tolower($1) == want { sub(/^[^:]*:[ ]*/, ""); print }
  ' | tail -1
}

# The identifier the weak ETag carries (`W/"<uid>"` → `<uid>`).
etag_uid() { header_value etag | sed -e 's|^W/||' -e 's|"||g'; }

# The tokens a world body names, each with its value: `__ID:<key>__` for a
# party or relationship the seed wrote, `__SUBJECT_ID:<n>__` for the n-th
# EHR's subject, `__LOGIN:<who>__` for a login.
TOKENS="$WORK/tokens.json"
echo '{}' > "$TOKENS"
token() {
  jq --arg k "$1" --arg v "$2" '.[$k] = $v' "$TOKENS" > "$TOKENS.next"
  mv "$TOKENS.next" "$TOKENS"
}
# Midnight UTC of the day the seed runs, from which `__DATE:<days>__` counts.
TODAY=$(( $(date -u +%s) / 86400 * 86400 ))
token "__SUBJECT_NS__" "$SUBJECT_NS"

# ── bodies ───────────────────────────────────────────────────────────────────

# Render `$1` into `$2`, replacing each remaining `TOKEN=VALUE` pair wherever
# TOKEN appears as a whole JSON string. The walker never writes JSON itself;
# every body is a file under scripts/sandbox/seed/.
render() {
  local src="$1" dst="$2" pair token value
  shift 2
  cp "$src" "$dst"
  for pair in "$@"; do
    token="${pair%%=*}"
    value="${pair#*=}"
    jq --arg t "$token" --arg v "$value" \
      'walk(if . == $t then $v else . end)' "$dst" > "$dst.next"
    mv "$dst.next" "$dst"
  done
}

# The offline mode: the bodies of steps 5, 6 and 11, and nothing sent.
if [[ -n "$RENDER_DIR" ]]; then
  while read -r entry; do
    subject=$(printf '%s' "$entry" | jq -r '.subject')
    subject_ref="$(subject_id "$subject")"
    render "$SEED_DIR/$(printf '%s' "$entry" | jq -r '.status')" "$RENDER_DIR/$subject.json" \
      "__SUBJECT_ID__=$subject_ref" "__SUBJECT_NS__=$SUBJECT_NS"
    final=$(printf '%s' "$entry" | jq -r '.final_status // empty')
    if [[ -n "$final" ]]; then
      render "$SEED_DIR/$final" "$RENDER_DIR/$subject.final.json" \
        "__SUBJECT_ID__=$subject_ref" "__SUBJECT_NS__=$SUBJECT_NS"
    fi
  done < <(manifest_ehrs)
  marker_ref="$(subject_id "$(mf '.marker.subject')")"
  render "$SEED_DIR/$(mf '.marker.status')" "$RENDER_DIR/marker.json" \
    "__SUBJECT_ID__=$marker_ref" "__SUBJECT_NS__=$SUBJECT_NS"
  exit 0
fi

# ── 0. the demo PERSON ───────────────────────────────────────────────────────

# The Basic user is a person bound by its key (#5009): every request is
# refused `refused-unbound` until a PERSON carries it, and refused for its
# scope until that PERSON holds them (#5438). The box's ferroehr-provision
# service creates that PERSON through /scim/v2/ as the `sandbox-scim` client
# after the wipe, and its ferroehr-hold service then grants it the scopes
# (deploy/hosted/docker-compose.yml), so the seed waits for the login to bind
# and hold them before it commits anything.
echo "==> seeding $BASE"
bound=""
for attempt in $(seq 1 "$ATTEMPTS"); do
  code=$(request "binding probe" GET "$BASE/definition/template/adl1.4")
  if [[ "$code" == 200 ]]; then
    bound=1
    break
  fi
  if [[ "$code" == 403 ]] && grep -q 'refused-unbound' "$BODY"; then
    echo "  the demo login binds no PERSON yet (attempt $attempt/$ATTEMPTS); waiting for its SCIM provisioning" >&2
    sleep "$RETRY_DELAY"
    continue
  fi
  if [[ "$code" == 403 ]]; then
    echo "  the demo PERSON holds no scopes yet (attempt $attempt/$ATTEMPTS); waiting for its grant" >&2
    sleep "$RETRY_DELAY"
    continue
  fi
  expect "binding probe" "$code" 200
done
[[ -n "$bound" ]] || {
  echo "::error::the demo login never bound a PERSON holding its scopes: read the box's ferroehr-provision and ferroehr-hold logs" >&2
  exit 1
}

# ── 1. completion marker ─────────────────────────────────────────────────────

MARKER_SUBJECT="$(mf '.marker.subject')"
MARKER_ID="$(subject_id "$MARKER_SUBJECT")"
code=$(request "marker lookup" GET \
  "$BASE/ehr?subject_id=$MARKER_ID&subject_namespace=$SUBJECT_NS")
case "$code" in
  200)
    echo "==> already seeded (marker EHR $MARKER_SUBJECT present); nothing to do"
    exit 0
    ;;
  404) ;;
  *) expect "marker lookup" "$code" 200 404 ;;
esac

# ── 2. ADL 1.4 operational templates ─────────────────────────────────────────

adl14_count=0
while read -r entry; do
  slug=$(printf '%s' "$entry" | jq -r '.slug')
  # An entry names its own OPT (a repository path) when it is not a CKM
  # template; the CKM pack is the default.
  opt=$(printf '%s' "$entry" | jq -r --arg d "$TPL_DIR/$slug.opt" '.file // $d')
  [[ "$opt" = /* ]] || opt="$ROOT_DIR/$opt"
  [[ -f "$opt" ]] || {
    echo "::error::missing operational template $opt" >&2
    exit 1
  }
  code=$(request "template $slug" POST "$BASE/definition/template/adl1.4" \
    -H 'Content-Type: application/xml' --data-binary @"$opt")
  expect "template $slug" "$code" 201 204 409
  adl14_count=$((adl14_count + 1))
done < <(jq -c '.adl14_templates[]' "$MANIFEST")
echo "==> $adl14_count ADL 1.4 operational templates"

# ── 3. the ADL 2 archetype library ───────────────────────────────────────────
#
# The whole vendored corpus is offered, parents before children, and both
# outcomes are pinned in the manifest: an artefact the CDR stores, and one its
# AOM2 validation refuses (the 2013 conversions constrain attributes the RM
# does not declare, restate or renumber parent slots, or specialise a refused
# parent). The per-file adjudication is the corpus gate in
# crates/openehr-adl/tests/it/ckm_archetype_packs.rs; a change in either count
# fails the run rather than passing quietly.

# Parents before children: a specialised archetype validates against its flat
# parent, and the CDR refuses one whose parent is not stored yet (VASID). The
# specialisation depth is the number of `-` in the id's concept segment
# (`CLUSTER.exam-abdomen` specialises `CLUSTER.exam`), so the list is ordered
# by that depth first and by name second; plain `LC_ALL=C sort` put `exam-…`
# before `exam.` because `-` sorts before `.`.
adls_parents_first() {
  local dir="$1"
  find "$dir" -name '*.adls' | while read -r f; do
    concept="$(basename "$f" | cut -d. -f2)"
    depth="${concept//[^-]/}"
    printf '%d\t%s\n' "${#depth}" "$f"
  done | LC_ALL=C sort -k1,1n -k2,2 | cut -f2-
}

lib_root="$ROOT_DIR/$(mf '.adl2.archetype_library.root')"
want_accepted="$(mf '.adl2.archetype_library.accepted')"
want_refused="$(mf '.adl2.archetype_library.refused')"
# The upload declares only 201, 400 and 409 (ITS-REST 1.1.0
# operations/definition_template_adl2_upload.yaml), so the CDR refuses an
# AOM2-invalid archetype with a 400 (#5044); the manifest pins that status.
refused_status="$(mf '.adl2.archetype_library.refused_status')"
[[ "$refused_status" =~ ^[0-9]{3}$ ]] || {
  echo "::error::the manifest's adl2.archetype_library.refused_status is '$refused_status'; want the HTTP status the upload refuses an invalid archetype with" >&2
  exit 1
}

# The pin assumes a CDR whose terminology server serves LOINC: VETDF refuses
# the files `loinc.refused_when_served` names only when it can look their
# bindings up, and only warns when it cannot (AM ADL2 master03 VETDF, "subject
# to tool accessibility"). Ask the CDR for a LOINC code first, so a posture
# that serves no LOINC fails here by name rather than as a count mismatch
# below (#4278). A local run without LOINC opts out with
# SANDBOX_TERMINOLOGY=without-loinc and expects those files stored.
loinc_probe="$(mf '.adl2.archetype_library.loinc.probe')"
loinc_named="$(mf '.adl2.archetype_library.loinc.refused_when_served[]')"
loinc_count="$(mf '.adl2.archetype_library.loinc.refused_when_served | length')"
loinc_outcome=refused
code=$(request "LOINC probe $loinc_probe" GET "$EXT/terminology/http%3A%2F%2Floinc.org/term/$loinc_probe")
case "${SANDBOX_TERMINOLOGY:-}:$code" in
  :200) echo "==> the CDR resolves LOINC $loinc_probe; VETDF verifies the library's LOINC bindings" ;;
  :404)
    echo "::error::the CDR resolves no LOINC (GET /terminology/http://loinc.org/term/$loinc_probe answered 404). The archetype-library pin assumes a terminology server that serves LOINC (FerroTERM with the LOINC index, FERROTERM_INDEXES); without it VETDF cannot refuse ${loinc_named//$'\n'/, }, and the library loads $((want_accepted + loinc_count)) / $((want_refused - loinc_count)). Fix the terminology posture, or set SANDBOX_TERMINOLOGY=without-loinc for a run that has no LOINC." >&2
    exit 1
    ;;
  without-loinc:404)
    loinc_outcome=accepted
    want_accepted=$((want_accepted + loinc_count))
    want_refused=$((want_refused - loinc_count))
    echo "==> SANDBOX_TERMINOLOGY=without-loinc and the CDR resolves no LOINC: expecting $want_accepted / $want_refused"
    ;;
  without-loinc:200)
    echo "::error::SANDBOX_TERMINOLOGY=without-loinc, but the CDR resolves LOINC $loinc_probe; unset it" >&2
    exit 1
    ;;
  *)
    if [[ -n "${SANDBOX_TERMINOLOGY:-}" && "${SANDBOX_TERMINOLOGY}" != without-loinc ]]; then
      echo "::error::SANDBOX_TERMINOLOGY is '$SANDBOX_TERMINOLOGY'; want unset or without-loinc" >&2
      exit 1
    fi
    expect "LOINC probe $loinc_probe" "$code" 200 404
    ;;
esac

accepted=0
refused=0
: > "$WORK/library-refused"
while read -r adls; do
  code=$(request "archetype $(basename "$adls")" POST "$BASE/definition/template/adl2" \
    -H "$CT_PLAIN" --data-binary @"$adls")
  case "$code" in
    200 | 201 | 409) accepted=$((accepted + 1)) ;;
    "$refused_status")
      refused=$((refused + 1))
      basename "$adls" >> "$WORK/library-refused"
      ;;
    *) expect "archetype $(basename "$adls")" "$code" 201 409 "$refused_status" ;;
  esac
done < <(adls_parents_first "$lib_root")

if [[ "$accepted" != "$want_accepted" ]] || [[ "$refused" != "$want_refused" ]]; then
  echo "::error::the ADL 2 archetype library loaded $accepted accepted / $refused refused; the manifest pins $want_accepted / $want_refused. Re-check the corpus and the manifest together." >&2
  exit 1
fi
while read -r named; do
  if grep -qxF "$named" "$WORK/library-refused"; then outcome=refused; else outcome=accepted; fi
  [[ "$outcome" == "$loinc_outcome" ]] || {
    echo "::error::$named was $outcome; under this terminology posture the manifest expects it $loinc_outcome" >&2
    exit 1
  }
done <<< "$loinc_named"
echo "==> $accepted ADL 2 archetypes stored ($refused refused by AOM2 validation, as pinned)"

# ── 4. this repository's own ADL 2 artefacts and templates ───────────────────
#
# The archetypes go first: a SOURCE template's `use_archetype` slot only
# flattens once its filler is in the store.

adl2_artefacts=0
while read -r rel; do
  file="$ROOT_DIR/$rel"
  [[ -f "$file" ]] || {
    echo "::error::missing ADL 2 artefact $file" >&2
    exit 1
  }
  code=$(request "adl2 artefact $(basename "$rel")" POST "$BASE/definition/template/adl2" \
    -H "$CT_PLAIN" --data-binary @"$file")
  expect "adl2 artefact $(basename "$rel")" "$code" 201 409
  adl2_artefacts=$((adl2_artefacts + 1))
done < <(mf '.adl2.artefacts[]')

adl2_templates=0
while read -r rel; do
  file="$ROOT_DIR/$rel"
  [[ -f "$file" ]] || {
    echo "::error::missing ADL 2 template $file" >&2
    exit 1
  }
  code=$(request "adl2 template $(basename "$rel")" POST "$BASE/definition/template/adl2" \
    -H "$CT_PLAIN" --data-binary @"$file")
  expect "adl2 template $(basename "$rel")" "$code" 201 409
  adl2_templates=$((adl2_templates + 1))
done < <(mf '.adl2.templates[].file')
echo "==> $adl2_artefacts ADL 2 archetypes + $adl2_templates ADL 2 templates of our own"

# ── 5. the EHRs ──────────────────────────────────────────────────────────────
#
# Resolved by subject first: the subject of an EHR_STATUS is unique across the
# repository, so a re-run reuses what it finds instead of failing on the
# duplicate. Only a freshly created EHR is filled below.

ehrs=()
created=()
reused=0
index=0
while read -r entry; do
  subject=$(printf '%s' "$entry" | jq -r '.subject')
  subject_ref="$(subject_id "$subject")"
  status_file="$SEED_DIR/$(printf '%s' "$entry" | jq -r '.status')"
  token "__SUBJECT_ID:${index}__" "$subject_ref"
  code=$(request "ehr lookup $subject" GET \
    "$BASE/ehr?subject_id=$subject_ref&subject_namespace=$SUBJECT_NS")
  case "$code" in
    200)
      ehrs[index]=$(jq -r '.ehr_id.value' "$BODY")
      created[index]=no
      reused=$((reused + 1))
      ;;
    404)
      render "$status_file" "$WORK/status.json" "__SUBJECT_ID__=$subject_ref" \
        "__SUBJECT_NS__=$SUBJECT_NS"
      code=$(request "ehr create $subject" POST "$BASE/ehr" \
        -H "$CT_JSON" --data-binary @"$WORK/status.json")
      expect "ehr create $subject" "$code" 201
      ehrs[index]=$(etag_uid)
      created[index]=yes
      ;;
    *) expect "ehr lookup $subject" "$code" 200 404 ;;
  esac
  : > "$WORK/comps.$index"
  index=$((index + 1))
done < <(manifest_ehrs)
echo "==> ${#ehrs[@]} demo EHRs ($reused reused)"

# ── 6. compositions ──────────────────────────────────────────────────────────

compositions=0
extra=0

# Commit `$3` copies of the body `$2` into the EHR at index `$1`, recording each
# new VERSIONED_OBJECT uid; then add `$4` further versions to the first of them,
# so LATEST_VERSION and ALL_VERSIONS differ on real data.
commit_series() {
  local slot="$1" body="$2" copies="$3" versions="$4" label="$5"
  local ehr="${ehrs[$slot]}" ovid vo code first=""
  for _ in $(seq 1 "$copies"); do
    code=$(request "$label" POST "$BASE/ehr/$ehr/composition" \
      -H "$CT_JSON" -H 'Prefer: return=minimal' \
      --data-binary @"$body")
    expect "$label" "$code" 201
    ovid=$(etag_uid)
    printf '%s\n' "${ovid%%::*}" >> "$WORK/comps.$slot"
    [[ -n "$first" ]] || first="$ovid"
    series_first="${first%%::*}"
    compositions=$((compositions + 1))
  done
  [[ "$versions" -gt 0 ]] || return 0
  vo="${first%%::*}"
  ovid="$first"
  jq 'del(.uid)' "$body" > "$WORK/update.json"
  for _ in $(seq 1 "$versions"); do
    code=$(request "$label version" PUT "$BASE/ehr/$ehr/composition/$vo" \
      -H "$CT_JSON" -H "If-Match: \"$ovid\"" \
      --data-binary @"$WORK/update.json")
    expect "$label version" "$code" 200 204
    ovid=$(etag_uid)
    extra=$((extra + 1))
  done
}

while read -r entry; do
  slug=$(printf '%s' "$entry" | jq -r '.slug')
  copies=$(printf '%s' "$entry" | jq -r '.compositions_per_ehr')
  versions=$(printf '%s' "$entry" | jq -r '.extra_versions')
  example=$(printf '%s' "$entry" | jq -r --arg d "$TPL_DIR/$slug.example.json" '.example // $d')
  if [[ "$example" == generated ]]; then
    # The CDR's own example of the template, as the browser journeys commit it.
    template=$(printf '%s' "$entry" | jq -r '.template_id')
    code=$(request "example $template" GET \
      "$BASE/definition/template/adl1.4/$(jq -rn --arg t "$template" '$t | @uri')/example" \
      -H 'Accept: application/json')
    expect "example $template" "$code" 200
    example="$WORK/$slug.example.json"
    cp "$BODY" "$example"
  fi
  [[ "$example" = /* ]] || example="$ROOT_DIR/$example"
  [[ -f "$example" ]] || {
    echo "::error::missing example composition $example" >&2
    exit 1
  }
  first_slot=yes
  while read -r slot; do
    [[ "${created[$slot]}" == yes ]] || continue
    if [[ "$first_slot" == yes ]]; then
      commit_series "$slot" "$example" "$copies" "$versions" "composition $slug"
      token "__SERIES:${slug}__" "$series_first"
      first_slot=no
    else
      commit_series "$slot" "$example" "$copies" 0 "composition $slug"
    fi
  done < <(printf '%s' "$entry" | jq -r '.ehrs[]')
done < <(jq -c '.adl14_templates[]' "$MANIFEST")

# The ADL 2 side commits the CDR's OWN generated example for each template, so
# that path carries real versioned content rather than definitions alone.
while read -r entry; do
  template=$(printf '%s' "$entry" | jq -r '.id')
  copies=$(printf '%s' "$entry" | jq -r '.compositions_per_ehr')
  [[ "$copies" -gt 0 ]] || continue
  code=$(request "adl2 example $template" GET \
    "$BASE/definition/template/adl2/$template/example" -H 'Accept: application/json')
  expect "adl2 example $template" "$code" 200
  cp "$BODY" "$WORK/adl2-example.json"
  while read -r slot; do
    [[ "${created[$slot]}" == yes ]] || continue
    commit_series "$slot" "$WORK/adl2-example.json" "$copies" 0 "composition $template"
  done < <(printf '%s' "$entry" | jq -r '.ehrs[]')
done < <(jq -c '.adl2.templates[]' "$MANIFEST")

# Compositions sent in the Simplified Flat format, with the measured values a
# chart draws: the CDR's generated example carries none.
while read -r entry; do
  slot=$(printf '%s' "$entry" | jq -r '.ehr')
  [[ "${created[$slot]}" == yes ]] || continue
  template=$(printf '%s' "$entry" | jq -r '.template_id')
  file="$SEED_DIR/$(printf '%s' "$entry" | jq -r '.file')"
  code=$(request "flat composition $template" POST "$BASE/ehr/${ehrs[$slot]}/composition" \
    -H 'Content-Type: application/openehr.wt.flat+json' -H "openehr-template-id: $template" \
    -H 'Prefer: return=minimal' --data-binary @"$file")
  expect "flat composition $template" "$code" 201
  ovid=$(etag_uid)
  printf '%s\n' "${ovid%%::*}" >> "$WORK/comps.$slot"
  compositions=$((compositions + 1))
done < <(jq -c '.flat_compositions[]' "$MANIFEST")
echo "==> $compositions compositions committed, $extra further versions"

# ── 7. the directory ─────────────────────────────────────────────────────────

directories=0
folder_file="$SEED_DIR/$(mf '.directory.folder')"
while read -r slot; do
  [[ "${created[$slot]}" == yes ]] || continue
  ehr="${ehrs[$slot]}"
  render "$folder_file" "$WORK/folder.json" \
    "__COMPOSITION_1__=$(sed -n 1p "$WORK/comps.$slot")" \
    "__COMPOSITION_2__=$(sed -n 2p "$WORK/comps.$slot")" \
    "__COMPOSITION_3__=$(sed -n 3p "$WORK/comps.$slot")"
  code=$(request "directory $ehr" POST "$BASE/ehr/$ehr/directory" \
    -H "$CT_JSON" --data-binary @"$WORK/folder.json")
  expect "directory $ehr" "$code" 201 409
  directories=$((directories + 1))
done < <(mf '.directory.ehrs[]')
echo "==> $directories EHR directories"

# ── 8. the second EHR_STATUS version ─────────────────────────────────────────
#
# Applied after the per-EHR content: `is_modifiable = false` is a write guard,
# so an EHR that ends up locked has to be filled first (RM ehr master04 §EHR
# Active Status).

statuses=0
index=0
while read -r entry; do
  slot=$index
  index=$((index + 1))
  [[ "${created[$slot]}" == yes ]] || continue
  final=$(printf '%s' "$entry" | jq -r '.final_status // empty')
  [[ -n "$final" ]] || continue
  subject=$(printf '%s' "$entry" | jq -r '.subject')
  subject_ref="$(subject_id "$subject")"
  ehr="${ehrs[$slot]}"
  code=$(request "ehr_status read $ehr" GET "$BASE/ehr/$ehr/ehr_status")
  expect "ehr_status read $ehr" "$code" 200
  current=$(etag_uid)
  render "$SEED_DIR/$final" "$WORK/status.json" "__SUBJECT_ID__=$subject_ref" \
    "__SUBJECT_NS__=$SUBJECT_NS"
  code=$(request "ehr_status update $ehr" PUT "$BASE/ehr/$ehr/ehr_status" \
    -H "$CT_JSON" -H "If-Match: \"$current\"" \
    --data-binary @"$WORK/status.json")
  expect "ehr_status update $ehr" "$code" 200 204
  statuses=$((statuses + 1))
done < <(manifest_ehrs)
echo "==> $statuses further EHR_STATUS versions"

# ── 9. demographics ──────────────────────────────────────────────────────────

persons=()
index=0
while read -r rel; do
  code=$(request "person $(basename "$rel")" POST "$BASE/demographic/person" \
    -H "$CT_JSON" --data-binary @"$SEED_DIR/$rel")
  expect "person $(basename "$rel")" "$code" 201
  ovid=$(etag_uid)
  persons[index]="${ovid%%::*}"
  token "__ID:person-$(printf '%02d' $((index + 1)))__" "${persons[index]}"
  index=$((index + 1))
done < <(mf '.demographics.persons[]')

roles=0
while read -r entry; do
  file=$(printf '%s' "$entry" | jq -r '.file')
  performer=$(printf '%s' "$entry" | jq -r '.performer')
  render "$SEED_DIR/$file" "$WORK/role.json" "__PERFORMER__=${persons[$performer]}"
  code=$(request "role $(basename "$file")" POST "$BASE/demographic/role" \
    -H "$CT_JSON" --data-binary @"$WORK/role.json")
  expect "role $(basename "$file")" "$code" 201
  roles=$((roles + 1))
done < <(jq -c '.demographics.roles[]' "$MANIFEST")

relationships=0
while read -r entry; do
  file=$(printf '%s' "$entry" | jq -r '.file')
  source_index=$(printf '%s' "$entry" | jq -r '.source')
  target_index=$(printf '%s' "$entry" | jq -r '.target')
  render "$SEED_DIR/$file" "$WORK/relationship.json" \
    "__SOURCE__=${persons[$source_index]}" "__TARGET__=${persons[$target_index]}"
  code=$(request "relationship $(basename "$file")" POST \
    "$EXT/demographic/party_relationship" \
    -H "$CT_JSON" --data-binary @"$WORK/relationship.json")
  expect "relationship $(basename "$file")" "$code" 201
  relationships=$((relationships + 1))
done < <(jq -c '.demographics.relationships[]' "$MANIFEST")
echo "==> ${#persons[@]} persons, $roles roles, $relationships party relationships"

# ── 10. the world every viewer page reads ────────────────────────────────────
#
# The provider, its care teams, a system client, the patients with their
# record subjects and the grants of the seeding administrator, then the access
# policy, the care relationships, the EHR access settings, the legal marks,
# the attestations and the reads that fill the access log. Every body is a
# file under world/, access/ or records/ whose `__ID:<key>__`, `__SUBJECT_ID:<n>__`,
# `__LOGIN:<who>__` and `__DATE:<days>__` tokens the walker substitutes.

# The login whose PERSON the token `__ID:<key>__` names: its container id, read
# from the PERSON whose details carry the login as a `login` identifier.
login_person() {
  local key="$1" login="$2" code id
  jq -n --arg q "SELECT p/uid/value, p/details FROM PERSON p" '{q: $q}' > "$WORK/q.json"
  code=$(request "PERSON of $login" POST "$BASE/query/aql" -H "$CT_JSON" --data-binary @"$WORK/q.json")
  expect "PERSON of $login" "$code" 200
  id=$(jq -r --arg l "$login" '[.rows[] | select([.[1].items[]?.value
      | select(type == "object" and .type == "login" and .id == $l)] | length > 0)
      | .[0] | split("::")[0]] | first // empty' "$BODY")
  [[ -n "$id" ]] || {
    echo "::error::no PERSON carries the login $login; the seed needs it bound before it runs" >&2
    exit 1
  }
  token "__ID:${key}__" "$id"
}

# Writes the commit of a world body `$1` (a file under the seed directory),
# rendered, to `$2` and fails on a token the walker does not know.
world_body() {
  local src="$SEED_DIR/$1" dst="$2"
  [[ -f "$src" ]] || {
    echo "::error::missing seed body $src" >&2
    exit 1
  }
  jq --slurpfile t "$TOKENS" --argjson today "$TODAY" '
    def day($d): ($today + ($d | tonumber) * 86400) | strftime("%Y-%m-%d");
    walk(if type == "string" then
      (if $t[0][.] != null then $t[0][.] else . end)
      | gsub("__DATE:(?<d>[+-]?[0-9]+)__"; day(.d))
      | gsub("__(?<k>[A-Z_]+:[A-Za-z0-9.-]+)__"; $t[0]["__" + .k + "__"] // ("__" + .k + "__"))
    else . end)' "$src" > "$dst"
  if grep -qE '__[A-Z_]+(:[A-Za-z0-9.-]+)?__' "$dst"; then
    echo "::error::$1 carries a token the walker cannot fill: $(grep -oE '__[A-Z_]+(:[A-Za-z0-9.-]+)?__' "$dst" | sort -u | paste -sd' ' -)" >&2
    exit 1
  fi
}

# The versioned object id of the n-th (1-based) composition committed into EHR slot `$1`.
composition_of() {
  local slot="$1" n="$2" vo
  vo=$(sed -n "${n}p" "$WORK/comps.$slot")
  [[ -n "$vo" ]] || {
    echo "::error::EHR slot $slot holds no composition $n for the world to name" >&2
    exit 1
  }
  printf '%s' "$vo"
}

# The current version uid the `ETag` of a GET of `$2` answers.
current_version() {
  local label="$1" url="$2" code
  code=$(request "$label" GET "$url")
  expect "$label" "$code" 200
  etag_uid
}

# The provider the deployment acts for (`[authz.party_binding] provider`): the
# demo hospital is that provider, so its care relationships and
# representations are the deployment's own.
code=$(request "deployment provider" GET "$EXT/admin/config")
expect "deployment provider" "$code" 200
token "__PROVIDER_ISSUER__" "$(jq -r '.authz.party_binding.provider.issuer // empty' "$BODY")"
token "__PROVIDER_ID__" "$(jq -r '.authz.party_binding.provider.id // empty' "$BODY")"
jq -e '.["__PROVIDER_ID__"] != ""' "$TOKENS" > /dev/null || {
  echo "::error::the deployment names no [authz.party_binding] provider; the world's care relationships need one" >&2
  exit 1
}
login_person seed-admin "$SEED_LOGIN"
[[ -n "$CLINICIAN" ]] && login_person clinician "$CLINICIAN"
token "__LOGIN:clinician__" "$CLINICIAN"
token "__LOGIN:proxy__" "$PROXY"

world_parties=0
while read -r entry; do
  key=$(jq -r '.key' <<< "$entry")
  kind=$(jq -r '.kind' <<< "$entry")
  world_body "$(jq -r '.file' <<< "$entry")" "$WORK/party.json"
  code=$(request "$kind $key" POST "$BASE/demographic/$kind" -H "$CT_JSON" --data-binary @"$WORK/party.json")
  expect "$kind $key" "$code" 201
  ovid=$(etag_uid)
  token "__ID:${key}__" "${ovid%%::*}"
  world_parties=$((world_parties + 1))
done < <(jq -c '.world.parties[]' "$MANIFEST")

world_versions=0
while read -r entry; do
  key=$(jq -r '.key' <<< "$entry")
  kind=$(jq -r '.kind' <<< "$entry")
  id=$(jq -r --arg k "__ID:${key}__" '.[$k]' "$TOKENS")
  current=$(current_version "$kind $key read" "$BASE/demographic/$kind/$id")
  world_body "$(jq -r '.file' <<< "$entry")" "$WORK/party.json"
  code=$(request "$kind $key version" PUT "$BASE/demographic/$kind/$id" -H "$CT_JSON" \
    -H "If-Match: \"$current\"" --data-binary @"$WORK/party.json")
  expect "$kind $key version" "$code" 200 204
  world_versions=$((world_versions + 1))
done < <(jq -c '.world.versions[]' "$MANIFEST")

while read -r entry; do
  key=$(jq -r '.key' <<< "$entry")
  world_body "$(jq -r '.file' <<< "$entry")" "$WORK/relationship.json"
  code=$(request "relationship $key" POST "$EXT/demographic/party_relationship" -H "$CT_JSON" \
    --data-binary @"$WORK/relationship.json")
  expect "relationship $key" "$code" 201
  ovid=$(etag_uid)
  token "__ID:${key}__" "${ovid%%::*}"
done < <(jq -c '.world.relationships[]' "$MANIFEST")
echo "==> $world_parties world parties, $world_versions further party versions, the representation"

# The care relationships, each written on the route of its subject; an
# `ended` body closes one as a second version.
cares=0
while read -r entry; do
  key=$(jq -r '.key' <<< "$entry")
  slot=$(jq -r '.ehr' <<< "$entry")
  subject="$(subject_id "$(jq -r --argjson s "$slot" '.ehrs[$s].subject' "$MANIFEST")")"
  world_body "$(jq -r '.file' <<< "$entry")" "$WORK/care.json"
  code=$(request "care relationship $key" POST \
    "$EXT/care_relationship?subject_id=$subject&subject_namespace=$SUBJECT_NS" \
    -H "$CT_JSON" --data-binary @"$WORK/care.json")
  expect "care relationship $key" "$code" 201
  ovid=$(etag_uid)
  token "__ID:${key}__" "${ovid%%::*}"
  ended=$(jq -r '.ended // empty' <<< "$entry")
  if [[ -n "$ended" ]]; then
    world_body "$ended" "$WORK/care.json"
    code=$(request "care relationship $key ended" PUT "$EXT/care_relationship/${ovid%%::*}" \
      -H "$CT_JSON" -H "If-Match: \"$ovid\"" --data-binary @"$WORK/care.json")
    expect "care relationship $key ended" "$code" 200 204
  fi
  cares=$((cares + 1))
done < <(jq -c '.world.care_relationships[]' "$MANIFEST")
echo "==> $cares care relationships"

# The access policy: its first version holds professionals to a care
# relationship, so the clinician's reads of two EHRs outside one rest on a
# justification and an emergency with their reasons; the final version lifts
# that requirement for the visitors who browse every record.
policy_url="$EXT/admin/access_policy"
world_body "$(mf '.world.access_policy.first')" "$WORK/policy.json"
code=$(request "access policy" PUT "$policy_url" -H "$CT_JSON" -H 'If-None-Match: *' \
  --data-binary @"$WORK/policy.json")
expect "access policy" "$code" 201
policy_version=$(etag_uid)
justified=0
if [[ -n "$CLINICIAN" ]]; then
  while read -r entry; do
    slot=$(jq -r '.ehr' <<< "$entry")
    purpose=$(jq -r '.purpose' <<< "$entry")
    reason=$(jq -r '.reason | @uri' <<< "$entry")
    code=$(REQ_AUTH="$CLINICIAN:$CLINICIAN_PASS" request "justified read ($purpose)" GET \
      "$BASE/ehr/${ehrs[$slot]}/ehr_status" -H "x-purpose-of-use: $purpose")
    if [[ "$code" != 403 ]] || ! grep -q 'refused-reason-missing' "$BODY"; then
      echo "::error::the clinician's read of EHR slot $slot under $purpose without a reason answered $code; the first policy version should ask for the reason" >&2
      head -c 400 "$BODY" >&2
      exit 1
    fi
    code=$(REQ_AUTH="$CLINICIAN:$CLINICIAN_PASS" request "justified read ($purpose)" GET \
      "$BASE/ehr/${ehrs[$slot]}/ehr_status" -H "x-purpose-of-use: $purpose" -H "x-access-reason: $reason")
    expect "justified read ($purpose)" "$code" 200
    justified=$((justified + 1))
  done < <(jq -c '.world.access_policy.justified[]' "$MANIFEST")
fi
world_body "$(mf '.world.access_policy.final')" "$WORK/policy.json"
code=$(request "access policy version" PUT "$policy_url" -H "$CT_JSON" -H "If-Match: \"$policy_version\"" \
  --data-binary @"$WORK/policy.json")
expect "access policy version" "$code" 200
echo "==> the access policy in 2 versions, $justified justified and emergency reads"

# The EHR access settings, each a new EHR_ACCESS version committed on the
# person's instruction over the admin route.
settings=0
while read -r entry; do
  slot=$(jq -r '.ehr' <<< "$entry")
  ehr="${ehrs[$slot]}"
  current=$(current_version "ehr_access read $ehr" "$EXT/ehr/$ehr/ehr_access")
  world_body "$(jq -r '.file' <<< "$entry")" "$WORK/settings.json"
  jq --arg e "$ehr" --arg p "$current" '{ehr_id: $e, preceding_version_uid: $p, settings: .}' \
    "$WORK/settings.json" > "$WORK/ehr-access.json"
  code=$(request "ehr_access $ehr" POST "$EXT/admin/ehr-access" -H "$CT_JSON" --data-binary @"$WORK/ehr-access.json")
  expect "ehr_access $ehr" "$code" 200
  settings=$((settings + 1))
done < <(jq -c '.world.ehr_access[]' "$MANIFEST")
echo "==> $settings EHR access settings"

# The legal marks: a restriction of processing, a research objection, the
# access-delay register with one held composition, and the retention register
# with two anchors, a whole-record hold and an object hold.
marks="$WORK/marks.json"
jq '.world.legal_marks' "$MANIFEST" > "$marks"
admin_post() {
  local label="$1" path="$2" body="$3" code
  shift 3
  code=$(request "$label" POST "$EXT/admin/$path" -H "$CT_JSON" -d "$body")
  expect "$label" "$code" "$@"
}
mark_vo() { composition_of "${ehrs[$(jq -r ".$1.ehr" "$marks")]}" "$(jq -r ".$1.composition" "$marks")"; }
restricted_slot=$(jq -r '.restriction.ehr' "$marks")
admin_post "restriction" restriction "$(jq -c --arg e "${ehrs[$restricted_slot]}" \
  --arg v "$(composition_of "$restricted_slot" "$(jq -r '.restriction.composition' "$marks")")" \
  '.restriction | {ehr_id: $e, vo_id: $v, ground, note}' "$marks")" 204
admin_post "research objection" research-objection "$(jq -c \
  --arg e "${ehrs[$(jq -r '.research_objection.ehr' "$marks")]}" \
  '.research_objection | {ehr_id: $e, objected: true, ground}' "$marks")" 204
code=$(request "access-delay register" GET "$EXT/admin/registers/access-delay")
expect "access-delay register" "$code" 200
register_version=$(header_value etag | tr -d '"')
code=$(request "access-delay register" PUT "$EXT/admin/registers/access-delay" -H "$CT_JSON" \
  -H "If-Match: \"$register_version\"" -d "$(jq -c '.access_delay_register' "$marks")")
expect "access-delay register" "$code" 200
delay_slot=$(jq -r '.access_delay.ehr' "$marks")
admin_post "access delay" access-delay "$(jq -c --arg e "${ehrs[$delay_slot]}" \
  --arg v "$(composition_of "$delay_slot" "$(jq -r '.access_delay.composition' "$marks")")" \
  '.access_delay | {ehr_id: $e, vo_id: $v, note}' "$marks")" 204
while read -r period; do
  code=$(request "retention period" PUT "$EXT/admin/retention/policy" -H "$CT_JSON" -d "$period")
  expect "retention period" "$code" 204
done < <(jq -c '.retention_policies[]' "$marks")
while read -r anchor; do
  slot=$(jq -r '.ehr' <<< "$anchor")
  code=$(request "retention anchor" PUT "$EXT/admin/retention/anchor" -H "$CT_JSON" \
    -d "$(jq -c --arg e "${ehrs[$slot]}" 'del(.ehr) + {ehr_id: $e}' <<< "$anchor")")
  expect "retention anchor" "$code" 204
done < <(jq -c '.retention_anchors[]' "$marks")
while read -r hold; do
  slot=$(jq -r '.ehr' <<< "$hold")
  admin_post "retention hold" retention/hold "$(jq -nc \
    --arg v "$(composition_of "$slot" "$(jq -r '.composition' <<< "$hold")")" '{vo_id: $v, held: true}')" 204
done < <(jq -c '.retention_holds[]' "$marks")
code=$(request "retention due" GET "$EXT/admin/retention/due")
expect "retention due" "$code" 200
[[ "$(jq '.due | length' "$BODY")" -gt 0 ]] || {
  echo "::error::the retention register lists nothing due; the anchors should put two EHRs past their period" >&2
  exit 1
}
echo "==> the legal marks: a restriction, an objection, an access delay, $(jq '.due | length' "$BODY") retention periods due"

# The attestations: the clinician commits a signed and a pending one, each
# with the composition it attests, through a CONTRIBUTION.
attested=0
if [[ -n "$CLINICIAN" ]]; then
  while read -r entry; do
    slot=$(jq -r '.ehr' <<< "$entry")
    world_body "$(jq -r '.file' <<< "$entry")" "$WORK/contribution.json"
    code=$(REQ_AUTH="$CLINICIAN:$CLINICIAN_PASS" request "attestation" POST \
      "$BASE/ehr/${ehrs[$slot]}/contribution" -H "$CT_JSON" -H 'x-purpose-of-use: TREAT' \
      --data-binary @"$WORK/contribution.json")
    expect "attestation" "$code" 201
    attested=$((attested + 1))
  done < <(jq -c '.world.attestations[]' "$MANIFEST")
fi

archetypes=0
while read -r rel; do
  code=$(request "archetype $(basename "$rel")" POST "$EXT/definition/archetype/adl1.4" \
    -H "$CT_PLAIN" --data-binary @"$ROOT_DIR/$rel")
  expect "archetype $(basename "$rel")" "$code" 201 409
  archetypes=$((archetypes + 1))
done < <(mf '.world.adl14_archetypes[]')
echo "==> $attested attested versions, $archetypes ADL 1.4 archetypes"

# ── 11. stored AQL queries ───────────────────────────────────────────────────
#
# Each one is executed after it is stored and must answer at least the row count
# the manifest pins: a query the demo data cannot feed is a broken demo, not a
# detail to discover from the viewer.

queries=0
while read -r entry; do
  name=$(printf '%s' "$entry" | jq -r '.name')
  version=$(printf '%s' "$entry" | jq -r '.version')
  aql="$SEED_DIR/$(printf '%s' "$entry" | jq -r '.file')"
  min_rows=$(printf '%s' "$entry" | jq -r '.min_rows')
  [[ -f "$aql" ]] || {
    echo "::error::missing AQL body $aql" >&2
    exit 1
  }
  code=$(request "query $name" PUT "$BASE/definition/query/$name/$version" \
    -H "$CT_PLAIN" --data-binary @"$aql")
  expect "query $name" "$code" 200 201 409

  parameters=$(printf '%s' "$entry" | jq -c '.parameters // empty')
  if [[ -n "$parameters" ]]; then
    printf '%s' "$parameters" \
      | jq --arg e "${ehrs[0]}" '{query_parameters: walk(if . == "__EHR_0__" then $e else . end)}' \
        > "$WORK/params.json"
    code=$(request "query run $name" POST "$BASE/query/$name" \
      -H "$CT_JSON" --data-binary @"$WORK/params.json")
  else
    code=$(request "query run $name" GET "$BASE/query/$name")
  fi
  expect "query run $name" "$code" 200

  # A cohort query runs the stored text on the cohort its subjects name
  # (FerroEHR's cohort extension), after the stored run above.
  subjects=$(printf '%s' "$entry" | jq -c '.cohort_subjects // empty')
  if [[ -n "$subjects" ]]; then
    jq -n --rawfile q "$aql" --argjson slots "$subjects" --slurpfile t "$TOKENS" --arg ns "$SUBJECT_NS" \
      '{q: $q, purpose: "secondary-use",
        subjects: [$slots[] | {id: $t[0]["__SUBJECT_ID:\(.)__"], namespace: $ns}]}' > "$WORK/cohort.json"
    code=$(request "cohort run $name" POST "$EXT/query/cohort" -H "$CT_JSON" --data-binary @"$WORK/cohort.json")
    expect "cohort run $name" "$code" 200
  fi
  rows=$(jq '.rows | length' "$BODY")
  if [[ "$rows" -lt "$min_rows" ]]; then
    echo "::error::stored query $name returned $rows rows, below the $min_rows the manifest pins — the seeded data no longer feeds it." >&2
    exit 1
  fi
  echo "query $name -> $rows rows"
  queries=$((queries + 1))
done < <(jq -c '.stored_queries[]' "$MANIFEST")
echo "==> $queries stored AQL queries, each answering rows"

# ── 12. the logs ─────────────────────────────────────────────────────────────
#
# Every request above wrote its access-log entry and its system record. The
# patient's representative reads the subject view of the patient's access log; the review across EHRs reads the full view; and, where the
# server signs with an OpenPGP key, the log is sealed into a signed checkpoint
# and verified.

patient_slot=$(mf '.world.logs.patient_ehr')
patient_ehr="${ehrs[$patient_slot]}"
if [[ -n "$PROXY" ]]; then
  code=$(REQ_AUTH="$PROXY:$PROXY_PASS" request "the representative's view of the access log" GET \
    "$EXT/ehr/$patient_ehr/access_log")
  expect "the representative's view of the access log" "$code" 200
  [[ "$(jq '.entries | length' "$BODY")" -gt 0 ]] || {
    echo "::error::the subject view of EHR $patient_ehr's access log lists no entry" >&2
    exit 1
  }
  echo "==> the subject view of a patient's access log lists $(jq '.entries | length' "$BODY") entries"
fi

min_review=$(mf '.world.logs.min_review_ehrs')
code=$(request "access-log review" GET "$EXT/admin/access-log?fetch=1000")
expect "access-log review" "$code" 200
reviewed=$(jq '[.entries[].ehr_id.value? // empty] | unique | length' "$BODY")
[[ "$reviewed" -ge "$min_review" ]] || {
  echo "::error::the access-log review names $reviewed EHRs, below the $min_review the manifest pins" >&2
  exit 1
}

# The system records (IHE ATNA, read over ITI-81) of the day the seed ran.
FHIR="${SANDBOX_BASE:-https://sandbox.ferroehr.eu}/ferroehr/rest/fhir/r4"
code=$(request "system records" GET "$FHIR/AuditEvent?date=ge$(date -u -r "$TODAY" +%Y-%m-%d 2>/dev/null || date -u -d "@$TODAY" +%Y-%m-%d)&_count=10")
expect "system records" "$code" 200
[[ "$(jq '.entry | length' "$BODY")" -gt 0 ]] || {
  echo "::error::ITI-81 answers no system record for the day the seed ran" >&2
  exit 1
}

# The kind of signature the server writes, read off the seeding admin's
# role version: an OpenPGP signature names a key the checkpoint is signed with.
role_id=$(jq -r '.["__ID:role-privacy-officer__"]' "$TOKENS")
code=$(request "signature kind" GET "$BASE/demographic/versioned_party/$role_id/version")
expect "signature kind" "$code" 200
if jq -e '.signature // "" | startswith("-----BEGIN PGP SIGNATURE-----")' "$BODY" > /dev/null; then
  code=$(request "access-log checkpoint" POST "$EXT/admin/access-log/checkpoint")
  expect "access-log checkpoint" "$code" 200 201
  code=$(request "access-log verification" POST "$EXT/admin/access-log/verify")
  expect "access-log verification" "$code" 200
  echo "==> the access log of $reviewed EHRs, sealed into a signed checkpoint that verifies"
else
  echo "==> the access log of $reviewed EHRs; the server signs versions by digest, so no checkpoint is signed"
fi

# The ids a harness needs to address the seeded records, when asked for.
if [[ -n "$IDS_OUT" ]]; then
  printf '%s\n' "${ehrs[@]}" | jq -R . | jq -s --slurpfile t "$TOKENS" '{ehrs: ., tokens: $t[0]}' > "$IDS_OUT"
  echo "==> the seeded ids are in $IDS_OUT"
fi

# ── 13. the completion marker ────────────────────────────────────────────────

render "$SEED_DIR/$(mf '.marker.status')" "$WORK/marker.json" \
  "__SUBJECT_ID__=$MARKER_ID" "__SUBJECT_NS__=$SUBJECT_NS"
code=$(request "marker create" POST "$BASE/ehr" \
  -H "$CT_JSON" --data-binary @"$WORK/marker.json")
expect "marker create" "$code" 201 409

# Every version the seed committed names the demo PERSON: the binding
# attributes each commit of the bound login to it. The marker's EHR_STATUS
# version is read back to show the committer references a PERSON.
code=$(request "marker read" GET \
  "$BASE/ehr?subject_id=$MARKER_ID&subject_namespace=$SUBJECT_NS")
expect "marker read" "$code" 200
marker_ehr="$(jq -r '.ehr_id.value' "$BODY")"
code=$(request "marker status version" GET "$BASE/ehr/$marker_ehr/versioned_ehr_status/version")
expect "marker status version" "$code" 200
committer_class="$(jq -r '.commit_audit.committer.external_ref.type // empty' "$BODY")"
[[ "$committer_class" == PERSON ]] || {
  echo "::error::the marker's EHR_STATUS version names no PERSON as its committer" >&2
  jq -c '.commit_audit.committer' "$BODY" >&2
  exit 1
}

echo "==> seeded ${#ehrs[@]} EHRs, $compositions compositions (+$extra versions), $directories directories, ${#persons[@]} persons, $queries stored queries"
