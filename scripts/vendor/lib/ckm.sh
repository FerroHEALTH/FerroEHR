# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# shellcheck shell=bash
# The CKM archetype list both archetype vendor scripts read.
#
# The contract is the OpenAPI document at `/rest/v1/openapi.json` ("CKM REST
# API" 1.8.0, read 2026-11-12; `/rest/v1/swagger.json` answers 404). Its
# `/archetypes` `resource-state` parameter "Defaults to all active (i.e., all
# except REJECTED and DEPRECATED)", so the list asks for every state the
# contract enumerates, read from the contract itself: 1263 archetypes on
# 2026-11-12 against 946 by default.
#
# Pagination is `?size=M&offset=N` only; any other paging parameter is ignored
# and yields a 20-row first page, so a list of 20 rows or fewer is refused.
#
# No specification governs this file; it is FerroEHR's own design.

CKM_REST="https://ckm.openehr.org/ckm/rest/v1"

# Writes every archetype CKM lists, in every lifecycle state, to $1 as the
# REST list's JSON array, and the contract it read to $1.openapi.json.
ckm_list_archetypes() {
  local out=$1 state query="" published
  curl -fsS --proto '=https' "$CKM_REST/openapi.json" -o "$out.openapi.json"
  while read -r state; do
    query="$query&resource-state=$state"
  done < <(ckm_listed_states "$out")
  if [[ $query != *"=REJECTED"* || $query != *"=DEPRECATED"* ]]; then
    echo "::error::$CKM_REST/openapi.json no longer enumerates the archetype" \
         "resource states with REJECTED and DEPRECATED among them" >&2
    return 1
  fi
  curl -fsS --proto '=https' "$CKM_REST/archetypes?size=10000&offset=0$query" \
    -H "Accept: application/json" -o "$out"
  published=$(jq 'length' "$out")
  if [[ "$published" -le 20 ]]; then
    echo "::error::the list endpoint returned only $published rows: CKM ignored the" \
         "pagination parameters (use ?size=M&offset=N)" >&2
    return 1
  fi
  # A list without REJECTED or DEPRECATED rows is the default list: CKM dropped the filter.
  if [[ $(jq '[.[] | select(.status == "REJECTED" or .status == "DEPRECATED")] | length' "$out") -eq 0 ]]; then
    echo "::error::the list carries no REJECTED or DEPRECATED archetype: CKM ignored" \
         "the resource-state parameter" >&2
    return 1
  fi
}

# The states the list asked for, one per line, from the contract ckm_list_archetypes read.
ckm_listed_states() {
  jq -r '.paths["/archetypes"].get.parameters[]
         | select(.name == "resource-state") | .schema.items.enum[]' "$1.openapi.json"
}
