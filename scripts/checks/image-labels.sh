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
#     authors, licenses, base.name, base.digest) — both places, identical values
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
SHARED="title description url documentation source vendor authors licenses base.name base.digest"

failures=0
report() { printf '%s\n' "$*" >&2; failures=$((failures + 1)); }

# The value of `org.opencontainers.image.<key>` in a Dockerfile LABEL block.
dockerfile_label() {
  awk -v key="org.opencontainers.image.$2=" '
    index($0, key) {
      i = index($0, key) + length(key)
      v = substr($0, i)
      sub(/^"/, "", v); sub(/"[[:space:]]*\\?[[:space:]]*$/, "", v)
      print v; exit
    }' "$1"
}

# The value declared in the workflow's `labels:` block for one lane. A lane is
# one `uses: build-image.yml` call; it opens at its `image: ghcr.io/...<name>`
# input (matched to end-of-line, so a name that prefixes another cannot open
# the wrong lane) and closes at the next lane's. The owner is derived from
# `github.repository_owner` in the workflow, so the lane opener is matched by
# its trailing `/<name>` rather than by a hard-coded owner.
workflow_label() {
  awk -v lane="/$1" -v key="org.opencontainers.image.$2=" '
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

  # base.name/base.digest must match the runtime stage's actual FROM pin, or the
  # image claims a parent it was not built on — worse than claiming none.
  from=$(grep -E '^FROM [^ ]+@sha256:' "$dockerfile" | tail -1 || true)
  if [[ -z "$from" ]]; then
    report "image-labels: $dockerfile has no digest-pinned FROM to check base.* against"
  else
    ref=$(printf '%s' "$from" | awk '{print $2}')
    want_name=${ref%@*}
    want_digest=${ref#*@}
    got_name=$(dockerfile_label "$dockerfile" base.name || true)
    got_digest=$(dockerfile_label "$dockerfile" base.digest || true)
    [[ "$got_name" = "$want_name" ]] \
      || report "image-labels: $dockerfile base.name is '$got_name', but its FROM is '$want_name'"
    [[ "$got_digest" = "$want_digest" ]] \
      || report "image-labels: $dockerfile base.digest is '$got_digest', but its FROM pins '$want_digest'"
  fi
done

# Every service container must run the exact base the postgres image is built
# on — a drifted service pin tests against different bytes than the image ships
# (found as a checklist item in #2408/#2410, enforced here since).
#
# EVERY workflow is scanned, not a named list: the check was written against
# ci.yml alone and sonar.yml's identical service pin — a second full instrumented
# suite against the same database — was never covered (#2775). A list would have
# to be extended by whoever adds the next service container, which is exactly the
# person who does not know this guard exists.
pg_from=$(grep -E '^FROM postgres:' docker/postgres/Dockerfile | awk '{print $2}' || true)
if [[ -z "$pg_from" ]]; then
  report "image-labels: docker/postgres/Dockerfile has no 'FROM postgres:' pin"
else
  found_any=0
  for wf in .github/workflows/*.yml; do
    pins=$(grep -Eo 'image: postgres:[^[:space:]]+' "$wf" | sed 's/^image: //' | sort -u || true)
    [[ -n "$pins" ]] || continue
    found_any=1
    for pin in $pins; do
      [[ "$pin" = "$pg_from" ]] \
        || report "image-labels: $wf pins service '$pin', but docker/postgres/Dockerfile FROM is '$pg_from'"
    done
  done
  # A guard that finds nothing to check is not passing, it is vacuous.
  [[ "$found_any" -eq 1 ]] \
    || report "image-labels: no workflow declares a postgres service pin — this check verified nothing"
fi

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
