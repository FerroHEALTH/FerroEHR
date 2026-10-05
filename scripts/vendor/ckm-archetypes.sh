#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor the official openEHR CKM archetype library — the ADL 1.4 half of the
# two-dialect archetype corpus.
#
# Source: the public openEHR Clinical Knowledge Manager REST API
# (https://ckm.openehr.org/ckm/rest/v1). Each file is CKM's own export,
# vendored verbatim with provenance.
#
# WHICH DIALECT CKM SERVES (verified 2026-08-01 — do not re-guess):
#   * `GET /archetypes/{cid}/adl` -> ADL **1.4** text (`adl_version=1.4` in the
#     archetype header). This is the ONLY ADL CKM publishes.
#   * `GET /archetypes/{cid}/xml` -> the AM 1.4 ARCHETYPE XML of the same
#     archetype (opt-in here via --with-xml; roughly +40% bytes).
#   * `/rest/v1` publishes NO ADL 2: `/adl2`, `/adl14`, `/adl2.4`, `/opt2` and
#     `/source` all 404, and `?format=ADL2` / `?version=2` are silently
#     ignored (byte-identical 1.4 response). The legacy servlet
#     `GET /ckm/retrieveArchetype?cid-archetype=<cid>&format=ADL2` DOES return
#     an `.adls` file, but its header carries the `generated` flag: it is
#     Ocean's own 1.4->2 conversion, not an authored ADL 2 source (verified
#     2026-09-03). The ADL 2.4 corpus therefore comes from a DIFFERENT
#     official source —
#     `scripts/vendor/adl2-archetypes.sh` (openEHR/adl-archetypes).
#     Never present a CKM export as ADL 2, and never fill the ADL 2 side by
#     running our own 1.4->2 converter over CKM output: that would test the
#     converter against itself.
#
# CKM REST PAGINATION GOTCHA: the list endpoints page with `?size=M&offset=N`
# (the Swagger 2.0 document at `/rest/v1/swagger.json`, verified on the wire
# 2026-09-03: `?size=2&offset=2` shifts the window, `?page=5&size=2` does not).
# `page`, `limit`, `pageSize`, `maxResults`, `count` and `rows` are all silently
# IGNORED and you get a 20-row first page — which reads exactly like "CKM only
# publishes 20 archetypes". Always fetch with size/offset and assert the count
# grew.
#
# Usage:
#   scripts/vendor/ckm-archetypes.sh                # ADL 1.4 texts
#   scripts/vendor/ckm-archetypes.sh --with-xml     # + the AM 1.4 XML twin
#   CKM_JOBS=8 scripts/vendor/ckm-archetypes.sh     # parallel (default 4)
set -Eeuo pipefail

CKM="https://ckm.openehr.org/ckm/rest/v1"
OUT="corpus/archetypes/ckm"
JOBS="${CKM_JOBS:-4}"

# ── re-entrant single fetch (the xargs worker; not a user-facing mode) ────
if [[ "${1:-}" == "--fetch-one" ]]; then
  cid=$2
  dest=$3
  fmt=$4 # adl | xml
  for attempt in 1 2 3; do
    if curl -fsS --max-time 240 "$CKM/archetypes/$cid/$fmt" -o "$dest"; then
      if grep -qi "archetype" <<< "$(head -c 2048 "$dest")"; then
        echo "OK   $cid $dest"
        exit 0
      fi
      rm -f "$dest"
      echo "BAD  $cid $dest (response is not an archetype)"
      exit 0
    fi
    sleep $((attempt * 2))
  done
  rm -f "$dest"
  echo "FAIL $cid $dest"
  exit 0
fi

WITH_XML=0
[[ "${1:-}" == "--with-xml" ]] && WITH_XML=1

ADL_DIR="$OUT/adl14"
XML_DIR="$OUT/xml"
mkdir -p "$ADL_DIR"
[[ $WITH_XML == 1 ]] && mkdir -p "$XML_DIR"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
STAMP=$(date -u +%Y-%m-%dT%H:%M:%SZ)

