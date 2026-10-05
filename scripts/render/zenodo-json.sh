#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Render `.zenodo.json` from `CITATION.cff` (#2210).
#
# Zenodo's own rule makes this file necessary and dangerous in one sentence:
# "If your repository contains both a .zenodo.json and a CITATION.cff file,
# Zenodo will only use the .zenodo.json metadata. The CITATION.cff will be
# COMPLETELY IGNORED for the GitHub release archiving."
# (https://help.zenodo.org/docs/github/describe-software/zenodo-json/)
#
# A hand-written copy would therefore silently disable the file CI already
# guards, and the two would drift with no signal — the deposit saying one thing
# while the citation box says another, under a DOI nobody can correct. So this
# file is GENERATED: CITATION.cff stays the single source for every fact both
# carry, and `citation-guard` runs this script in --check mode.
#
# The output is the FLAT LEGACY DEPOSIT shape the help page documents — not
# the InvenioRDM record shape this script previously emitted. Measured
# first-hand on this repository's own first GitHub-archived deposit
# (10.5281/zenodo.21940280, v3.17.6, read 2026-08-15): the record-shape file
# was IGNORED entirely and the deposit fell back to GitHub's raw repo
# metadata (title "FerroHEALTH/FerroEHR: v3.17.6", the full 29-name
# contributor list as creators), refuting the earlier claim that the
# integration accepts the record shape. The help page's complete example
# (read 2026-08-15) is the authority for the field spellings used below:
# flat top-level keys, `license` as a lowercase id, camelCase
# `related_identifiers[].relation`, creators as {name, orcid, affiliation}.
#
# Usage:
#   scripts/render/zenodo-json.sh            # write .zenodo.json
#   scripts/render/zenodo-json.sh --check    # fail if the committed file is stale
set -euo pipefail
cd "$(dirname "$0")/../.." || exit 1

command -v jq >/dev/null || { echo "jq is required" >&2; exit 1; }

CFF="CITATION.cff"
OUT=".zenodo.json"
MODE="${1:-write}"

# CITATION.cff is YAML, but every field read here is a flat quoted scalar, a
# folded block, or a simple list — so sed/awk read it and jq builds the JSON.
# No YAML parser, and no second language embedded in this script.
cff_scalar() {
  local v
  v="$(sed -nE "s/^$1:[[:space:]]*(.*)$/\1/p" "$CFF" | head -1)"
  v="${v%\"}"; v="${v#\"}"
  [[ -n "$v" ]] || { echo "$CFF has no \`$1\`" >&2; exit 1; }
  printf '%s' "$v"
}

# A folded block scalar (`key: >-`), rejoined onto one line.
cff_block() {
  awk -v key="$1" '
    $0 ~ "^" key ":[[:space:]]*>-[[:space:]]*$" { grab = 1; next }
    grab && /^[[:space:]]+/ { sub(/^[[:space:]]+/, ""); printf "%s%s", sep, $0; sep = " "; next }
    grab { exit }
  ' "$CFF"
}

# A plain list of scalars (`key:` then `  - value` lines), as a JSON array.
cff_list() {
  awk -v key="$1" '
    $0 ~ "^" key ":[[:space:]]*$" { grab = 1; next }
    grab && /^[[:space:]]+- / { sub(/^[[:space:]]+- /, ""); gsub(/^"|"$/, ""); print; next }
    grab { exit }
  ' "$CFF" | jq -R . | jq -s .
}

# `authors` → legacy-deposit `creators` ({name, orcid, affiliation}). The
# ORCID is emitted as the BARE identifier: CITATION.cff stores the full
# https://orcid.org/… URL, and passing that through yields a record with no
# linked ORCID at all.
cff_creators() {
  awk '
    /^authors:[ \t]*$/ { grab = 1; next }
    !grab { next }
    # Dedent ends the block. Tested on the ORIGINAL line: stripping the indent
    # first would make every entry look dedented and end the block immediately.
    /^[^ \t]/ { exit }
    {
      line = $0
      if (sub(/^[ \t]*- /, "", line)) { if (rec != "") print rec; rec = "" }
      else sub(/^[ \t]+/, "", line)
      rec = rec (rec ? "\t" : "") line
    }
    END { if (rec != "") print rec }
  ' "$CFF" | jq -R -s '
    split("\n") | map(select(length > 0)) | map(
      (split("\t") | map(select(length > 0)) | map(
         capture("^(?<k>[^:]+):[[:space:]]*(?<v>.*)$")
         | {(.k): (.v | sub("^\"";"") | sub("\"$";""))}
       ) | add) as $a
      | { name: ($a["family-names"] + ", " + $a["given-names"]) }
      + (if $a.orcid then {orcid: ($a.orcid | split("/") | last)} else {} end)
      + (if $a.affiliation then {affiliation: $a.affiliation} else {} end)
    )'
}

