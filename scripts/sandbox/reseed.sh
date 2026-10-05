#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# Seed the hosted sandbox (#2710, dataset #3045) with demo data through the
# PUBLIC API — the same surface visitors use, so the seed itself proves the
# deployment. This file is a WALKER: what gets loaded is
# scripts/sandbox/seed/manifest.json plus the bodies beside it, so adding
# content is editing data. The manifest's `notes` describe the dataset, its
# measured runtime and the placeholder tokens substituted below.
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
# SANDBOX_USER / SANDBOX_PASS (default the public demo credentials).
set -Eeuo pipefail

BASE="${SANDBOX_BASE:-https://sandbox.ferroehr.eu}/ferroehr/rest/openehr/v1"
AUTH="${SANDBOX_USER:-ferroehr}:${SANDBOX_PASS:-ferroehr}"
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

# ── the manifest ─────────────────────────────────────────────────────────────

# Read the manifest with a jq filter, raw output.
mf() { jq -r "$1" "$MANIFEST"; }

SUBJECT_NS="$(mf '.subject_namespace')"

# ── the wire ─────────────────────────────────────────────────────────────────

# One request, retried through stale instances. `$1` = a label, `$2` = the
# method, `$3` = the URL; the rest go to curl verbatim. Prints the final status
# code; the response headers land in $HDR and the body in $BODY.
request() {
  local label="$1" method="$2" url="$3"
  shift 3
  local attempt code
  for attempt in $(seq 1 "$ATTEMPTS"); do
    code=$(curl -sS -m 300 -u "$AUTH" -D "$HDR" -o "$BODY" -w '%{http_code}' \
      -X "$method" "$url" "$@" || echo 000)
    case "$code" in
      5?? | 000)
        echo "  $label answered $code (attempt $attempt/$ATTEMPTS); a stale instance may still be serving — retrying" >&2
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
  tr -d '\r' < "$HDR" | awk -v want="$1" '
    BEGIN { want = tolower(want) ":" }
    tolower($1) == want { sub(/^[^:]*:[ ]*/, ""); print }
  ' | tail -1
}

# The identifier the weak ETag carries (`W/"<uid>"` → `<uid>`).
etag_uid() { header_value etag | sed -e 's|^W/||' -e 's|"||g'; }

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

# ── 1. completion marker ─────────────────────────────────────────────────────

echo "==> seeding $BASE"
MARKER_SUBJECT="$(mf '.marker.subject')"
code=$(request "marker lookup" GET \
  "$BASE/ehr?subject_id=$MARKER_SUBJECT&subject_namespace=$SUBJECT_NS")
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
  find "$1" -name '*.adls' | while read -r f; do
    concept="$(basename "$f" | cut -d. -f2)"
    depth="${concept//[^-]/}"
    printf '%d\t%s\n' "${#depth}" "$f"
  done | LC_ALL=C sort -k1,1n -k2,2 | cut -f2-
}

lib_root="$ROOT_DIR/$(mf '.adl2.archetype_library.root')"
want_accepted="$(mf '.adl2.archetype_library.accepted')"
want_refused="$(mf '.adl2.archetype_library.refused')"
accepted=0
refused=0
while read -r adls; do
  code=$(request "archetype $(basename "$adls")" POST "$BASE/definition/template/adl2" \
    -H 'Content-Type: text/plain' --data-binary @"$adls")
  case "$code" in
    200 | 201 | 409) accepted=$((accepted + 1)) ;;
    422) refused=$((refused + 1)) ;;
    *) expect "archetype $(basename "$adls")" "$code" 201 409 422 ;;
  esac
done < <(adls_parents_first "$lib_root")

if [[ "$accepted" != "$want_accepted" ]] || [[ "$refused" != "$want_refused" ]]; then
  echo "::error::the ADL 2 archetype library loaded $accepted accepted / $refused refused; the manifest pins $want_accepted / $want_refused. Re-check the corpus and the manifest together." >&2
  exit 1
fi
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
    -H 'Content-Type: text/plain' --data-binary @"$file")
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
    -H 'Content-Type: text/plain' --data-binary @"$file")
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
  status_file="$SEED_DIR/$(printf '%s' "$entry" | jq -r '.status')"
  code=$(request "ehr lookup $subject" GET \
    "$BASE/ehr?subject_id=$subject&subject_namespace=$SUBJECT_NS")
  case "$code" in
    200)
      ehrs[index]=$(jq -r '.ehr_id.value' "$BODY")
      created[index]=no
      reused=$((reused + 1))
      ;;
    404)
      render "$status_file" "$WORK/status.json" "__SUBJECT_ID__=$subject"
      code=$(request "ehr create $subject" POST "$BASE/ehr" \
        -H 'Content-Type: application/json' --data-binary @"$WORK/status.json")
      expect "ehr create $subject" "$code" 201
      ehrs[index]=$(etag_uid)
      created[index]=yes
      ;;
    *) expect "ehr lookup $subject" "$code" 200 404 ;;
  esac
  : > "$WORK/comps.$index"
  index=$((index + 1))
