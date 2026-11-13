#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor CKM's GENERATED ADL 2 export of every archetype it publishes (#4973).
#
# Source: the legacy CKM servlet
#   GET https://ckm.openehr.org/ckm/retrieveArchetype?cid-archetype=<cid>&format=ADL2
# which answers an `.adls` whose header carries `generated`: CKM's own 1.4->2
# conversion (Archie's converter since CKM Release 1.22.0, 2026-11-04), never
# an authored ADL 2 source. This pack measures that conversion and the CKM
# sources. It is never the ADL 2 corpus (that is `adl2-archetypes.sh`) and
# never an oracle for our own converter (`.claude/rules/vendored-corpora.md`).
#
# The archetype list is the REST API's in every lifecycle state, REJECTED and
# DEPRECATED included (scripts/vendor/lib/ckm.sh), so the file names match
# `ckm-archetypes.sh` one for one.
# A reply that is not generated ADL 2 is recorded as unexported, never kept:
# NOTFOUND (404, a resource not public, such as one in INITIAL or TEAMREVIEW
# state), EMPTY (200 with no body: CKM's conversion produced nothing), BAD
# (another body) or FAIL (no answer after three tries).
#
# Polite by default: two workers, each pausing CKM_PAUSE seconds after a fetch.
#
# Usage:
#   scripts/vendor/ckm-adl2-export.sh
#   CKM_JOBS=2 CKM_PAUSE=0.5 scripts/vendor/ckm-adl2-export.sh
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=scripts/vendor/lib/ckm.sh
. "$ROOT/scripts/vendor/lib/ckm.sh"
CKM_SERVLET="https://ckm.openehr.org/ckm/retrieveArchetype"
OUT="corpus/archetypes/ckm/adl2-generated"
JOBS="${CKM_JOBS:-2}"
PAUSE="${CKM_PAUSE:-0.5}"

# ── re-entrant single fetch (the xargs worker; not a user-facing mode) ────
if [[ "${1:-}" == "--fetch-one" ]]; then
  cid=$2
  dest=$3
  for attempt in 1 2 3; do
    status=$(curl -sS --proto '=https' --max-time 240 -w '%{http_code}' \
      "$CKM_SERVLET?cid-archetype=$cid&format=ADL2" -o "$dest" || echo 000)
    sleep "$PAUSE"
    if [[ $status == 404 ]]; then
      rm -f "$dest"
      echo "NOTFOUND $cid $dest"
      exit 0
    fi
    if [[ $status == 200 && ! -s $dest ]]; then
      rm -f "$dest"
      echo "EMPTY $cid $dest"
      exit 0
    fi
    if [[ $status == 200 ]]; then
      header=$(head -c 512 "$dest")
      if [[ $header =~ ^archetype\ \(adl_version=2[^\)]*generated ]]; then
        echo "OK   $cid $dest"
        exit 0
      fi
      rm -f "$dest"
      echo "BAD  $cid $dest (not a generated ADL 2 archetype)"
      exit 0
    fi
    sleep $((attempt * 2))
  done
  rm -f "$dest"
  echo "FAIL $cid $dest"
  exit 0
fi

mkdir -p "$OUT"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
STAMP=$(date -u +%Y-%m-%dT%H:%M:%SZ)

echo "==> listing the CKM archetype library, every lifecycle state"
ckm_list_archetypes "$WORK/archetypes.json"
published=$(jq 'length' "$WORK/archetypes.json")

# One file per archetype, named by its HRID; a repeated HRID gets `__2`, `__3`
# on the resourceMainId sort order, as `ckm-archetypes.sh` names them.
jq '
  [ .[] | { cid, hrid: .resourceMainId } ]
  | sort_by(.hrid)
  | group_by(.hrid)
  | map(to_entries | map(.value + { n: (.key + 1) }))
  | flatten
  | map(. + { name: (if .n == 1 then .hrid else .hrid + "__" + (.n | tostring) end) })
  ' "$WORK/archetypes.json" > "$WORK/rows.json"