SPECS='[
  "https://specifications.openehr.org/releases/RM/Release-1.1.0",
  "https://specifications.openehr.org/releases/ITS-REST/Release-1.1.0",
  "https://specifications.openehr.org/releases/QUERY/Release-1.1.0",
  "https://specifications.openehr.org/releases/AM/Release-2.3.0"
]'
CRATES='["openehr-base","openehr-rm","openehr-am","openehr-term",
         "openehr-lang","openehr-query","openehr-its","openehr-adl"]'

rendered="$(
  jq -n \
    --arg title       "$(cff_scalar title)" \
    --arg abstract    "$(cff_block abstract)" \
    --arg version     "$(cff_scalar version)" \
    --arg released    "$(cff_scalar date-released)" \
    --arg license     "$(cff_scalar license | tr '[:upper:]' '[:lower:]')" \
    --arg repo        "$(cff_scalar repository-code)" \
    --arg site        "$(cff_scalar url)" \
    --argjson keywords "$(cff_list keywords)" \
    --argjson creators "$(cff_creators)" \
    --argjson specs    "$SPECS" \
    --argjson crates   "$CRATES" \
'{
  # The flat legacy deposit shape — every key spelled as the help page
  # example spells it. No `doi` key on purpose: Zenodo mints the version DOI
  # itself, and a supplied one would collide with the minting.
  upload_type: "software",
  access_right: "open",
  language: "eng",
  title: $title,
  description: ("<p>" + $abstract + "</p>"),
  publication_date: $released,
  version: $version,
  creators: $creators,
  license: $license,
  keywords: $keywords,
  references: [
    "openEHR Reference Model (RM) Release 1.1.0. openEHR International. https://specifications.openehr.org/releases/RM/Release-1.1.0",
    "openEHR Archetype Query Language (AQL), QUERY Release 1.1.0. openEHR International. https://specifications.openehr.org/releases/QUERY/Release-1.1.0",
    "openEHR REST API (ITS-REST) Release 1.1.0. openEHR International. https://specifications.openehr.org/releases/ITS-REST/Release-1.1.0",
    "openEHR Archetype Model (AM) Release 2.3.0. openEHR International. https://specifications.openehr.org/releases/AM/Release-2.3.0"
  ],
  related_identifiers: (
    # A supplied related_identifiers list REPLACES the ones the GitHub
    # integration would add (measured on the v3.17.7 record, #2404), and the
    # record page derives its "Available in GitHub" link from the
    # isSupplementTo entry — so this entry must BE the stock tag-tree link,
    # never the repo root. The version here is the tag by construction: the
    # release cut bumps CITATION.cff in the PR the tag is cut from, and
    # citation-guard pins it to the workspace version.
    [ { identifier: $site,
        relation: "isDocumentedBy",
        resource_type: "publication-softwaredocumentation" },
      { identifier: ($repo + "/tree/v" + $version),
        relation: "isSupplementTo",
        resource_type: "software" } ]
    + ($specs | map({ identifier: ., relation: "isDerivedFrom" }))
    + ($crates | map({ identifier: ("https://crates.io/crates/" + .),
                       relation: "hasPart",
                       resource_type: "software" }))
  ),
  notes: "Implements the openEHR specifications at these pinned versions: Reference Model 1.2.0, BASE 1.3.0, Archetype Model 1.4.0 and 2.4.0, Terminology 3.1.0, AQL (QUERY) 1.1.0, ITS-REST 1.1.0 and ITS-XML. The specification layer is generated from the official machine-readable specifications rather than hand-written, and conformance is measured per release by a built-in openEHR CNF conformance runner whose results are committed alongside the source. Requires PostgreSQL 18. Licensing: the recorded licence covers this project'\''s own code; vendored third-party material keeps its upstream terms — Apache-2.0 for the openEHR machine-readable artifacts and test corpora, CC-BY-SA-3.0 for the openEHR specification text and CKM-derived clinical models — each recorded in the PROVENANCE.md of the tree that carries it."
}'
)"

if [[ "$MODE" = "--check" ]]; then
  [[ -f "$OUT" ]] || { echo "::error::$OUT is missing — run scripts/render/zenodo-json.sh" >&2; exit 1; }
  if ! printf '%s\n' "$rendered" | diff -u "$OUT" - >/dev/null; then
    echo "::error::$OUT is stale versus $CFF — run scripts/render/zenodo-json.sh" >&2
    printf '%s\n' "$rendered" | diff -u "$OUT" - || true
    exit 1
  fi
  echo "zenodo-json: $OUT matches $CFF"
  exit 0
fi

printf '%s\n' "$rendered" > "$OUT"
echo "zenodo-json: wrote $OUT from $CFF"
