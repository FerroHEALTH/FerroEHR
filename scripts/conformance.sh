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
# then drives the `smart` documents in a second `test` of the same image
# (#4450), with the SMART Platform base URL and tokens it mints from the
# committed test issuer under docs/conformance/party/smart/, which the
# Veredictum Action forwards neither of, and writes the same views under
# $CONF_SMART_OUT. No token is written anywhere: the instrument mints each one
# in memory for the run, and the recording carries none.
#
# Usage:
#   scripts/conformance.sh [run]   compose, review, drive, export, tear down
#   scripts/conformance.sh review  review the party statement against the
#                                  catalogue, with no deployment
#   scripts/conformance.sh up      compose the deployment on fresh volumes and
#                                  leave it serving, for a driver of its own
#                                  (the conformance workflow runs the
#                                  Veredictum Action against it)
#   scripts/conformance.sh smart   drive the `smart` documents against the
#                                  deployment already serving (the conformance
#                                  workflow runs it after the Action)
#   scripts/conformance.sh down    tear the deployment and its volumes down
#
# The deployment runs FerroTERM beside FerroEHR (the `terminology` profile,
# pinned by digest in docker/sut-ferroehr.yml), and docker/sut-terminology.yml
# points `[terminology.server]` at it, so the AQL TERMINOLOGY() arm the
# statement selects runs against a real terminology server (#4476).
#
# Env:
#   CONF_OUT          artefact root (default: docs/conformance; `ferroehr` is
#                     appended).
#   CONF_DOCUMENTS    space-separated document ids to narrow the run to
#                     (`--document`); a narrowed run is a working tool, never
#                     the record, and drives no SMART run.
#   CONF_NO_COMPOSE   if set, compose nothing and drive the deployment already
#                     serving CONF_BASE_URL (the bulk-delete workflows then run
#                     against whatever state it holds).
#   CONF_BASE_URL     the openEHR REST base (default: the composed deployment,
#                     http://localhost:8080/ferroehr/rest/openehr/v1).
#   CONF_SMART_OUT    where the SMART run writes its views (default:
#                     the artefact root with `ferroehr-smart` appended).
#   CONF_PLATFORM_URL the SMART Platform base URL (default: the composed
#                     deployment's REST root, http://localhost:8080/ferroehr/rest,
#                     where it serves /.well-known/smart-configuration).
#   CONF_NETWORK      the network the instrument container joins (default:
#                     host, as the Veredictum Action runs it).
#   SKIP_BUILD        if set, compose up without --build (published image).
#   CONTAINER_ENGINE  docker or podman (scripts/lib/veredictum.sh).
#   VEREDICTUM_CDR_USER / VEREDICTUM_CDR_PASSWORD,
#   VEREDICTUM_CDR_ADMIN_USER / VEREDICTUM_CDR_ADMIN_PASSWORD
#                     the Basic logins (default: the users of
#                     docker/ferroehr.dev.toml).
#
# Exit status is the instrument's, the worse of the two runs: 0 when every row passed, was skipped or
# failed only as advisory, 1 when a row failed or errored, 2 when the run could
# not start. No openEHR specification governs this pipeline: our own design.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${CONF_OUT:-$REPO_ROOT/docs/conformance}/ferroehr"
SMART_OUT="${CONF_SMART_OUT:-${CONF_OUT:-$REPO_ROOT/docs/conformance}/ferroehr-smart}"
PARTY="$REPO_ROOT/docs/conformance/party/ferroehr"
STATEMENT="$PARTY/statement.json"
IXIT="$PARTY/ixit.json"
# The SMART run's IXIT declares the test issuer it mints with
# (`scoped_tokens.mint`); the Action's run must not read it, because every
# issuer it declares needs the key in the run's environment.
SMART_IXIT="$PARTY/ixit-smart.json"
SMART_KEY="$REPO_ROOT/docs/conformance/party/smart/cnf-smart-test.key.pem"
PLATFORM_URL="${CONF_PLATFORM_URL:-http://localhost:8080/ferroehr/rest}"
BASE_URL="${CONF_BASE_URL:-http://localhost:8080/ferroehr/rest/openehr/v1}"
NETWORK="${CONF_NETWORK:-host}"

STAGE="${1:-run}"
case "$STAGE" in
  run | review | up | smart | down) ;;
  *)
    echo "usage: scripts/conformance.sh [run|review|up|smart|down]" >&2
    exit 2
    ;;
esac
[[ "$#" -le 1 ]] || { echo "usage: scripts/conformance.sh [run|review|up|smart|down]" >&2; exit 2; }

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
[[ -f "$SMART_IXIT" ]] || { echo "conformance: ixit not found: $SMART_IXIT" >&2; exit 2; }
[[ -f "$SMART_KEY" ]] || { echo "conformance: test issuer key not found: $SMART_KEY" >&2; exit 2; }

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

