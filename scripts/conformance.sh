#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# openEHR conformance run: the Veredictum catalogue against one FerroEHR.
#
# Brings up the conformance deployment on fresh volumes (the exclusive-server
# ground the bulk-delete workflows need), reviews the party statement, drives
# `veredictum test` against it through the release image the pin in
# scripts/lib/veredictum.sh names, and writes the views `export` renders:
#
#   $CONF_OUT/ferroehr/results.json   one row per workflow row
#   $CONF_OUT/ferroehr/recording.har  every exchange (holds response bodies;
#                                     gitignored, never published)
#   $CONF_OUT/ferroehr/junit.xml, ctrf.json, summary.md
#
# Usage:
#   scripts/conformance.sh
#
# Env:
#   CONF_OUT          artefact root (default: docs/conformance; `ferroehr` is
#                     appended).
#   CONF_DOCUMENTS    space-separated document ids to narrow the run to
#                     (`--document`); a narrowed run is a working tool, never
#                     the record.
#   CONF_NO_COMPOSE   if set, compose nothing and drive the deployment already
#                     serving CONF_BASE_URL (the bulk-delete workflows then run
#                     against whatever state it holds).
#   CONF_BASE_URL     the openEHR REST base (default: the composed deployment,
#                     http://localhost:8080/ferroehr/rest/openehr/v1).
#   CONF_NETWORK      the network the instrument container joins (default:
#                     host, as the Veredictum Action runs it).
#   SKIP_BUILD        if set, compose up without --build (published image).
#   CONTAINER_ENGINE  docker or podman (scripts/lib/veredictum.sh).
#   VEREDICTUM_CDR_USER / VEREDICTUM_CDR_PASSWORD,
#   VEREDICTUM_CDR_ADMIN_USER / VEREDICTUM_CDR_ADMIN_PASSWORD
#                     the Basic logins (default: the users of
#                     docker/ferroehr.dev.toml).
#
# Exit status is the instrument's: 0 when every row passed, was skipped or
# failed only as advisory, 1 when a row failed or errored, 2 when the run could
# not start. No openEHR specification governs this pipeline: our own design.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${CONF_OUT:-$REPO_ROOT/docs/conformance}/ferroehr"
PARTY="$REPO_ROOT/docs/conformance/party/ferroehr"
STATEMENT="$PARTY/statement.json"
IXIT="$PARTY/ixit.json"
BASE_URL="${CONF_BASE_URL:-http://localhost:8080/ferroehr/rest/openehr/v1}"
NETWORK="${CONF_NETWORK:-host}"

# shellcheck source=scripts/lib/veredictum.sh
source "$REPO_ROOT/scripts/lib/veredictum.sh"
# shellcheck source=scripts/lib/cnf-compose.sh
. "$REPO_ROOT/scripts/lib/cnf-compose.sh"

if [[ -n "${CONF_PERF_CLASS:-}" ]] || [[ -n "${CONF_STRESS:-}" ]]; then
  echo "conformance: the measured lanes move to the Veredictum perf tree (#4452); this script runs the catalogue only" >&2
  exit 2
fi

[[ -f "$IXIT" ]] || { echo "conformance: ixit not found: $IXIT" >&2; exit 2; }
[[ -f "$STATEMENT" ]] || { echo "conformance: statement not found: $STATEMENT" >&2; exit 2; }

ENGINE="$(veredictum_engine)"

# The conformance deployment: docker/sut-ferroehr.yml with the SMART and
# terminology overlays, as its own compose project so it never tears down a dev
# stack. Paths inside the sut-*.yml files are repo-root-relative, hence
# `--project-directory`.
COMPOSE=("$ENGINE" compose -p ferroehr-cnf --project-directory "$REPO_ROOT" --profile terminology)
while IFS= read -r file; do
  COMPOSE+=(-f "$file")
done < <(cnf_compose_files 0 "$REPO_ROOT")

# Build provenance for the compose-built image (the OCI REVISION build arg).
export REVISION="${REVISION:-$(git -C "$REPO_ROOT" rev-parse --short=12 HEAD 2>/dev/null || echo unknown)}"

