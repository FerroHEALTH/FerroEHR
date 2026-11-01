#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor the official openEHR CKM template library as OPT 1.4 XML.
#
# Source: the public openEHR Clinical Knowledge Manager REST API
# (https://ckm.openehr.org/ckm/rest/v1). Every file is CKM's own
# Operational Template export, vendored verbatim with provenance.
#
# TWO PACKS, one script:
#
#   * the CURATED journey pack -> corpus/templates/ckm/<slug>.opt
#     Hand-picked, all COMPOSITION-rooted, each with the clinical role it was
#     chosen for. The slugs are REFERENCED BY NAME from this repository's tests
#     and benches, scripts/sandbox/reseed.sh and
#     scripts/generate-ckm-examples.sh, and the conformance instrument's
#     measured workloads carry copies under the same slugs (Veredictum's
#     perf/payloads.yaml), so never rename or drop one.
#
#   * the FULL library -> corpus/templates/ckm/full/<slug>.opt
#     Every template CKM publishes (slug derived from the display name),
#     for breadth gates over the OPT 1.4 reader / WebTemplate builder.
#
# CKM REST PAGINATION GOTCHA (cost an afternoon once — do not relearn it):
# the list endpoints page with `?size=M&offset=N` (the Swagger 2.0 document at
# `/rest/v1/swagger.json`, verified on the wire 2026-09-03). `page`, `limit`,
# `pageSize`, `maxResults`, `count`, `rows` are all silently IGNORED and you
# get a 20-row first page, which reads exactly like "CKM only publishes 20
# templates". Always fetch with size/offset, and assert the count grew.
#
# Some CKM resources live in a private incubator and 404 without an account;
# those are recorded as unreachable in the provenance file rather than
# silently skipped.
#
# Usage:
#   scripts/vendor/ckm-templates.sh              # curated pack + full library
#   scripts/vendor/ckm-templates.sh --curated    # curated pack only
#   scripts/vendor/ckm-templates.sh --full       # full library only
#   CKM_JOBS=8 scripts/vendor/ckm-templates.sh   # parallel fetches (default 4)
#
# Example skeletons (`*.example.json`) for the curated pack are generated
# separately against the composed SUT by scripts/generate-ckm-examples.sh.
set -Eeuo pipefail

CKM="https://ckm.openehr.org/ckm/rest/v1"
OUT="corpus/templates/ckm"
FULL="$OUT/full"
JOBS="${CKM_JOBS:-4}"

# ── re-entrant single fetch (the xargs worker; not a user-facing mode) ────
if [[ "${1:-}" == "--fetch-one" ]]; then
  cid=$2
  dest=$3
  for attempt in 1 2 3; do
    if curl -fsS --max-time 240 \
        "$CKM/templates/$cid/opt" -H "Accept: application/xml" -o "$dest"; then
      if grep -q "<template" <<<"$(head -c 2048 "$dest")"; then
        echo "OK   $cid $dest"
        exit 0
      fi
      rm -f "$dest"
      echo "BAD  $cid $dest (response is not an OPT)"
      exit 0
    fi
    sleep $((attempt * 2))
  done
  rm -f "$dest"
  echo "FAIL $cid $dest"
  exit 0
fi

MODE="${1:-both}"
case "$MODE" in
  both | --both) MODE=both ;;
  --curated) MODE=curated ;;
  --full) MODE=full ;;
  *)
    echo "usage: $0 [--curated|--full]" >&2
    exit 2
    ;;
esac