done < <(jq -c '.ehrs[]' "$MANIFEST")
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
      -H 'Content-Type: application/json' -H 'Prefer: return=minimal' \
      --data-binary @"$body")
    expect "$label" "$code" 201
    ovid=$(etag_uid)
    printf '%s\n' "${ovid%%::*}" >> "$WORK/comps.$slot"
    [[ -n "$first" ]] || first="$ovid"
    compositions=$((compositions + 1))
  done
  [[ "$versions" -gt 0 ]] || return 0
  vo="${first%%::*}"
  ovid="$first"
  jq 'del(.uid)' "$body" > "$WORK/update.json"
  for _ in $(seq 1 "$versions"); do
    code=$(request "$label version" PUT "$BASE/ehr/$ehr/composition/$vo" \
      -H 'Content-Type: application/json' -H "If-Match: \"$ovid\"" \
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
    -H 'Content-Type: application/json' --data-binary @"$WORK/folder.json")
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
  ehr="${ehrs[$slot]}"
  code=$(request "ehr_status read $ehr" GET "$BASE/ehr/$ehr/ehr_status")
  expect "ehr_status read $ehr" "$code" 200
  current=$(etag_uid)
  render "$SEED_DIR/$final" "$WORK/status.json" "__SUBJECT_ID__=$subject"
  code=$(request "ehr_status update $ehr" PUT "$BASE/ehr/$ehr/ehr_status" \
    -H 'Content-Type: application/json' -H "If-Match: \"$current\"" \
    --data-binary @"$WORK/status.json")
  expect "ehr_status update $ehr" "$code" 200 204
  statuses=$((statuses + 1))
done < <(jq -c '.ehrs[]' "$MANIFEST")
echo "==> $statuses further EHR_STATUS versions"

# ── 9. demographics ──────────────────────────────────────────────────────────

persons=()
index=0
while read -r rel; do
  code=$(request "person $(basename "$rel")" POST "$BASE/demographic/person" \
    -H 'Content-Type: application/json' --data-binary @"$SEED_DIR/$rel")
  expect "person $(basename "$rel")" "$code" 201
  ovid=$(etag_uid)
  persons[index]="${ovid%%::*}"
  index=$((index + 1))
done < <(mf '.demographics.persons[]')

roles=0
while read -r entry; do
  file=$(printf '%s' "$entry" | jq -r '.file')
  performer=$(printf '%s' "$entry" | jq -r '.performer')
  render "$SEED_DIR/$file" "$WORK/role.json" "__PERFORMER__=${persons[$performer]}"
  code=$(request "role $(basename "$file")" POST "$BASE/demographic/role" \
    -H 'Content-Type: application/json' --data-binary @"$WORK/role.json")
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
    "$BASE/demographic/party_relationship" \
    -H 'Content-Type: application/json' --data-binary @"$WORK/relationship.json")
  expect "relationship $(basename "$file")" "$code" 201
  relationships=$((relationships + 1))
done < <(jq -c '.demographics.relationships[]' "$MANIFEST")
echo "==> ${#persons[@]} persons, $roles roles, $relationships party relationships"

# ── 10. stored AQL queries ───────────────────────────────────────────────────
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
    -H 'Content-Type: text/plain' --data-binary @"$aql")
  expect "query $name" "$code" 200 201 409

  parameters=$(printf '%s' "$entry" | jq -c '.parameters // empty')
  if [[ -n "$parameters" ]]; then
    printf '%s' "$parameters" \
      | jq --arg e "${ehrs[0]}" '{query_parameters: walk(if . == "__EHR_0__" then $e else . end)}' \
        > "$WORK/params.json"
    code=$(request "query run $name" POST "$BASE/query/$name" \
      -H 'Content-Type: application/json' --data-binary @"$WORK/params.json")
  else
    code=$(request "query run $name" GET "$BASE/query/$name")
  fi
  expect "query run $name" "$code" 200

  rows=$(jq '.rows | length' "$BODY")
  if [[ "$rows" -lt "$min_rows" ]]; then
    echo "::error::stored query $name returned $rows rows, below the $min_rows the manifest pins — the seeded data no longer feeds it." >&2
    exit 1
  fi
  echo "query $name -> $rows rows"
  queries=$((queries + 1))
done < <(jq -c '.stored_queries[]' "$MANIFEST")
echo "==> $queries stored AQL queries, each answering rows"

# ── 11. the completion marker ────────────────────────────────────────────────

render "$SEED_DIR/$(mf '.marker.status')" "$WORK/marker.json" \
  "__SUBJECT_ID__=$MARKER_SUBJECT"
code=$(request "marker create" POST "$BASE/ehr" \
  -H 'Content-Type: application/json' --data-binary @"$WORK/marker.json")
expect "marker create" "$code" 201 409

echo "==> seeded ${#ehrs[@]} EHRs, $compositions compositions (+$extra versions), $directories directories, ${#persons[@]} persons, $queries stored queries"
