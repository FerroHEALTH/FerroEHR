#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# scripts/watch/ghcr-orphans.sh — enumerate GENUINELY unreachable GHCR manifests.
#
# WHY (#2779): every image push in this estate is push-by-digest followed by a
# separate tag application (scan-and-tag.yml). A run that dies between those two
# steps leaves a manifest in the registry that no tag and no index points at, and
# nothing in the estate prunes it — GHCR keeps it, and it keeps counting against
# the package's storage and its version list, forever.
#
# WHY A NAIVE UNTAGGED LIST IS USELESS HERE: on this repository ~80% of every
# package's versions are untagged and almost all of them are REACHABLE. A
# multi-arch image is a tagged INDEX whose per-architecture manifests carry no
# tags of their own; buildkit adds one attestation manifest per platform to that
# same index; and because GHCR serves no referrers API, cosign signatures and
# SLSA attestations reach the registry through the OCI fallback TAG
# `sha256-<hex>` — a tagged index whose children are, again, untagged. So this
# script computes reachability rather than reporting absence of a tag:
#
#   orphan = untagged AND not a child of any tagged index (at any depth)
#
# The tagged half comes from the GitHub packages API (which knows the tags), the
# child half from the OCI distribution API (which knows the indexes). Both are
# read-only.
#
# THIS SCRIPT NEVER DELETES, and the watcher that calls it never deletes either.
# Deletion is a manual, reviewed act: an automated prune that miscomputed
# reachability by one media type would destroy published attestations and
# signatures for releases that are immutable by policy, and the failure would be
# silent until a consumer's `gh attestation verify` returned a 404. The report
# lists the exact `gh api -X DELETE` command for each orphan instead.
#
# Usage:
#   scripts/watch/ghcr-orphans.sh                 # report to stdout
#   scripts/watch/ghcr-orphans.sh --out report.md # also write the markdown body
#   scripts/watch/ghcr-orphans.sh --owner X --package Y   # narrow the sweep
#
# Requires: gh (authenticated, `read:packages`), jq, curl.
# Exits non-zero only when the PROBE cannot answer — an unreachable API, a
# manifest the registry refuses, an answer jq cannot read. Finding orphans is a
# successful run (the watcher family's run-colour law, #2778).

# Every backtick below is markdown in a report body, never command substitution.
# shellcheck disable=SC2016
set -euo pipefail

OWNER="FerroHEALTH"
PACKAGES=(ferroehr ferroehr-viewer ferroehr-postgres)
OUT=""

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --owner) OWNER="$2"; shift 2 ;;
    --package) PACKAGES=("$2"); shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

for tool in gh jq curl; do
  command -v "$tool" >/dev/null 2>&1 || { echo "$tool is required" >&2; exit 1; }
done

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# Every manifest media type a child of ours can be. Listing them explicitly
# matters: a registry served a v2 schema when the Accept header omitted the OCI
# index type, and the index would then read as a leaf with no children — every
# per-arch manifest under it would be reported as an orphan.
ACCEPT='application/vnd.oci.image.index.v1+json'
ACCEPT="${ACCEPT},application/vnd.docker.distribution.manifest.list.v2+json"
ACCEPT="${ACCEPT},application/vnd.oci.image.manifest.v1+json"
ACCEPT="${ACCEPT},application/vnd.docker.distribution.manifest.v2+json"

# A pull token for one repository. The packages are public, so an anonymous
# token is enough and is what a third party auditing this would use; a token in
# GH_TOKEN/GITHUB_TOKEN is offered to the token endpoint when present so the
# script also works before a package is made public.
ghcr_token() {
  local repo="$1" auth=() body
  if [[ -n "${GITHUB_TOKEN:-${GH_TOKEN:-}}" ]]; then
    auth=(-u "x:${GITHUB_TOKEN:-${GH_TOKEN}}")
  fi
  body="$(curl --proto '=https' --tlsv1.2 -sSL --fail \
    --retry 5 --retry-connrefused --retry-delay 2 --max-time 60 \
    "${auth[@]+"${auth[@]}"}" \
    "https://ghcr.io/token?service=ghcr.io&scope=repository:${repo}:pull")" || return 1
  printf '%s' "$body" | jq -er '.token'
}