# cid | slug | journey role — every entry COMPOSITION-rooted (committable
# as a composition; ENTRY/CLUSTER-rooted CKM "item" templates cannot carry
# a commit and are deliberately absent).
SET=(
  # ── monitoring streams ────────────────────────────────────────────────
  "1013.26.380|vital-signs|vitals_round (full observation round)"
  # NOTE: 1013.26.61 (ODL Report Vital Signs) is EXCLUDED: its OPT carries an
  # AOM defect (an assumed_value outside its constrained code list — AM 1.4
  # Assumed_value_valid), rejected by conformant AOM validation.
  # ── laboratory / imaging pipelines ────────────────────────────────────
  "1013.26.408|generic-lab-test-result|lab_pipeline (result contribution)"
  "1013.26.2|ereferral|lab_pipeline / imaging_pipeline (order)"
  "1013.26.386|ccta-report|imaging_pipeline (report)"
  # ── medication (the eMAR loop) ────────────────────────────────────────
  "1013.26.80|eprescription-fhir|medication_round (order + administrations)"
  "1013.26.357|medicines-list|medicines_reconciliation (ward-seeded, updated)"
  # ── encounter documents & summaries ───────────────────────────────────
  "1013.26.191|gp-data-set|correction target (ward-seeded, amended)"
  "1013.26.376|international-patient-summary|admission / discharge summary"
  "1013.26.360|problem-list|admission (problem list)"
  # ── specialist & registry reporting ───────────────────────────────────
  "1013.26.199|bc-breast-cancer-report|specialist_report (cancer synoptic report)"
  "1013.26.40|treat-registry-report|registry_submission (registry export)"
  # ── public-health surveillance ────────────────────────────────────────
  "1013.26.377|sars-event-notification|public_health_notification (statutory notification)"
  "1013.26.282|covid19-infection-report|public_health_notification (confirmed-case follow-up)"
  "1013.26.988|poisoning-case-investigation|case_investigation"
  "1013.26.980|diphtheria-case-investigation|case_investigation"
  # ── scale probe: the largest OPT the CKM publishes (5.2 MB, #1462) ────
  "1013.26.977|congenital-syphilis-case-investigation|case_investigation (largest published form — the large-payload scale probe)"
)

mkdir -p "$OUT"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

STAMP=$(date -u +%Y-%m-%dT%H:%M:%SZ)

