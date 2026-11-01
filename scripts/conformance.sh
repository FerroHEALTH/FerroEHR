#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# openEHR conformance run: the Veredictum catalogue against one FerroEHR, and
# the measured performance lane beside it.
#
# Brings up the conformance deployment on fresh volumes (the exclusive-server
# ground the bulk-delete workflows need), reviews the party statement, drives
# `veredictum test` against it through the release image the pin in
# scripts/lib/veredictum.sh names, and writes the views `export` renders:
#
#   $OUT/results.json   one row per workflow row
#   $OUT/recording.har  every exchange (holds response bodies; gitignored,
#                       never published)
#   $OUT/junit.xml, ctrf.json, summary.md
#
# One run covers the `smart` documents too (#4503): it passes the SMART
# Platform base URL and the committed test issuer's key under
# docs/conformance/party/smart/, which the IXIT's `scoped_tokens.mint` signs
# with, as VEREDICTUM_SCOPED_TOKEN_KEY_FILE, as the Veredictum Action does. No
# token is written anywhere: the instrument mints each one in memory, and the
# recording carries none.
#
# The specification profile (CONF_PROFILE) is the text every case citation is
# read against. `released`, the frozen release, is the record of record and
# alone backs a released claim; `development`, the pinned master of each
# openEHR repository, is published beside it under its own directory and never
# mixed into the record.
#
# Usage:
#   scripts/conformance.sh [run]   compose, review, drive, export, tear down
#   scripts/conformance.sh review  review the party statement against the
#                                  catalogue, with no deployment
#   scripts/conformance.sh up      compose the deployment on fresh volumes and
#                                  leave it serving, for a driver of its own
#                                  (the conformance workflow runs the
#                                  Veredictum Action against it)
#   scripts/conformance.sh down    tear the deployment and its volumes down
#   scripts/conformance.sh perf    the measured lane: compose the deployment
#                                  with docker/sut-measurement.yml on fresh
#                                  volumes, run `veredictum perf` over the
#                                  pinned workload, re-derive the record with
#                                  `perf-judge` and redraw the book's charts
#                                  (scripts/render/perf-assets.sh)
#   scripts/conformance.sh perf-aa the same deployment measured against itself,
#                                  interleaved (`perf-compare --aa`), so the
#                                  record carries its between-run noise floor
#
# The measured lane runs the pinned binary on the host beside the cached clone
# of the tag (scripts/lib/veredictum.sh): `perf` settles the database with
# `docker exec` into the container the IXIT's `containers` block names. A
# published record is measured on the idle reference host its host
# prescription names (CONF_PERF_HOST); on any other host the record reads the
# prescription as unmatched, and a hosted runner only compares A/B (Veredictum
# book, Measured performance).
#
# The deployment runs FerroTERM beside FerroEHR (the `terminology` profile,
# pinned by digest in docker/sut-ferroehr.yml), and docker/sut-terminology.yml
# points `[terminology.server]` at it, so the AQL TERMINOLOGY() arm the
# statement selects runs against a real terminology server (#4476).
#
# Env:
#   CONF_PROFILE      released (default) or development.
#   CONF_OUT          artefact root (default: docs/conformance); `ferroehr`
#                     is appended for the released profile and
#                     `ferroehr-development` for the development profile.
#   CONF_DOCUMENTS    space-separated document ids to narrow the run to
#                     (`--document`); a narrowed run is a working tool, never
#                     the record.
#   CONF_NO_COMPOSE   if set, compose nothing and drive the deployment already
#                     serving CONF_BASE_URL (the bulk-delete workflows then run
#                     against whatever state it holds).
#   CONF_BASE_URL     the openEHR REST base (default: the composed deployment,
#                     http://localhost:8080/ferroehr/rest/openehr/v1).
#   CONF_PLATFORM_URL the SMART Platform base URL (default: the composed
#                     deployment's REST root, http://localhost:8080/ferroehr/rest,
#                     where it serves /.well-known/smart-configuration).
#   CONF_NETWORK      the network the instrument container joins (default:
#                     host, as the Veredictum Action runs it).
#   SKIP_BUILD        if set, compose up without --build (published image).
#   CONTAINER_ENGINE  docker or podman (scripts/lib/veredictum.sh).
#   CONF_PERF_WORKLOAD the pinned workload under perf/workloads/ the measured
#                     lane offers (default: clinical-mix).
#   CONF_PERF_POSTURE the posture `perf --posture` declares (default:
#                     minimal-signed-digest, the deployment's shipped signing
#                     with compression off, docker/sut-measurement.yml).
#   CONF_PERF_HOST    the host prescription under perf/hosts/ the record claims
#                     (default: reference-8cpu-8gb; empty claims none).
#   CONF_PERF_OUT     the published record (default:
#                     docs/conformance/ferroehr/perf-result.json).
#   VEREDICTUM_CDR_USER / VEREDICTUM_CDR_PASSWORD,
#   VEREDICTUM_CDR_ADMIN_USER / VEREDICTUM_CDR_ADMIN_PASSWORD
#                     the Basic logins (default: the users of
#                     docker/ferroehr.dev.toml).
#
# Exit status is the instrument's: 0 when every row passed, was skipped or
# failed only as a warning, 1 when a row failed or errored, 2 when the run
# could not start. The measured stages exit with the first failing command's
# status. No openEHR specification governs this pipeline or performance: our
# own design.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="${CONF_PROFILE:-released}"
case "$PROFILE" in
  released) OUT="${CONF_OUT:-$REPO_ROOT/docs/conformance}/ferroehr" ;;
  development) OUT="${CONF_OUT:-$REPO_ROOT/docs/conformance}/ferroehr-development" ;;
  *)
    echo "conformance: CONF_PROFILE is \"$PROFILE\" (accepted: released, development)" >&2
    exit 2
    ;;