# One manifest by digest. Guarded by the caller: a refusal is a probe failure,
# never "it has no children".
fetch_manifest() {
  local repo="$1" digest="$2" token="$3"
  curl --proto '=https' --tlsv1.2 -sSL --fail \
    --retry 5 --retry-connrefused --retry-delay 2 --max-time 60 \
    -H "Authorization: Bearer ${token}" -H "Accept: ${ACCEPT}" \
    "https://ghcr.io/v2/${repo}/manifests/${digest}"
}

failures=0
total_untagged=0
total_by_index=0
total_referrer=0
total_orphans=0
: > "$WORK/report.md"
: > "$WORK/orphans.tsv"

for pkg in "${PACKAGES[@]}"; do
  repo="${OWNER}/${pkg}"
  echo "== ${repo}" >&2

  # `name` is the manifest digest, `id` is what the delete endpoint takes.
  if ! gh api "/orgs/${OWNER}/packages/container/${pkg}/versions?per_page=100" --paginate \
      --jq '.[] | [.name, (.id|tostring), ((.metadata.container.tags // []) | join(","))] | @tsv' \
      > "$WORK/${pkg}.versions"; then
    echo "::error::could not list versions of ${repo} — the probe could not answer" >&2
    failures=$((failures + 1))
    continue
  fi

  awk -F'\t' '$3 != ""' "$WORK/${pkg}.versions" > "$WORK/${pkg}.tagged"
  awk -F'\t' '$3 == ""' "$WORK/${pkg}.versions" > "$WORK/${pkg}.untagged"

  if ! token="$(ghcr_token "$repo")"; then
    echo "::error::could not obtain a pull token for ${repo} — the probe could not answer" >&2
    failures=$((failures + 1))
    continue
  fi

  # Walk every tagged manifest and record the digests it points at. The `sha256-`
  # fallback tags are walked like any other index: their children are the
  # signature and provenance blobs' manifests, which is exactly the set a naive
  # untagged list would offer up for deletion.
  : > "$WORK/${pkg}.by-index"
  : > "$WORK/${pkg}.by-referrer"
  : > "$WORK/${pkg}.nested"
  probe_ok=1
  while IFS=$'\t' read -r digest _id tags; do
    [[ -n "$digest" ]] || continue
    case "$tags" in sha256-*) sink="$WORK/${pkg}.by-referrer" ;; *) sink="$WORK/${pkg}.by-index" ;; esac
    if ! manifest="$(fetch_manifest "$repo" "$digest" "$token")"; then
      echo "::error::${repo}@${digest} is tagged (${tags}) but its manifest could not be fetched — reachability is unknown, so nothing is reported" >&2
      probe_ok=0
      break
    fi
    printf '%s' "$manifest" | jq -r '.manifests // [] | .[].digest' >> "$sink" || {
      echo "::error::${repo}@${digest} returned a manifest jq could not read" >&2
      probe_ok=0
      break
    }
    # A child that is ITSELF an index has grandchildren to reach; the media type
    # in the parent's descriptor says so, so no extra request is spent asking.
    printf '%s' "$manifest" \
      | jq -r '.manifests // [] | .[] | select(.mediaType | test("index|manifest.list")) | .digest' \
      >> "$WORK/${pkg}.nested"
  done < "$WORK/${pkg}.tagged"

  if [[ "$probe_ok" -eq 0 ]]; then
    failures=$((failures + 1))
    continue
  fi

  # The nested level. Nothing in this estate writes such a shape today, so this
  # pass normally reads nothing — it is here because "normally" is not a
  # reachability proof, and a missed level presents as a deletable orphan.
  while IFS= read -r child; do
    [[ -n "$child" ]] || continue
    if ! manifest="$(fetch_manifest "$repo" "$child" "$token")"; then
      echo "::error::${repo}@${child} is referenced by a tagged index but could not be fetched" >&2
      probe_ok=0
      break
    fi
    printf '%s' "$manifest" | jq -r '.manifests // [] | .[].digest' >> "$WORK/${pkg}.by-index"
  done < <(sort -u "$WORK/${pkg}.nested")

  if [[ "$probe_ok" -eq 0 ]]; then
    failures=$((failures + 1))
    continue
  fi

  sort -u "$WORK/${pkg}.by-index" > "$WORK/${pkg}.by-index.u"
  sort -u "$WORK/${pkg}.by-referrer" > "$WORK/${pkg}.by-referrer.u"
  cat "$WORK/${pkg}.by-index.u" "$WORK/${pkg}.by-referrer.u" | sort -u > "$WORK/${pkg}.referenced"

  untagged=$(wc -l < "$WORK/${pkg}.untagged" | tr -d ' ')
  cut -f1 "$WORK/${pkg}.untagged" | sort -u > "$WORK/${pkg}.untagged.d"
  by_index=$(comm -12 "$WORK/${pkg}.untagged.d" "$WORK/${pkg}.by-index.u" | wc -l | tr -d ' ')
  by_referrer=$(comm -12 "$WORK/${pkg}.untagged.d" "$WORK/${pkg}.by-referrer.u" | wc -l | tr -d ' ')
  comm -23 "$WORK/${pkg}.untagged.d" "$WORK/${pkg}.referenced" > "$WORK/${pkg}.orphans.d"
  orphans=$(wc -l < "$WORK/${pkg}.orphans.d" | tr -d ' ')

  total_untagged=$((total_untagged + untagged))
  total_by_index=$((total_by_index + by_index))
  total_referrer=$((total_referrer + by_referrer))
  total_orphans=$((total_orphans + orphans))

  {
    printf '### `%s`\n\n' "$repo"
    printf '| | count |\n|---|---|\n'
    printf '| versions | %s |\n' "$(wc -l < "$WORK/${pkg}.versions" | tr -d ' ')"
    printf '| tagged | %s |\n' "$(wc -l < "$WORK/${pkg}.tagged" | tr -d ' ')"
    printf '| untagged | %s |\n' "$untagged"
    printf '| untagged, but a per-arch or attestation child of a tagged index | %s |\n' "$by_index"
    printf '| untagged, but a child of a `sha256-…` referrer index (signature / provenance) | %s |\n' "$by_referrer"
    printf '| **genuine orphans** | **%s** |\n\n' "$orphans"
  } >> "$WORK/report.md"

  if [[ "$orphans" -gt 0 ]]; then
    {
      printf 'Unreachable manifests in `%s`, with the command that removes each one:\n\n' "$repo"
      printf '```shell\n'
      while IFS= read -r d; do
        [[ -n "$d" ]] || continue
        id=$(awk -F'\t' -v d="$d" '$1==d{print $2; exit}' "$WORK/${pkg}.untagged")
        printf '# %s\n' "$d"
        printf 'gh api -X DELETE /user/packages/container/%s/versions/%s\n' "$pkg" "$id"
        printf '%s\t%s\t%s\n' "$pkg" "$d" "$id" >> "$WORK/orphans.tsv"
      done < "$WORK/${pkg}.orphans.d"
      printf '```\n\n'
    } >> "$WORK/report.md"
  fi
