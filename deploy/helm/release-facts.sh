#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# Rewrite the chart's PER-RELEASE facts to a given application version, at
# PACKAGE time (#2779).
#
# `helm package --app-version <v>` sets the packaged chart's appVersion field and
# nothing else. Two other places in the packaged chart restate that same version,
# and both would otherwise advertise the previous release:
#
#   * the FIRST-PARTY entries of `artifacthub.io/images` in Chart.yaml — the
#     image list Artifact Hub scans, so a stale tag reports vulnerabilities for
#     software nobody is running. First-party means the images built from this
#     repository at this version: `ferroehr`, `ferroehr-viewer` and
#     `ferroehr-postgres` today. A third-party image the chart deploys —
#     `ghcr.io/ferrohealth/ferroterm` since #3305 — is a separate product on
#     its own release line, pinned in the chart's values and helpers, so
#     rewriting its tag to a FerroEHR version would publish an image reference
#     that does not exist. Its tag is left exactly as committed.
#
#     WHICH entry is which is DECLARED: each `image:` line in the annotation
#     carries a trailing `# party: first` or `# party: third` marker, and this
#     script rewrites the first kind and only the first kind.
#     scripts/checks/chart-appversion.sh reads the same marker and refuses an
#     unmarked line, so the set checked and the set rewritten are one set by
#     construction. Matching the repository NAME instead would silently
#     rewrite a future `ghcr.io/ferrohealth/ferroehr-<something>` built by
#     somebody else, and silently leave a first-party image published under
#     another name a release behind;
#   * README.md — a GENERATED file (helm-docs, from Chart.yaml + the `# --`
#     comments in values.yaml) whose install example, "This release" table,
#     attestation example and version badge all name the appVersion. Artifact Hub
#     renders it as the package's front page.
#
# The sibling artifacthub-changes.sh already injects `artifacthub.io/changes`
# rather than committing it, on the argument that a fact about a release is stale
# in the tree from the next merge onwards. This is the same argument applied to
# the two sites that were still hand-edited at every cut.
#
# It is a set of ANCHORED substitutions rather than a `yq` round-trip or a
# helm-docs re-run: Chart.yaml is ~120 lines of load-bearing commentary (the
# kubeVersion floor's KEP table, the deliberately-absent annotations) that a
# structural rewrite would re-emit, and helm-docs is a Go binary this publishing
# lane would have to install and pin to run once. Every substitution is
# counted, and the script refuses when a count does not add up, so a file that
# changed shape fails loudly instead of publishing stale facts.
#
# Usage: deploy/helm/release-facts.sh <version>
#   e.g. deploy/helm/release-facts.sh 4.0.6
# A version equal to the committed appVersion is a no-op for the README (the
# between-releases dispatch path).

set -euo pipefail

VERSION="${1:-}"
if [[ -z "$VERSION" ]]; then
  echo "usage: $(basename "$0") <version>" >&2
  exit 2
fi
# A tag, never a ref: `v`-prefixed input is the mistake that would publish
# annotations pointing at tags this project does not push (the book's own
# warning: published image tags carry no `v` prefix).
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "::error::'$VERSION' is not a bare X.Y.Z[-suffix] image tag (published tags carry no 'v' prefix)" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHART="${SCRIPT_DIR}/ferroehr/Chart.yaml"
README="${SCRIPT_DIR}/ferroehr/README.md"
for required in "$CHART" "$README"; do
  [[ -f "$required" ]] || { echo "::error::no ${required}" >&2; exit 1; }
done

OLD=$(sed -nE 's/^appVersion: *"?([^"]+)"?$/\1/p' "$CHART")
[[ -n "$OLD" ]] || { echo "::error::could not read appVersion from ${CHART}" >&2; exit 1; }

# ── Chart.yaml: the first-party artifacthub.io/images tags ───────────────────
# `image: <ref>` occurs only inside that annotation, and the marker says which
# of those lines is ours. The counts below cover the marked-first half alone, so
# a third-party entry neither inflates the expectation nor fails the check by
# keeping its own tag.
FIRST_PARTY='^[[:space:]]+image: [^[:space:]]+[[:space:]]+# party: first$'
img_before=$(grep -cE "$FIRST_PARTY" "$CHART" || true)
if [[ "$img_before" -eq 0 ]]; then
  echo "::error::${CHART} declares no 'image: …  # party: first' line, so there is nothing to rewrite — the artifacthub.io/images annotation changed shape, or the party markers were dropped." >&2
  exit 1
fi
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
sed -E "s|^([[:space:]]+image: [^[:space:]]+):[^[:space:]:]+([[:space:]]+# party: first)\$|\\1:${VERSION}\\2|" "$CHART" > "$tmp"
cat "$tmp" > "$CHART"
img_after=$(grep -cE "^[[:space:]]+image: [^[:space:]]+:${VERSION}[[:space:]]+# party: first\$" "$CHART" || true)
if [[ "$img_after" -ne "$img_before" ]]; then
  echo "::error::rewrote ${img_after} of ${img_before} first-party artifacthub.io/images tags in ${CHART}." >&2
  exit 1
fi

# ── README.md: every restatement of the appVersion ───────────────────────────
# Four anchored contexts, not a global replace: a global one would also rewrite
# the CHART version if the two ever happened to be the same string, and the
# chart version is not a per-release fact.
readme_patterns=(
  "AppVersion-${OLD}-informational"
  "AppVersion: ${OLD}"
  "image.tag=${OLD}"
  "ferroehr:${OLD}"
  "| \`${OLD}\` |"
)
if [[ "$OLD" != "$VERSION" ]]; then
  matched=0
  for pattern in "${readme_patterns[@]}"; do
    replacement="${pattern//${OLD}/${VERSION}}"
    before=$(grep -cF "$pattern" "$README" || true)
    [[ "$before" -eq 0 ]] && continue
    matched=$((matched + 1))
    sed -i.bak "s|$(printf '%s' "$pattern" | sed 's/[|&\\]/\\&/g')|$(printf '%s' "$replacement" | sed 's/[|&\\]/\\&/g')|g" "$README"
    rm -f "${README}.bak"
    after=$(grep -cF "$replacement" "$README" || true)
    remaining=$(grep -cF "$pattern" "$README" || true)
    if [[ "$after" -lt "$before" || "$remaining" -ne 0 ]]; then
      echo "::error::rewriting '${pattern}' in ${README} left ${remaining} occurrence(s) behind." >&2
      exit 1
    fi
  done
  # Zero matches means the committed README does not restate the committed
  # appVersion — it is generated FROM Chart.yaml, so the two can only disagree
  # if the README was never regenerated. That is not a shape this may publish
  # around: the v4.0.5 cut left it a release behind (6.0.19/4.0.4 against a
  # 6.0.20/4.0.5 chart) and only a local `deploy/helm/validate.sh` run noticed,
  # because the CI lane skips the drift check when helm-docs is absent.
  if [[ "$matched" -eq 0 ]]; then
    echo "::error::${README} restates no appVersion ${OLD}, so it cannot be moved to ${VERSION}. It is generated from Chart.yaml — regenerate it: helm-docs --chart-search-root ${SCRIPT_DIR}/ferroehr --template-files README.md.gotmpl" >&2
    exit 1
  fi
fi

echo "release facts set to ${VERSION} (from a committed default of ${OLD}):" >&2
grep -E "^[[:space:]]+image: " "$CHART" >&2
grep -nE "AppVersion|image\.tag=|ferroehr:[0-9]" "$README" >&2 || true