echo "==> listing the full CKM archetype library (size/offset pagination)"
curl -fsS "$CKM/archetypes?size=10000&offset=0" -H "Accept: application/json" \
  -o "$WORK/archetypes.json"

# The list is turned into two job files and a provenance TSV with jq. The
# duplicate-HRID suffix (`__2`, `__3`) is the one piece of real logic: CKM can
# publish the same resourceMainId twice, and two rows writing one file name would
# silently vendor only the last. `group_by` + the within-group index reproduces
# the previous counter exactly, on the same `resourceMainId` sort order.
published=$(jq 'length' "$WORK/archetypes.json")
if [[ "$published" -le 20 ]]; then
  echo "::error::the list endpoint returned only $published rows — CKM ignored the" \
       "pagination parameters (use ?size=M&offset=N)" >&2
  exit 1
fi

jq '
  # Stable name per archetype: the HRID, suffixed only for a repeat of it.
  [ .[] | { cid, hrid: .resourceMainId, display: .resourceMainDisplayName,
            status, modified: .modificationTime, rev: (.revisionLatest // "") } ]
  | sort_by(.hrid)
  | group_by(.hrid)
  | map(to_entries | map(.value + { n: (.key + 1) }))
  | flatten
  | map(. + { name: (if .n == 1 then .hrid else .hrid + "__" + (.n | tostring) end) })
  | map({ cid, name, display: (.display | gsub("\\|"; "/")), status, modified,
          revision: (.rev | tostring) })
  ' "$WORK/archetypes.json" > "$WORK/rows.json"

jq -r --arg dir "$ADL_DIR" '.[] | "\(.cid) \($dir)/\(.name).adl"' \
  "$WORK/rows.json" > "$WORK/jobs_adl.txt"
jq -r --arg dir "$XML_DIR" '.[] | "\(.cid) \($dir)/\(.name).xml"' \
  "$WORK/rows.json" > "$WORK/jobs_xml.txt"
printf '==> %s archetypes published by CKM\n' "$published"

echo "==> fetching ADL 1.4 texts (jobs=$JOBS)"
find "$ADL_DIR" -name '*.adl' -delete
# shellcheck disable=SC2016 # $0/$1/$2 are the inner bash -c positionals, not this shell's
xargs -P "$JOBS" -n 2 bash -c 'bash "$0" --fetch-one "$1" "$2" adl' "$0" \
  < "$WORK/jobs_adl.txt" | tee "$WORK/adl.log"

if [[ $WITH_XML == 1 ]]; then
  echo "==> fetching AM 1.4 archetype XML (jobs=$JOBS)"
  find "$XML_DIR" -name '*.xml' -delete
  # shellcheck disable=SC2016 # $0/$1/$2 are the inner bash -c positionals, not this shell's
  xargs -P "$JOBS" -n 2 bash -c 'bash "$0" --fetch-one "$1" "$2" xml' "$0" \
    < "$WORK/jobs_xml.txt" | tee "$WORK/xml.log"
else
  : > "$WORK/xml.log"
fi

# Join the ADL fetch verdicts onto the rows ONCE, so the inventory counts, the
# two aggregate tables and the summary line cannot disagree. A per-file failure
# is RECORDED (CKM answers 404 for private incubator resources), never fatal.
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
' < "$WORK/adl.log" > "$WORK/classified.json"
XML_EXPORTS=$(grep -cE '^(OK|BAD|FAIL) ' "$WORK/xml.log" || true)

# The static prose stays in shell `echo`s: it carries apostrophes and braces
# that would have to fight the quoting of a jq program, and it is not data.
{
  echo "# CKM archetype library (ADL 1.4) — provenance"
  echo
  echo "Every archetype the official openEHR CKM (\`$CKM\`) publishes, exported"
  echo "by CKM itself and vendored verbatim by"
  echo "\`scripts/vendor/ckm-archetypes.sh\` on $STAMP."
  echo
  echo "## Dialect"
  echo
  echo "\`adl14/\` holds CKM's \`GET /archetypes/{cid}/adl\` response — **ADL 1.4**"
  echo "text (\`adl_version=1.4\`). \`/rest/v1\` publishes NO ADL 2 (\`/adl2\`,"
  echo "\`/adl14\`, \`/opt2\` 404; \`?format=ADL2\` is ignored); the legacy servlet's"
  echo "\`format=ADL2\` export is Ocean's own GENERATED 1.4->2 conversion, so the **ADL 2.4 half of the corpus comes from"
  echo "\`scripts/vendor/adl2-archetypes.sh\`** (openEHR/adl-archetypes). A CKM"
  echo "export is never labelled ADL 2, and the ADL 2 side is never produced by"
  echo "running our own 1.4->2 converter over these files — that would test the"
  echo "converter against itself."
  echo
  if [[ $WITH_XML == 1 ]]; then
    echo "\`xml/\` holds the AM 1.4 ARCHETYPE XML twin of the same $XML_EXPORTS exports"
    echo "(\`GET /archetypes/{cid}/xml\`), for the XML codec's read path."
  else
    echo "The AM 1.4 ARCHETYPE XML twin (\`GET /archetypes/{cid}/xml\`) is NOT"
    echo "vendored here; re-run with \`--with-xml\` to add it."
  fi
  echo
  echo "## Exercised, with adjudicated refusals"
  echo
  echo "The pack is parsed 100% by"
  echo "\`crates/openehr-adl/tests/it/ckm_archetype_packs.rs\`"
  echo "(\`ckm_adl14_pack_parses\`). A file the conformant reader REFUSES is listed"
  echo "in that gate's \`ADJUDICATED_CKM_ADL14\` table with the syntax code the"
  echo "refusal must carry plus the spec ground for it, and the gate asserts the"
  echo "refusal HAPPENS and carries that code — a refusal is a negative test, not"
  echo "a skip. Never delete a refused file: that drops the negative case"
  echo "(\`.claude/rules/vendored-corpora.md\`, \`.claude/rules/testing.md\`)."
  echo
  echo "## Licensing"
  echo
  echo "CKM publishes no repository-level license; each archetype carries its"
  echo "own \`description\` > \`licence\` metadata (predominantly CC-BY-SA 3.0"
  echo "where stated — see the individual file). Vendored verbatim, so the"
  echo "authorship and licence metadata ride along in every file; root"
  echo "reference copy: \`LICENSE-CC-BY-SA-3.0\`."
  echo
  echo "## Inventory"
  echo
  jq -r '
    # Both aggregate tables sort by descending count, then by name.
    def tally(f): (.ok | group_by(f) | map({ key: (.[0] | f), n: length })
                   | sort_by([-.n, .key]));
    [ "- published by CKM: **\(.published)**",
      "- vendored: **\(.ok | length)**",
      "- unreachable: **\(.bad | length)**",
      "",
      "| RM class | count |",
      "|---|---|" ]
    + (tally(.name | split(".") | .[0]) | map("| \(.key) | \(.n) |"))
    + [ "", "| status | count |", "|---|---|" ]
    + (tally(.status) | map("| \(.key) | \(.n) |"))
    + [ "" ]
    + (if (.bad | length) > 0 then
        [ "## Unreachable (recorded, not skipped)",
          "",
          "CKM answers 404 for resources held in a private incubator; they are",
          "only exportable by a signed-in account with access.",
          "",
          "| cid | archetype | status |",
          "|---|---|---|" ]
        + (.bad | map("| \(.cid) | \(.name) | \(.status) |"))
        + [ "" ]
      else [] end)
    + [ "## Vendored",
        "",
        "| cid | archetype | display name | status | modified | revision |",
        "|---|---|---|---|---|---|" ]
    + (.ok | map("| \(.cid) | `\(.name)` | \(.display) | \(.status) | \(.modified) | \(.revision) |"))
    | .[]
  ' "$WORK/classified.json"
} > "$OUT/PROVENANCE.md"

jq -r --arg prov "$OUT/PROVENANCE.md" '
  "==> \(.ok | length) ADL 1.4 archetypes vendored, \(.bad | length) unreachable → \($prov)",
  (if (.bad | length) > 0 then "    unreachable: \(.bad | map(.cid) | join(", "))" else empty end)
' "$WORK/classified.json"
