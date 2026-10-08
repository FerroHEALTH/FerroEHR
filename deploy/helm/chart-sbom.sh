#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# A CycloneDX SBOM of a packaged chart: the chart, and every image it deploys,
# named by digest.
#
# CRA Annex I Part II(1) asks for an SBOM "covering at the very least the
# top-level dependencies of the products" (docs/law/eu/cra/text.html). A chart's
# top-level dependencies are the images it deploys, and a tag can move, so each
# image is resolved to the digest its tag names at the moment the chart is
# published. The image list is the packaged `artifacthub.io/images` annotation,
# the set scripts/checks/chart-appversion.sh holds to what the templates deploy
# and deploy/helm/release-facts.sh rewrites for the release.
#
# The publish lane attests the document against the pushed chart digest
# (build-chart.yml), so `gh attestation verify --predicate-type
# https://cyclonedx.org/bom` reads it back for any chart version.
#
# Usage: deploy/helm/chart-sbom.sh <packaged-chart.tgz> <out.cdx.json> [chart-digest [chart-repository]]
#   The digest is empty on a dry run, which pushes nothing; the repository
#   defaults to ghcr.io/ferrohealth/charts/ferroehr.
# Requires: helm, yq, jq, oras.

set -euo pipefail

PACKAGE="${1:-}"
OUT="${2:-}"
CHART_DIGEST="${3:-}"
CHART_REPOSITORY="${4:-ghcr.io/ferrohealth/charts/ferroehr}"
if [[ -z "$PACKAGE" || -z "$OUT" ]]; then
  echo "usage: $(basename "$0") <packaged-chart.tgz> <out.cdx.json> [chart-digest]" >&2
  exit 2
fi
[[ -f "$PACKAGE" ]] || { echo "error: no package at ${PACKAGE}" >&2; exit 2; }

chart="$(helm show chart "$PACKAGE")"
name="$(yq '.name' <<<"$chart")"
version="$(yq '.version' <<<"$chart")"
app_version="$(yq '.appVersion' <<<"$chart")"
images="$(yq -o=json '.annotations["artifacthub.io/images"] // "" | from_yaml // []' <<<"$chart")"
count="$(jq 'length' <<<"$images")"
[[ "$count" -gt 0 ]] || { echo "error: ${PACKAGE} declares no artifacthub.io/images" >&2; exit 1; }

# One component per image: its repository, the tag the chart names, and the
# digest that tag resolves to now (or the digest the reference already pins).
components="[]"
for i in $(seq 0 $((count - 1))); do
  entry="$(jq -c ".[$i]" <<<"$images")"
  ref="$(jq -r '.image' <<<"$entry")"
  repo="${ref%%[:@]*}"
  rest="${ref#"$repo"}"
  tag=""
  digest=""
  case "$rest" in
  :*@*)
    tag="${rest#:}"
    tag="${tag%%@*}"
    digest="${rest#*@}"
    ;;
  :*) tag="${rest#:}" ;;
  @*) digest="${rest#@}" ;;
  *) ;;
  esac
  if [[ -z "$digest" ]]; then
    digest="$(oras resolve "$ref")" || { echo "error: could not resolve ${ref} to a digest" >&2; exit 1; }
  fi
  [[ "$digest" =~ ^sha256:[0-9a-f]{64}$ ]] || { echo "error: ${ref} resolved to '${digest}', not a sha256 digest" >&2; exit 1; }
  components="$(jq -c \
    --argjson entry "$entry" --arg repo "$repo" --arg tag "$tag" --arg digest "$digest" '
    . + [{
      type: "container",
      "bom-ref": "\($repo)@\($digest)",
      name: ($repo | split("/") | last),
      version: (if $tag != "" then $tag else $digest end),
      hashes: [{alg: "SHA-256", content: ($digest | ltrimstr("sha256:"))}],
      purl: ("pkg:oci/\($repo | split("/") | last)@\($digest | sub(":"; "%3A"))"
             + "?repository_url=\($repo | split("/") | .[:-1] | join("/") | @uri)"
             + (if $tag != "" then "&tag=\($tag | @uri)" else "" end)),
      properties: [
        {name: "ferroehr:image-reference", value: "\($repo)\(if $tag != "" then ":" + $tag else "" end)@\($digest)"},
        {name: "ferroehr:platforms", value: (($entry.platforms // []) | join(","))}
      ]
    }]' <<<"$components")"
  echo "  ${repo}${tag:+:${tag}} -> ${digest}"
done

# CycloneDX 1.5 §serialNumber: a urn:uuid per BOM; actions/attest refuses a CycloneDX
# document without bomFormat, serialNumber and specVersion (its src/sbom.ts).
serial="urn:uuid:$(uuidgen | tr "[:upper:]" "[:lower:]")"

jq -n \
  --arg name "$name" --arg version "$version" --arg app "$app_version" \
  --arg serial "$serial" \
  --arg chart_digest "$CHART_DIGEST" --arg chart_repo "$CHART_REPOSITORY" --arg timestamp "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --argjson components "$components" '
  ("chart:\($name)@\($version)") as $chart_ref
  | {
      bomFormat: "CycloneDX",
      specVersion: "1.5",
      serialNumber: $serial,
      version: 1,
      metadata: {
        timestamp: $timestamp,
        tools: {components: [{type: "application", name: "deploy/helm/chart-sbom.sh"}]},
        supplier: {name: "Cadasto B.V.", url: ["https://www.cadasto.com/"]},
        component: ({
          type: "application",
          "bom-ref": $chart_ref,
          name: $name,
          version: $version,
          properties: [{name: "ferroehr:app-version", value: $app}]
        } + (if $chart_digest != "" then
               {hashes: [{alg: "SHA-256", content: ($chart_digest | ltrimstr("sha256:"))}],
                purl: ("pkg:oci/\($name)@\($chart_digest | sub(":"; "%3A"))"
                       + "?repository_url=\($chart_repo | split("/") | .[:-1] | join("/") | @uri)"
                       + "&tag=\($version | @uri)")}
             else {} end))
      },
      components: $components,
      dependencies: [
        {ref: $chart_ref, dependsOn: [$components[]."bom-ref"]},
        ($components[] | {ref: ."bom-ref", dependsOn: []})
      ]
    }' > "$OUT"

# The fields actions/attest keys a CycloneDX document on: refuse to hand it one
# it would reject, so the failure is here and names the cause.
jq -e '.bomFormat == "CycloneDX" and (.serialNumber | test("^urn:uuid:[0-9a-f-]{36}$")) and .specVersion == "1.5"' "$OUT" >/dev/null \
  || { echo "chart-sbom: $OUT lacks bomFormat, serialNumber or specVersion" >&2; exit 1; }

echo "${OUT}: CycloneDX 1.5, chart ${name} ${version}, ${count} image(s) by digest"
