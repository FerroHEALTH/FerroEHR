#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The deployment-conformance harness (#2178).
#
# The conformance instrument measures the REST surface; this is the same
# instrument for the other half, "the deployment works": the defects that
# live between "the code is correct" and "the thing we hand an operator
# works", which no rendered YAML or mocked unit test reaches.
#
# Three rules it holds itself to, each one a lesson from those defects:
#
#   1. Observe at the FAR END. Not "the API said 201" but "the blob is in the
#      bucket"; not "the config looks right" but "the server reports it".
#   2. Follow the DOCUMENTATION, not the source. Several findings were the book
#      describing something that does not work, and a probe configured the way
#      the source says would never catch that class again.
#   3. Say what was NOT exercised. Silence read as coverage is how this
#      happened; the report ends with the gaps, in its own output.
#
# Usage:
#   scripts/deploy-probe.sh                  # the compose platform
#   scripts/deploy-probe.sh --keep-up        # leave the stack running to poke at
#
# Env:
#   FERROEHR_IMAGE   the server image under probe (default: the compose pin).
#                    The harness measures the SUT it is given — pointing it at
#                    an older image is a legitimate way to watch a regression
#                    probe fail.
#   PROBE_OUT        where the machine-readable record lands.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

KEEP_UP=0
[[ "${1:-}" = "--keep-up" ]] && KEEP_UP=1

PROBE_TMP="$(mktemp -d)"
export PROBE_TMP
PROBE_OUT="${PROBE_OUT:-docs/conformance/deployment/compose.json}"

# shellcheck source=scripts/deploy-probes/lib.sh
. scripts/deploy-probes/lib.sh
# shellcheck source=scripts/deploy-probes/compose.sh
. scripts/deploy-probes/compose.sh
# shellcheck source=scripts/deploy-probes/oidc.sh
. scripts/deploy-probes/oidc.sh
# shellcheck source=scripts/deploy-probes/signing.sh
. scripts/deploy-probes/signing.sh
# shellcheck source=scripts/deploy-probes/observability.sh
. scripts/deploy-probes/observability.sh
# shellcheck source=scripts/deploy-probes/terminology.sh
. scripts/deploy-probes/terminology.sh
# shellcheck source=scripts/deploy-probes/signing_pgp.sh
. scripts/deploy-probes/signing_pgp.sh
# shellcheck source=scripts/deploy-probes/replicas.sh
. scripts/deploy-probes/replicas.sh
# shellcheck source=scripts/deploy-probes/hospital_smart.sh
. scripts/deploy-probes/hospital_smart.sh
# shellcheck source=scripts/deploy-probes/hospital.sh
. scripts/deploy-probes/hospital.sh
# shellcheck source=scripts/deploy-probes/hospital_pix.sh
. scripts/deploy-probes/hospital_pix.sh
# shellcheck source=scripts/deploy-probes/reset.sh
. scripts/deploy-probes/reset.sh

cleanup() {
  if [[ "$KEEP_UP" -eq 0 ]]; then
    compose_down
  else
    dim "── stack left running (--keep-up): $CDR"
  fi
  rm -rf "$PROBE_TMP"
}
trap cleanup EXIT

command -v docker >/dev/null || { red "docker not found on PATH"; exit 1; }
# The pinned ferrohealth-sop generates the probes' signing keys (scripts/sop.sh install).
PATH="$(bash "$(dirname "$0")/sop.sh" path):$PATH"

# The image the CDR container runs, read from the container once the stack is
# up: the reference it started from, the local image id, and the registry
# digests, empty for an image built on this host. The requested reference
# alone hid a probe of an image older than its tree (#5673), so the header and
# the record (`PROBE_IMAGE`, which lib.sh's probe_report writes) name what ran.
probe_image_identity() {
  local container ref id digests
  container="$(dc ps -q ferroehr 2>/dev/null)"
  container="${container%%$'\n'*}"
  if [[ -z "$container" ]]; then
    red "  ran:    no ferroehr container is running to read its image from"
    return 0
  fi
  ref="$(docker inspect --format '{{.Config.Image}}' "$container" 2>/dev/null)"
  id="$(docker inspect --format '{{.Image}}' "$container" 2>/dev/null)"
  digests="$(docker image inspect --format '{{join .RepoDigests " "}}' "$id" 2>/dev/null)"
  PROBE_IMAGE="$(printf '{"ref":"%s","id":"%s","digests":"%s"}' \
    "$(json_string "$ref")" "$(json_string "$id")" "$(json_string "$digests")")"
  echo "  ran:    $ref"
  echo "  id:     $id"
  echo "  digest: ${digests:-<none, built on this host>}"
}