esac
PARTY="$REPO_ROOT/docs/conformance/party/ferroehr"
STATEMENT="$PARTY/statement.json"
IXIT="$PARTY/ixit.json"
# The test issuer the IXIT's `scoped_tokens.mint` declares; the SMART overlay
# (docker/sut-smart.yml) trusts its public half.
SMART_KEY="$REPO_ROOT/docs/conformance/party/smart/cnf-smart-test.key.pem"
PLATFORM_URL="${CONF_PLATFORM_URL:-http://localhost:8080/ferroehr/rest}"
BASE_URL="${CONF_BASE_URL:-http://localhost:8080/ferroehr/rest/openehr/v1}"
NETWORK="${CONF_NETWORK:-host}"

STAGE="${1:-run}"
USAGE="usage: scripts/conformance.sh [run|review|up|down|perf|perf-aa]"
MEASUREMENT=0
case "$STAGE" in
  run | review | up | down) ;;
  perf | perf-aa) MEASUREMENT=1 ;;
  *)
    echo "$USAGE" >&2
    exit 2
    ;;
esac
[[ "$#" -le 1 ]] || { echo "$USAGE" >&2; exit 2; }

# shellcheck source=scripts/lib/veredictum.sh
source "$REPO_ROOT/scripts/lib/veredictum.sh"
# shellcheck source=scripts/lib/cnf-compose.sh
. "$REPO_ROOT/scripts/lib/cnf-compose.sh"

[[ -f "$IXIT" ]] || { echo "conformance: ixit not found: $IXIT" >&2; exit 2; }
[[ -f "$STATEMENT" ]] || { echo "conformance: statement not found: $STATEMENT" >&2; exit 2; }
[[ -f "$SMART_KEY" ]] || { echo "conformance: test issuer key not found: $SMART_KEY" >&2; exit 2; }

ENGINE="$(veredictum_engine)"

# The conformance deployment: docker/sut-ferroehr.yml with the SMART and
# terminology overlays, as its own compose project so it never tears down a dev
# stack, and with the measurement overlay for the measured stages. The IXIT's
# `containers` block names this project's containers. Paths inside the
# sut-*.yml files are repo-root-relative, hence `--project-directory`.
COMPOSE=("$ENGINE" compose -p ferroehr-cnf --project-directory "$REPO_ROOT" --profile terminology)
while IFS= read -r file; do
  COMPOSE+=(-f "$file")
done < <(cnf_compose_files "$MEASUREMENT" "$REPO_ROOT")

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
  echo "==> Reviewing the party statement against the catalogue ($PROFILE profile)"
  instrument --network none -- validate --root /work/catalogue --specs /work/specs/openehr \
    --profile "$PROFILE" --statement /work/input/statement.json
}

compose_down() {
  (cd "$REPO_ROOT" && "${COMPOSE[@]}" down -v) || true
}