# Pulls the pinned image and refuses one whose engine reports another release.
pull_instrument() {
  echo "==> Pulling the pinned instrument (veredictum $VEREDICTUM_VERSION)"
  "$ENGINE" pull --quiet "$VEREDICTUM_IMAGE" >/dev/null
  local reported
  reported="$("$ENGINE" run --rm --entrypoint /usr/local/bin/veredictum "$VEREDICTUM_IMAGE" --version)"
  [[ "$reported" == "veredictum $VEREDICTUM_VERSION ("* ]] || {
    echo "conformance: the pinned image reports '$reported', not veredictum $VEREDICTUM_VERSION" >&2
    exit 2
  }
}

review_statement() {
  echo "==> Reviewing the party statement against the catalogue"
  instrument --network none -- validate --root /work/catalogue --specs /work/specs/openehr \
    --statement /work/input/statement.json
}

compose_down() {
  (cd "$REPO_ROOT" && "${COMPOSE[@]}" down -v) || true
}

compose_up() {
  # A conformance deployment sends no usage report, whatever the caller's shell holds.
  export FERROEHR__USAGE_REPORT__ENABLED=false
  echo "==> Composing FerroEHR and FerroTERM on fresh volumes (the exclusive-server ground)"
  compose_down
  if [[ -n "${SKIP_BUILD:-}" ]]; then
    (cd "$REPO_ROOT" && "${COMPOSE[@]}" up -d --wait ferroehr)
  else
    # A verdict on this server only means something against the current
    # sources, so the image is built unless explicitly skipped.
    (cd "$REPO_ROOT" && "${COMPOSE[@]}" up -d --build --wait ferroehr)
  fi
}

# Writes the JUnit, CTRF and Markdown views beside the results in $1.
export_views() {
  echo "==> Exporting the JUnit, CTRF and Markdown views"
  instrument --network none --user "$(id -u):$(id -g)" -v "$1:/work/out" -- \
    export --results /work/out/results.json --junit /work/out/junit.xml \
    --ctrf /work/out/ctrf.json --markdown /work/out/summary.md
}

# Drives the `smart` documents against the deployment serving BASE_URL and
# PLATFORM_URL, with the tokens the instrument mints from the committed test
# issuer the SMART overlay (docker/sut-smart.yml) trusts. Sets SMART_RC to the
# instrument's exit status; exits 2 when the run wrote no results.
drive_smart() {
  mkdir -p "$SMART_OUT"
  SMART_OUT="$(cd "$SMART_OUT" && pwd -P)"
  rm -f "$SMART_OUT/results.json" "$SMART_OUT/recording.har" "$SMART_OUT/junit.xml" \
    "$SMART_OUT/ctrf.json" "$SMART_OUT/summary.md"
  echo "==> Driving the SMART documents against $BASE_URL (Platform $PLATFORM_URL)"
  SMART_RC=0
  instrument --network "$NETWORK" --user "$(id -u):$(id -g)" -v "$SMART_OUT:/work/out" \
    -v "$SMART_IXIT:/work/input/ixit-smart.json:ro" \
    -v "$SMART_KEY:/work/input/smart-issuer.key.pem:ro" \
    -e VEREDICTUM_SCOPED_TOKEN_KEY_FILE=/work/input/smart-issuer.key.pem \
    -e VEREDICTUM_CDR_USER -e VEREDICTUM_CDR_PASSWORD \
    -- test --base-url "$BASE_URL" --platform-url "$PLATFORM_URL" --auth basic \
    --statement /work/input/statement.json --ixit /work/input/ixit-smart.json \
    --document smart_discovery --document smart_scopes --out /work/out || SMART_RC=$?
  if [[ ! -f "$SMART_OUT/results.json" ]]; then
    echo "conformance: the SMART run wrote no results (exit $SMART_RC)" >&2
    exit 2
  fi
  export_views "$SMART_OUT"
  echo "==> SMART artefacts in $SMART_OUT (instrument exit $SMART_RC)"
}

case "$STAGE" in
  review)
    pull_instrument
    review_statement
    exit 0
    ;;
  up)
    compose_up
    exit 0
    ;;
  smart)
    pull_instrument
    drive_smart
    exit "$SMART_RC"
    ;;
  down)
    compose_down
    exit 0
    ;;
esac

pull_instrument
review_statement

if [[ -z "${CONF_NO_COMPOSE:-}" ]]; then
  trap compose_down EXIT
  compose_up
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

export_views "$OUT"

echo "==> Artefacts in $OUT (instrument exit $test_rc)"
ls -1 "$OUT"

if [[ "${#documents[@]}" -eq 0 ]]; then
  drive_smart
  # 2 (could not run) outranks 1 (a failed row), which outranks 0.
  [[ "$SMART_RC" -le "$test_rc" ]] || test_rc="$SMART_RC"
fi
exit "$test_rc"