done

if [[ "$failures" -gt 0 ]]; then
  echo "ghcr-orphans: $failures package(s) could not be assessed — reporting nothing." >&2
  exit 1
fi

{
  printf 'Untagged manifests in GHCR, split by whether anything still reaches them.\n'
  printf 'A tagged index reaches its per-architecture manifests, its buildkit\n'
  printf 'attestation manifests, and — through the `sha256-…` OCI fallback tag,\n'
  printf 'because GHCR serves no referrers API — the cosign signature and SLSA\n'
  printf 'provenance of every published digest. Only what NOTHING reaches is an\n'
  printf 'orphan, and only orphans are listed below.\n\n'
  printf '| | count |\n|---|---|\n'
  printf '| untagged versions | %s |\n' "$total_untagged"
  printf '| reachable as an index child | %s |\n' "$total_by_index"
  printf '| reachable as a referrer artifact | %s |\n' "$total_referrer"
  printf '| **genuine orphans** | **%s** |\n\n' "$total_orphans"
  cat "$WORK/report.md"
  if [[ "$total_orphans" -gt 0 ]]; then
    printf 'These are the residue of a run that pushed by digest and then died before\n'
    printf '`scan-and-tag` applied its tags. Deletion is deliberately MANUAL: an\n'
    printf 'automated prune that miscomputes reachability by one media type destroys\n'
    printf 'published signatures and attestations for releases this project publishes\n'
    printf 'as immutable, and the damage is invisible until a consumer verifies. Check\n'
    printf 'a couple of digests by hand (`docker buildx imagetools inspect\n'
    printf '%s/<pkg>@<digest>`) before running anything above.\n' "ghcr.io/${OWNER}"
  fi
} > "$WORK/body.md"

cat "$WORK/body.md"
if [[ -n "$OUT" ]]; then
  cp "$WORK/body.md" "$OUT"
fi
printf 'orphans=%s\n' "$total_orphans" >&2
