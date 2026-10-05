#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Generate the committed example skeletons for the CKM journey template
# pack: upload every vendored OPT to a running ferroehr SUT, fetch its
# example composition (the ITS-REST "Get example data by template"
# endpoint), and commit the responses byte-identical — the deterministic
# payload ground EVERY SUT receives at measurement time (never fetched at
# run time; fairness across SUTs).
#
# Precondition: a running SUT (default: the dev compose stack). Re-run
# after re-vendoring the pack (scripts/vendor/ckm-templates.sh).
set -Eeuo pipefail

BASE="${SUT_BASE:-http://localhost:8080/ferroehr/rest/openehr/v1}"
AUTH="${SUT_USER:-ferroehr-user}:${SUT_PASS:-SuperSecretPassword}"
PACK="corpus/templates/ckm"

command -v jq >/dev/null || { echo "::error::jq required" >&2; exit 1; }

for opt in "$PACK"/*.opt; do
  slug=$(basename "$opt" .opt)
  # The OPT's template_id is the wire identifier.
  # The template_id, read with sed rather than python: the element pair may be
  # split across lines in a pretty-printed OPT, so the newlines are collapsed
  # first and the first match taken — the same first-match semantics the previous
  # regex had.
  tid=$(tr '\n' ' ' < "$opt" \
    | sed -n 's|.*<template_id>[[:space:]]*<value>\([^<]*\)</value>.*|\1|p' \
    | head -1)
  [[ -n "$tid" ]] || { echo "::error::no <template_id><value> in $opt" >&2; exit 1; }
  echo "==> $slug ($tid)"
  status=$(curl -sS -o /dev/null -w '%{http_code}' -u "$AUTH" \
    -X POST "$BASE/definition/template/adl1.4" \
    -H "Content-Type: application/xml" --data-binary @"$opt")
  case "$status" in
    201|409) ;;
    *) echo "::error::OPT upload for $slug returned $status" >&2; exit 1 ;;
  esac
  # Percent-encode the template id for the path segment. jq's @uri applies the
  # same unreserved set as RFC 3986 (`A-Za-z0-9-_.~` unescaped), which is what
  # the previous `quote(safe='')` produced.
  encoded=$(jq -rn --arg tid "$tid" '$tid | @uri')
  curl -fsS -u "$AUTH" \
    "$BASE/definition/template/adl1.4/$encoded/example?detail_level=medium" \
    -H "Accept: application/json" | jq -S . > "$PACK/$slug.example.json"
  # sanity: a COMPOSITION instance
  jq -e '._type == "COMPOSITION"' "$PACK/$slug.example.json" >/dev/null \
    || { echo "::error::$slug example is not a COMPOSITION" >&2; exit 1; }
done

examples=("$PACK"/*.example.json)
echo "==> generated ${#examples[@]} example skeletons"
