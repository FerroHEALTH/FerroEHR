#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Image-metadata guard: the OCI annotation keys are declared once, correctly, and
# the two places that declare them agree.
#
# Three defects motivated this, all found on the published images (#2146):
#
#   * No annotations existed at all. A LABEL lives in the image config blob; an
#     ANNOTATION lives in the manifest or the index. GHCR reads a package's
#     description from the INDEX annotation, so three images carrying a perfectly
#     good description LABEL all displayed "No description provided".
#   * Six of the spec's fourteen predefined keys were unset, including
#     `documentation` (we publish a docs site and never pointed at it) and
#     `base.name`/`base.digest`, whose values were already pinned in the `FROM`.
#   * All three Dockerfiles declared `licenses="Apache-2.0"` for a then-MIT project.
#     That was invisible on GHCR because `build-push-action`'s `labels:` input
#     overrides a Dockerfile LABEL, so CI published the correct licence — and
#     anyone building the Dockerfile directly, the documented compose path,
#     shipped an image asserting a licence the project does not use.
#
# The third is the one this guard exists for. The same metadata is declared in
# two places — the Dockerfile, so a direct build is correct, and the publishing
# workflow, whose repository-derived defaults are wrong for a repo that ships
# three images (title comes from the repo NAME, so all three would claim to be
# "FerroEHR"). Two declarations of one fact drift; this makes them fail instead.
#
# EVERY workflow that declares them is checked, not one: the main lane
# (containers.yml) and the release pipeline (release.yml) each carry the three
# `uses: build-image.yml` calls with their labels (#2776), so a label corrected
# in one of them alone would publish a differently-described image at a release
# than on main.
#
# Ownership, which is what the checks below encode:
#   build-INDEPENDENT (title, description, url, documentation, source, vendor,
#     authors, licenses) — both places, identical values
#   base.name, base.digest — NEITHER place: the reusable build-image.yml lane
#     reads them from the runtime stage's digest-pinned FROM through
#     scripts/lib/base-image.sh, so the FROM Dependabot moves is the one place
#     the base digest is written (#4953). A hand-written copy is refused, since
#     it is the copy a bump leaves behind.
#   build-DEPENDENT (created, version, revision, ref.name) — the workflow only,
#     since a Dockerfile cannot know them
#   ref.name — deliberately unset; see the waiver below
#
# Usage: scripts/checks/image-labels.sh   (no arguments)
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"
cd "$(dirname "$0")/../.."

WORKFLOWS=".github/workflows/containers.yml
.github/workflows/release.yml"
# image name : Dockerfile. Each publishing lane is a `uses:` call of the
# reusable build-image.yml, keyed by its full `image:` input ref matched to
# end-of-line — `ferroehr` is a prefix of `ferroehr-viewer`, so a bare name
# match would read the wrong lane's labels.
IMAGES="ferroehr:docker/Dockerfile
ferroehr-viewer:docker/viewer/Dockerfile
ferroehr-postgres:docker/postgres/Dockerfile"
BUILD_WORKFLOW=.github/workflows/build-image.yml

# The keys both places must declare, identically.
SHARED="title description url documentation source vendor authors licenses"
DERIVED="base.name base.digest"

failures=0
report() { printf '%s\n' "$*" >&2; failures=$((failures + 1)); }

# The value of `org.opencontainers.image.<key>` in a Dockerfile LABEL block.
dockerfile_label() {
  local dockerfile="$1" key="$2"
  awk -v key="org.opencontainers.image.$key=" '
    index($0, key) {
      i = index($0, key) + length(key)
      v = substr($0, i)
      sub(/^"/, "", v); sub(/"[[:space:]]*\\?[[:space:]]*$/, "", v)
      print v; exit
    }' "$dockerfile"
}