compose_up() {
  # A conformance deployment sends no usage report, whatever the caller's shell holds.
  export FERROEHR__USAGE_REPORT__ENABLED=false
  if [[ "$MEASUREMENT" == 1 ]]; then
    echo "==> Composing FerroEHR and FerroTERM on fresh volumes, measurement posture"
  else
    echo "==> Composing FerroEHR and FerroTERM on fresh volumes (the exclusive-server ground)"
  fi
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

# The measured lane. `perf` writes the record, `perf-judge` re-derives every
# number of it from its histograms and fails on any differing byte, and the
# charts are drawn from the record as committed.
measure() {
  local bin perf_root catalogue workload record work
  bin="$(veredictum_bin)"
  perf_root="$(veredictum_perf)"
  catalogue="$(veredictum_catalogue)"
  workload="$(veredictum_workload "${CONF_PERF_WORKLOAD:-clinical-mix}")"
  [[ -f "$workload" ]] || { echo "conformance: no pinned workload at $workload" >&2; exit 2; }
  record="${CONF_PERF_OUT:-$REPO_ROOT/docs/conformance/ferroehr/perf-result.json}"
  mkdir -p "$(dirname "$record")"
  # The seeded corpus listing and an A/A run's two records stay out of the
  # tree: only the published record is committed.
  work="$(veredictum_cache_root)/runs/$(date -u +%Y%m%dT%H%M%SZ)"
  mkdir -p "$work"

  local -a run=(--root "$perf_root" --catalogue "$catalogue" --workload "$workload"
    --statement "$STATEMENT" --posture "${CONF_PERF_POSTURE:-minimal-signed-digest}"
    --network-leg same-host)
  local host="${CONF_PERF_HOST-reference-8cpu-8gb}"
  [[ -z "$host" ]] || run+=(--host-prescription "$host")

  echo "==> Host load before the measured window: $(uptime)"
  if [[ "$STAGE" == perf ]]; then
    echo "==> Measuring $(basename "$workload" .yaml) against $BASE_URL (record $record)"
    "$bin" perf --ixit "$IXIT" "${run[@]}" --corpus-state "$work/corpus.json" --out "$record"
  else
    echo "==> Measuring $(basename "$workload" .yaml) against itself, interleaved (work in $work)"
    "$bin" perf-compare --aa "$IXIT" "${run[@]}" --out "$work"
    cp "$work/a.perf-result.json" "$record"
    cp "$work/perf-compare.md" "$(dirname "$record")/perf-compare-aa.md"
  fi
  echo "==> Re-deriving the record (perf-judge)"
  "$bin" perf-judge --result "$record"
  bash "$REPO_ROOT/scripts/render/perf-assets.sh"
  echo "==> Record: $record (work files in $work)"
}

case "$STAGE" in
  perf | perf-aa)
    [[ -z "${CONF_NO_COMPOSE:-}" ]] || { echo "conformance: the measured stages compose their own deployment; CONF_NO_COMPOSE is refused" >&2; exit 2; }
    trap compose_down EXIT
    compose_up
    measure
    exit 0
    ;;
  review)
    pull_instrument
    review_statement
    exit 0
    ;;
  up)
    compose_up
    exit 0
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

test_args=(test --base-url "$BASE_URL" --platform-url "$PLATFORM_URL" --profile "$PROFILE"
  --auth basic --admin-auth basic
  --statement /work/input/statement.json --ixit /work/input/ixit.json --out /work/out)
read -r -a documents <<< "${CONF_DOCUMENTS:-}"
for document in ${documents[@]+"${documents[@]}"}; do
  test_args+=(--document "$document")
done

echo "==> Driving the catalogue against $BASE_URL (Platform $PLATFORM_URL, $PROFILE profile)"
test_rc=0
instrument --network "$NETWORK" --user "$(id -u):$(id -g)" -v "$OUT:/work/out" \
  -v "$SMART_KEY:/work/secrets/smart-issuer.key.pem:ro" \
  -e VEREDICTUM_SCOPED_TOKEN_KEY_FILE=/work/secrets/smart-issuer.key.pem \
  -e VEREDICTUM_CDR_USER -e VEREDICTUM_CDR_PASSWORD \
  -e VEREDICTUM_CDR_ADMIN_USER -e VEREDICTUM_CDR_ADMIN_PASSWORD \
  -- "${test_args[@]}" || test_rc=$?
if [[ ! -f "$OUT/results.json" ]]; then
  echo "conformance: the run wrote no results (exit $test_rc)" >&2
  exit 2
fi

export_views "$OUT"

echo "==> Artefacts in $OUT ($PROFILE profile, instrument exit $test_rc)"
ls -1 "$OUT"
exit "$test_rc"