# The Basic logins of docker/ferroehr.dev.toml: a USER and an ADMIN principal.
export VEREDICTUM_CDR_USER="${VEREDICTUM_CDR_USER:-ferroehr}"
export VEREDICTUM_CDR_PASSWORD="${VEREDICTUM_CDR_PASSWORD:-ferroehr}"
export VEREDICTUM_CDR_ADMIN_USER="${VEREDICTUM_CDR_ADMIN_USER:-ferroehr-admin}"
export VEREDICTUM_CDR_ADMIN_PASSWORD="${VEREDICTUM_CDR_ADMIN_PASSWORD:-ferroehr}"

# Runs the pinned engine in the release image with the party set mounted
# read-only; the caller adds its own mounts and network before the image.
instrument() {
  local -a opts=()
  while [[ $# -gt 0 ]] && [[ "$1" != -- ]]; do
    opts+=("$1")
    shift
  done
  shift
  "$ENGINE" run --rm "${opts[@]}" \
    -v "$STATEMENT:/work/input/statement.json:ro" \
    -v "$IXIT:/work/input/ixit.json:ro" \
    --entrypoint /usr/local/bin/veredictum "$VEREDICTUM_IMAGE" "$@"
}

echo "==> Pulling the pinned instrument (veredictum $VEREDICTUM_VERSION)"
"$ENGINE" pull --quiet "$VEREDICTUM_IMAGE" >/dev/null
reported="$("$ENGINE" run --rm --entrypoint /usr/local/bin/veredictum "$VEREDICTUM_IMAGE" --version)"
[[ "$reported" == "veredictum $VEREDICTUM_VERSION ("* ]] || {
  echo "conformance: the pinned image reports '$reported', not veredictum $VEREDICTUM_VERSION" >&2
  exit 2
}

echo "==> Reviewing the party statement against the catalogue"
instrument --network none -- validate --root /work/catalogue --specs /work/specs/openehr \
  --statement /work/input/statement.json

compose_down() {
  (cd "$REPO_ROOT" && "${COMPOSE[@]}" down -v) || true
}

if [[ -z "${CONF_NO_COMPOSE:-}" ]]; then
  # A conformance deployment sends no usage report, whatever the caller's shell holds.
  export FERROEHR__USAGE_REPORT__ENABLED=false
  trap compose_down EXIT
  echo "==> Composing FerroEHR on fresh volumes (the exclusive-server ground)"
  compose_down
  if [[ -n "${SKIP_BUILD:-}" ]]; then
    (cd "$REPO_ROOT" && "${COMPOSE[@]}" up -d --wait ferroehr)
  else
    # A verdict on this server only means something against the current
    # sources, so the image is built unless explicitly skipped.
    (cd "$REPO_ROOT" && "${COMPOSE[@]}" up -d --build --wait ferroehr)
  fi
fi

mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd -P)"
rm -f "$OUT/results.json" "$OUT/recording.har" "$OUT/junit.xml" "$OUT/ctrf.json" "$OUT/summary.md"

test_args=(test --base-url "$BASE_URL" --auth basic --admin-auth basic
  --statement /work/input/statement.json --ixit /work/input/ixit.json --out /work/out)
read -r -a documents <<< "${CONF_DOCUMENTS:-}"
for document in ${documents[@]+"${documents[@]}"}; do
  test_args+=(--document "$document")
done

echo "==> Driving the catalogue against $BASE_URL"
test_rc=0
instrument --network "$NETWORK" --user "$(id -u):$(id -g)" -v "$OUT:/work/out" \
  -e VEREDICTUM_CDR_USER -e VEREDICTUM_CDR_PASSWORD \
  -e VEREDICTUM_CDR_ADMIN_USER -e VEREDICTUM_CDR_ADMIN_PASSWORD \
  -- "${test_args[@]}" || test_rc=$?
if [[ ! -f "$OUT/results.json" ]]; then
  echo "conformance: the run wrote no results (exit $test_rc)" >&2
  exit 2
fi

echo "==> Exporting the JUnit, CTRF and Markdown views"
instrument --network none --user "$(id -u):$(id -g)" -v "$OUT:/work/out" -- \
  export --results /work/out/results.json --junit /work/out/junit.xml \
  --ctrf /work/out/ctrf.json --markdown /work/out/summary.md

echo "==> Artefacts in $OUT (instrument exit $test_rc)"
ls -1 "$OUT"
exit "$test_rc"