# ── the curated journey pack ──────────────────────────────────────────────
PROV="$OUT/PROVENANCE.md"
if [[ "$MODE" != full ]]; then
  {
    echo "# CKM template pack — provenance"
    echo
    echo "Vendored from the official openEHR CKM (\`$CKM\`) by"
    echo "\`scripts/vendor/ckm-templates.sh\` on $STAMP."
    echo "Each file is CKM's own OPT export for the cited template, verbatim."
    echo "Example skeletons (\`*.example.json\`) are generated once against the"
    echo "composed SUT by \`scripts/generate-ckm-examples.sh\` and committed"
    echo "(byte-identical payload ground for every SUT; never fetched at run"
    echo "time). The conformance instrument's measured workloads carry their"
    echo "own copies of this pack under the same slugs (Veredictum's"
    echo "\`perf/payloads.yaml\`)."
    echo
    echo "The **curated pack** below is referenced by slug from this"
    echo "repository's tests and benches, the sandbox reseed, the example"
    echo "generator and Veredictum's \`perf/payloads.yaml\` — the slugs are a"
    echo "stable contract. The **full library** is a separate pack under"
    echo "\`full/\` with its own provenance file."
    echo
    echo "## Licensing"
    echo
    echo "CKM publishes no repository-level license; each OPT embeds its source"
    echo "archetypes' \`licence\` metadata, and the corpus is **mixed**: a count"
    echo "over this directory on 2026-08-12 found **114 files carrying CC-BY-SA"
    echo "4.0 and 10 carrying CC-BY-SA 3.0**. Read the individual file; its own"
    echo "metadata is the authority. Vendored verbatim, so authorship and"
    echo "licence metadata ride along; root reference copies:"
    echo "\`LICENSE-CC-BY-SA-3.0\` and \`LICENSE-CC-BY-SA-4.0\`, and \`REUSE.toml\`"
    echo "declares this tree as \`CC-BY-SA-3.0 AND CC-BY-SA-4.0\`."
    echo
    echo "| cid | slug | display name | status | modified | journey role |"
    echo "|---|---|---|---|---|---|"
  } > "$PROV"

  for entry in "${SET[@]}"; do
    IFS='|' read -r cid slug role <<< "$entry"
    echo "==> curated $cid ($slug)"
    meta=$(curl -fsS "$CKM/templates/$cid" -H "Accept: application/json")
    # one field per call: display names contain spaces AND pipes, so a
    # word-split read of a combined line is not safe here
    name=$(printf '%s' "$meta" | jq -r '.resourceMainDisplayName | gsub("\\|"; "/")')
    status=$(printf '%s' "$meta" | jq -r '.status')
    modified=$(printf '%s' "$meta" | jq -r '.modificationTime')
    bash "$0" --fetch-one "$cid" "$OUT/$slug.opt" | tee -a "$WORK/curated.log"
    grep -q "^OK   $cid " "$WORK/curated.log" || {
      echo "::error::$cid ($slug) did not yield an OPT — the curated pack is a contract" >&2
      exit 1
    }
    echo "| $cid | $slug | $name | $status | $modified | $role |" >> "$PROV"
  done
  {
    echo
    echo "## Adjudicated boundaries"
    echo
    echo "Two EHDS Annex I priority categories have no template in this pack, and the"
    echo "gap is a boundary rather than an omission (#3206): CKM publishes no operational"
    echo "template for **electronic dispensations** (Annex I 3) or **discharge reports**"
    echo "(Annex I 6) as such at the pinned revision, and this pack vendors CKM content"
    echo "verbatim, never a template authored here to fill a category. The"
    echo "international-patient-summary template covers admission and discharge"
    echo "summaries as a COMPOSITION shape, not the discharge-report category the"
    echo "Regulation names. The question reopens when a CKM template for either category"
    echo "is published, or when the Article 36 implementing acts fix the exchange format"
    echo "those categories must be provided in."
  } >> "$PROV"
  echo "==> curated pack: $(grep -c '^OK' "$WORK/curated.log") OPTs → $OUT"
fi