# The value declared in the workflow's `labels:` block for one lane. A lane is
# one `uses: build-image.yml` call; it opens at its `image: ghcr.io/...<name>`
# input (matched to end-of-line, so a name that prefixes another cannot open
# the wrong lane) and closes at the next lane's. The owner is derived from
# `github.repository_owner` in the workflow, so the lane opener is matched by
# its trailing `/<name>` rather than by a hard-coded owner.
workflow_label() {
  local lane="$1" key="$2"
  awk -v lane="/$lane" -v key="org.opencontainers.image.$key=" '
    /image: ghcr\.io\// {
      line = $0
      sub(/^[[:space:]]*/, "", line); sub(/[[:space:]]*$/, "", line)
      inlane = (substr(line, length(line) - length(lane) + 1) == lane)
    }
    inlane && index($0, key) {
      i = index($0, key) + length(key)
      v = substr($0, i)
      sub(/^[[:space:]]*/, "", v); sub(/[[:space:]]*$/, "", v)
      print v; exit
    }' "$WORKFLOW"
}

for entry in $IMAGES; do
  image=${entry%%:*}
  dockerfile=${entry#*:}
  [[ -f "$dockerfile" ]] || { report "image-labels: missing $dockerfile"; continue; }

  for key in $SHARED; do
    d=$(dockerfile_label "$dockerfile" "$key" || true)
    if [[ -z "$d" ]]; then
      report "image-labels: $dockerfile declares no org.opencontainers.image.$key"
      continue
    fi
    for WORKFLOW in $WORKFLOWS; do
      w=$(workflow_label "$image" "$key" || true)
      if [[ -z "$w" ]]; then
        report "image-labels: $WORKFLOW declares no org.opencontainers.image.$key for $image"
        continue
      fi
      # The Dockerfile substitutes ${VERSION}/${REVISION} from build args; the
      # shared keys are all literals, so a plain comparison is right.
      if [[ "$d" != "$w" ]]; then
        report "image-labels: $image .$key disagrees —
    $dockerfile: $d
    $WORKFLOW: $w"
      fi
    done
  done

  # base.name/base.digest are derived from the runtime stage's FROM pin at
  # build time, so that pin must be readable and no copy may be written down.
  bash scripts/lib/base-image.sh labels "$dockerfile" >/dev/null \
    || report "image-labels: $dockerfile has no digest-pinned FROM to derive base.* from"
  for key in $DERIVED; do
    d=$(dockerfile_label "$dockerfile" "$key" || true)
    [[ -z "$d" ]] \
      || report "image-labels: $dockerfile declares org.opencontainers.image.$key; it is read from the FROM pin at build time, so declare it nowhere"
    for WORKFLOW in $WORKFLOWS; do
      w=$(workflow_label "$image" "$key" || true)
      [[ -z "$w" ]] \
        || report "image-labels: $WORKFLOW declares org.opencontainers.image.$key for $image; build-image.yml reads it from the FROM pin, so declare it nowhere"
    done
  done
done

# Every PostgreSQL service container runs the base the postgres image is built
# on, read from docker/postgres/Dockerfile's FROM by the job it needs (#5112),
# so a Dependabot bump of that FROM moves every service with it. A drifted
# service pin would test against different bytes than the image ships
# (#2408/#2410). EVERY workflow is scanned, not a named list (#2775): a literal
# pin is refused wherever it appears, a derived image must name its producer,
# and the producer must read the FROM and hold the value to name@sha256, since
# zizmor cannot see through the expression and each one carries its
# unpinned-images suppression.
if ! bash scripts/lib/base-image.sh ref docker/postgres/Dockerfile >/dev/null; then
  report "image-labels: docker/postgres/Dockerfile has no digest-pinned FROM for the services to run"
fi
found_any=0
for wf in .github/workflows/*.yml; do
  literal=$(grep -Eo 'image: postgres:[^[:space:]]+' "$wf" | sort -u || true)
  [[ -z "$literal" ]] \
    || report "image-labels: $wf pins a PostgreSQL service by hand ($literal); take \${{ needs.<job>.outputs.postgres_image }} from the job that reads docker/postgres's FROM"
  derived=$(grep -Ec 'image: \$\{\{ needs\.[a-z_-]+\.outputs\.postgres_image \}\}' "$wf" || true)
  [[ "$derived" -gt 0 ]] || continue
  found_any=1
  suppressed=$(grep -Ec 'image: \$\{\{ needs\.[a-z_-]+\.outputs\.postgres_image \}\} # zizmor: ignore\[unpinned-images\] .+' "$wf" || true)
  [[ "$suppressed" -eq "$derived" ]] \
    || report "image-labels: $wf has $derived derived PostgreSQL service image(s) and $suppressed with its unpinned-images suppression and reason"
  # shellcheck disable=SC2016 # the literal workflow text searched for
  grep -qF 'postgres_image: ${{ steps.postgres.outputs.image }}' "$wf" \
    || report "image-labels: $wf runs a derived PostgreSQL service but no job publishes postgres_image from a step with id postgres"
  # shellcheck disable=SC2016 # the literal workflow text searched for
  grep -qF 'image="$(bash scripts/lib/base-image.sh ref docker/postgres/Dockerfile)"' "$wf" \
    || report "image-labels: $wf derives the PostgreSQL service image from something other than docker/postgres's FROM"
  # shellcheck disable=SC2016 # the literal workflow text searched for
  grep -qF '[[ "$image" =~ ^[^@[:space:]]+@sha256:[0-9a-f]{64}$ ]]' "$wf" \
    || report "image-labels: $wf does not hold the derived PostgreSQL service image to name@sha256:<digest>"
done
# A guard that finds nothing to check is not passing, it is vacuous.
[[ "$found_any" -eq 1 ]] \
  || report "image-labels: no workflow runs a derived PostgreSQL service image; this check verified nothing"

# Annotations must reach the INDEX, which is the only place GHCR reads a package
# description from. The publishing mechanics live ONCE in the reusable
# build-image.yml lane; each publishing workflow calls it once per image.
levels=$(grep -c 'DOCKER_METADATA_ANNOTATIONS_LEVELS: index,manifest' "$BUILD_WORKFLOW" || true)
[[ "$levels" -eq 1 ]] \
  || report "image-labels: expected the one reusable metadata step with DOCKER_METADATA_ANNOTATIONS_LEVELS: index,manifest in $BUILD_WORKFLOW, found $levels"
# shellcheck disable=SC2016 # ${{ … }} is the literal Actions expression being searched for
annots=$(grep -c 'annotations: ${{ steps.meta.outputs.annotations }}' "$BUILD_WORKFLOW" || true)
[[ "$annots" -eq 1 ]] \
  || report "image-labels: expected the one reusable build step passing the annotations output in $BUILD_WORKFLOW, found $annots"
# shellcheck disable=SC2016 # the literal workflow text being searched for
derive=$(grep -c 'bash scripts/lib/base-image.sh labels "$DOCKERFILE"' "$BUILD_WORKFLOW" || true)
[[ "$derive" -eq 1 ]] \
  || report "image-labels: expected the one step deriving base.* from the Dockerfile in $BUILD_WORKFLOW, found $derive"
# shellcheck disable=SC2016 # ${{ … }} is the literal Actions expression being searched for
fed=$(grep -c 'labels: ${{ steps.base.outputs.labels }}' "$BUILD_WORKFLOW" || true)
[[ "$fed" -eq 1 ]] \
  || report "image-labels: expected the metadata step to take the derived labels in $BUILD_WORKFLOW, found $fed"
for WORKFLOW in $WORKFLOWS; do
  lanes=$(grep -c 'uses: ./.github/workflows/build-image.yml' "$WORKFLOW" || true)
  [[ "$lanes" -eq 3 ]] \
    || report "image-labels: expected 3 publishing lanes calling build-image.yml in $WORKFLOW, found $lanes"
done

# The one predefined key we deliberately leave unset, recorded rather than
# silently skipped: `ref.name` is "name of the reference for a target", which the
# spec leaves to the consumer and which our tags already express — an image
# carries several tags, so a single ref.name would have to pick one arbitrarily.
# `created` is set by the action from the build time and needs no declaration.

if [[ "$failures" -gt 0 ]]; then
  echo "image-labels: $failures problem(s) — see above." >&2
  exit 1
fi
echo "image-labels: OK."