jq -r --arg dir "$OUT" '.[] | "\(.cid) \($dir)/\(.name).adls"' \
  "$WORK/rows.json" > "$WORK/jobs.txt"
printf '==> %s archetypes listed by CKM (jobs=%s, pause=%ss)\n' "$published" "$JOBS" "$PAUSE"

find "$OUT" -name '*.adls' -delete
# shellcheck disable=SC2016 # $0/$1/$2 are the inner bash -c positionals, not this shell's
xargs -P "$JOBS" -n 2 bash -c 'bash "$0" --fetch-one "$1" "$2"' "$0" \
  < "$WORK/jobs.txt" | tee "$WORK/fetch.log"

exported=$(grep -c '^OK ' "$WORK/fetch.log" || true)
(cd "$OUT" && find . -name '*.adls' -print0 | sort -z | xargs -0 shasum -a 256 | sed 's#  \./#  #') \
  > "$OUT/SHA256SUMS"

{
  echo "# CKM generated ADL 2 export: provenance"
  echo
  echo "Fetched by \`scripts/vendor/ckm-adl2-export.sh\` on $STAMP from"
  echo "\`$CKM_SERVLET?cid-archetype=<cid>&format=ADL2\`, one request per archetype"
  echo "the REST list (\`$CKM_REST/archetypes\`, every \`resource-state\` its"
  echo "contract \`$CKM_REST/openapi.json\` enumerates) names: $published listed,"
  echo "$exported exported. CKM ran Release 1.22.0 with Archie's \`adl_2.4_support\`"
  echo "converter (announced on https://discourse.openehr.org/t/adl2-generation-appears-to-be-incorrect/17448,"
  echo "post 13, 2026-11-04)."
  echo
  echo "## What this is"
  echo
  echo "Every file is CKM's GENERATED 1.4->2 conversion: its header carries"
  echo "\`generated\`. It is vendored verbatim to measure that conversion and the"
  echo "CKM sources (#4512, #4973). It is not authored ADL 2: the ADL 2 corpus is"
  echo "\`scripts/vendor/adl2-archetypes.sh\`, and these files are never an oracle"
  echo "for FerroEHR's own converter."
  echo
  echo "## Pin"
  echo
  echo "The servlet serves the current conversion and takes no version, so the"
  echo "pin is the fetch date above and the sha256 of every file in"
  echo "\`SHA256SUMS\`. A re-run that changes a digest is a change in CKM's"
  echo "conversion or in the archetype."
  echo
  echo "## Exercised"
  echo
  echo "\`crates/openehr-adl/tests/it/ckm_adl2_generated.rs\` parses every file and"
  echo "validates it as an ADL 2 upload is validated, parents first; each refusal"
  echo "is adjudicated there with its rule code and whether the conversion or the"
  echo "source causes it."
  echo
  echo "## Licensing"
  echo
  echo "CKM publishes no repository-level licence; each archetype carries its own"
  echo "\`description\` licence metadata (predominantly CC-BY-SA 3.0), which rides"
  echo "along verbatim; reference copy: \`LICENSE-CC-BY-SA-3.0\` at the repository root."
  echo
  echo "## Not exported"
  echo
  echo "NOTFOUND is a 404 (a resource CKM does not make public, such as one in"
  echo "INITIAL or TEAMREVIEW state); EMPTY is a 200 with no body, CKM's"
  echo "conversion producing nothing; BAD is a body that is not generated ADL 2;"
  echo "FAIL is no answer after three tries."
  echo
  if grep -qE '^(BAD|EMPTY|FAIL|NOTFOUND) ' "$WORK/fetch.log"; then
    echo "| cid | file | outcome |"
    echo "|---|---|---|"
    grep -E '^(BAD|EMPTY|FAIL|NOTFOUND) ' "$WORK/fetch.log" | sort -k2 \
      | awk '{ f=$3; sub(".*/", "", f); printf "| %s | %s | %s |\n", $2, f, $1 }'
  else
    echo "None: every listed archetype was exported."
  fi
} > "$OUT/PROVENANCE.md"

echo "==> $exported of $published exported into $OUT"