# ── the full library ─────────────────────────────────────────────────────
if [[ "$MODE" != curated ]]; then
  mkdir -p "$FULL"
  echo "==> listing the full CKM template library (size/offset pagination)"
  curl -fsS "$CKM/templates?size=10000&offset=0" -H "Accept: application/json" \
    -o "$WORK/templates.json"

  # Slugs come from CKM display names, so they collide: the counter suffixes the
  # second and later occurrences of a base. The 20-row floor is the CKM paging
  # trap — every parameter other than size/offset is silently ignored and yields a
  # 20-row first page that reads like the whole library.
  jq '
    if length <= 20 then
      error("::error::the list endpoint returned only \(length) rows — CKM ignored the pagination parameters (use ?size=M&offset=N)")
    else . end
    | sort_by(.cid)
    | reduce .[] as $t ({ seen: {}, rows: [] };
        ($t.resourceMainDisplayName | ascii_downcase | gsub("[^a-z0-9]+"; "-")
          | sub("^-+"; "") | sub("-+$"; "") | .[0:80]) as $stripped
        | (if $stripped == "" then "template" else $stripped end) as $base
        | ((.seen[$base] // 0) + 1) as $n
        | .seen[$base] = $n
        | .rows += [{
            cid: $t.cid,
            slug: (if $n > 1 then "\($base)-\($n)" else $base end),
            name: ($t.resourceMainDisplayName | gsub("\\|"; "/")),
            status: $t.status,
            modified: $t.modificationTime,
            version: ($t.versionAssetLatest // "" | tostring)
          }])
    | .rows
  ' "$WORK/templates.json" > "$WORK/rows.json"

  jq -r --arg out_dir "$FULL" '.[] | "\(.cid) \($out_dir)/\(.slug).opt"' \
    "$WORK/rows.json" > "$WORK/jobs.txt"
  echo "==> $(jq 'length' "$WORK/rows.json") templates published by CKM"

  # fetch everything; a per-file failure is recorded, never fatal (private
  # incubator resources 404 without a CKM account)
  find "$FULL" -name '*.opt' -delete
  xargs -P "$JOBS" -n 2 bash "$0" --fetch-one < "$WORK/jobs.txt" \
    | tee "$WORK/full.log"

  # Join the fetch log's per-cid verdict onto the rows ONCE, into `classified`,
  # so the provenance file, its tables and the summary line below cannot
  # disagree about the counts. A per-file failure is RECORDED rather than fatal
  # (CKM answers 404 for private incubator resources) — hence two tables.
  jq -Rn --slurpfile rows "$WORK/rows.json" '
    ( reduce inputs as $line ({};
        ([$line | splits("\\s+")] | map(select(length > 0))) as $p
        | if ($p | length) >= 2 and ($p[0] | IN("OK", "BAD", "FAIL"))
          then .[$p[1]] = $p[0] else . end)
    ) as $outcome
    | ($rows | first) as $all
    | { published: ($all | length),
        ok: ($all | map(select($outcome[.cid] == "OK"))),
        bad: ($all | map(select($outcome[.cid] | IN("BAD", "FAIL")))) }
  ' < "$WORK/full.log" > "$WORK/classified.json"

  # The static prose stays in shell `echo`s, matching the curated pack above.
  # It also keeps apostrophes out of the jq programs, where they would have to
  # fight the surrounding single quotes.
  {
    echo "# CKM template library (full pack) — provenance"
    echo
    echo "Every template the official openEHR CKM (\`$CKM\`) publishes, exported"
    echo "by CKM itself as an Operational Template and vendored verbatim by"
    echo "\`scripts/vendor/ckm-templates.sh\` on $STAMP."
    echo
    echo "This is the BREADTH pack: real-world OPT 1.4 shapes for the reader /"
    echo "WebTemplate builder gates. The curated hospital-simulation journey"
    echo "pack is the parent directory (its own \`PROVENANCE.md\`); the slugs here"
    echo "are derived from CKM display names and are NOT a naming contract."
    echo
    echo "## Licensing"
    echo
    echo "CKM publishes no repository-level license; each OPT embeds its source"
    echo "archetypes' \`licence\` metadata (predominantly CC-BY-SA 3.0 where"
    echo "stated — see the individual file). Vendored verbatim, so authorship"
    echo "and licence metadata ride along; root reference copy:"
    echo "\`LICENSE-CC-BY-SA-3.0\`."
    echo
    jq -r '
      [ "- published by CKM: **\(.published)**",
        "- vendored: **\(.ok | length)**",
        "- unreachable: **\(.bad | length)**",
        "" ]
      + (if (.bad | length) > 0 then
          [ "## Unreachable (recorded, not skipped)",
            "",
            "CKM answers 404 for resources held in a private incubator; they are",
            "only exportable by a signed-in account with access.",
            "",
            "| cid | display name | status |",
            "|---|---|---|" ]
          + (.bad | map("| \(.cid) | \(.name) | \(.status) |"))
          + [ "" ]
        else [] end)
      + [ "## Vendored",
          "",
          "| cid | file | display name | status | modified | asset version |",
          "|---|---|---|---|---|---|" ]
      + (.ok | map("| \(.cid) | `\(.slug).opt` | \(.name) | \(.status) | \(.modified) | \(.version) |"))
      | .[]
    ' "$WORK/classified.json"
  } > "$FULL/PROVENANCE.md"

  jq -r --arg prov "$FULL/PROVENANCE.md" '
    "==> full library: \(.ok | length) vendored, \(.bad | length) unreachable → \($prov)",
    (if (.bad | length) > 0 then "    unreachable: \(.bad | map(.cid) | join(", "))" else empty end)
  ' "$WORK/classified.json"
fi