bold "── deployment probes: compose ──────────────────────────────"
echo "  image:  ${FERROEHR_IMAGE:-<the compose default pin>} (the running image is read once the stack is up)"
echo "  cdr:    $CDR"
echo

# The documented recipe, used as documented: the multimedia keys are EXPORTED,
# not written into a file. #2169 was exactly this path silently doing nothing,
# so the harness must take it rather than a shortcut the book never mentions.
export FERROEHR__RECORD__MULTIMEDIA__ENABLED=true
export FERROEHR__RECORD__MULTIMEDIA__S3__URL=http://seaweedfs:8333
export FERROEHR__RECORD__MULTIMEDIA__S3__BUCKET=openehr-multimedia
# A probe instance sends no usage report; every family's stack reads this.
export FERROEHR__USAGE_REPORT__ENABLED=false

# The database image is built from THIS tree unless the caller names one, for
# the same reason the server image is: the compose default is the last
# published tag, and probing it measures the previous release rather than the
# change in hand. It matters here specifically because the image carries the
# `initdb` script that provisions the domain roles, and `initdb` runs once, on
# an empty data directory — so a pulled image can never show a role the tree
# added.
if [[ -z "${FERROEHR_POSTGRES_IMAGE:-}" ]]; then
  bold "building the database image from docker/postgres"
  docker build -q -t ferroehr-postgres:probe docker/postgres >/dev/null
  export FERROEHR_POSTGRES_IMAGE=ferroehr-postgres:probe
fi

compose_down
bold "bringing the stack up (postgres + CDR + seaweedfs + init + the Basic user's provisioning)"
# A login no demographic party carries is refused, so the Basic user is bound
# through SCIM before any probe calls with it.
compose_up ferroehr seaweedfs seaweedfs-init ferroehr-provision
probe_image_identity

# One family, or all of them. The terminology family needs a licensed release
# and ~8 GB, so its CI lane runs it alone rather than paying for every other
# family to prove something already proven on every pull request.
run_family() {
  local family="$1"
  [[ -z "${PROBE_ONLY:-}" ]] || [[ "${PROBE_ONLY}" = "$family" ]]
}

run_family shipped_config && probes_shipped_config_boots
run_family deployment_posture && probes_deployment_posture
run_family bootstrap && probes_bootstrap
run_family multimedia && probes_multimedia
run_family management && probes_management
run_family management_separate_listener && probes_management_separate_listener
run_family signing && probes_signing
run_family signing_pgp && probes_signing_pgp
run_family signing_rotation && probes_signing_rotation
run_family multimedia_restart && probes_multimedia_restart
run_family multimedia_off && probes_multimedia_off
run_family multimedia_broken && probes_multimedia_broken
run_family health_broken && probes_health_broken
run_family oidc && probes_oidc
run_family oidc_roles && probes_oidc_roles
run_family oidc && oidc_down
run_family domain_roles && probes_domain_roles
run_family backup_restore && probes_backup_restore
run_family observability && probes_observability
run_family terminology_ferroterm && probes_terminology_ferroterm
run_family terminology && probes_terminology
run_family replicas && probes_replicas
# The hospital scenario runs behind FerroSMART with an upstream provider, its
# own CI job per provider (deploy-probe-hospital), so only PROBE_ONLY=hospital
# runs it.
[[ "${PROBE_ONLY:-}" = hospital ]] && probes_hospital
# Last: it deletes the stack's volumes, which is the procedure under test.
run_family original_state && probes_original_state
run_family decommission && probes_decommission

# ── The honest half ───────────────────────────────────────────────────────────
# Everything #2178 asks for that this run does NOT do. Each entry is a probe
# somebody still has to write; none of them is silently absent.
uncovered "kubernetes platform" \
  "a separate harness covers it — scripts/deploy-probe-k8s.sh, recorded in kubernetes.json"
uncovered "viewer journeys (#2164)" \
  "scripts/ui-e2e.sh already drives these with a real browser; folding it in is the next step"


probe_report "$PROBE_OUT" "compose"
