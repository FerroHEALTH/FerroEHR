#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# Boot every committed values overlay against a real ferroehr image.
#
# WHY this exists separately from deploy/helm/validate.sh: that script proves a
# render is well-formed, hardened and byte-stable, and none of those properties
# imply the server ACCEPTS what was rendered. Issue #2159 is the whole argument —
# all three overlays rendered clean, passed lint, matched their goldens, and not
# one of them produced a configuration the server would start on. The only thing
# that can tell you is the server, so this runs it.
#
# What it replays, per overlay — the complete delivery surface, not just the file:
#   * the ConfigMap's ferroehr.toml (and, when the configuration holds a secret
#     the chart cannot route, the Secret's copy instead) mounted at /etc/ferroehr;
#   * every `config.files` entry beside it;
#   * every file-shaped key of the chart's Secret, with its REAL rendered value,
#     mounted at /etc/ferroehr-secrets — an overlay's own credential fixtures are
#     the thing under test (a 15-byte HMAC secret is a boot failure, and a
#     placeholder written in its place would hide exactly that);
#   * every FERROEHR__* environment variable the Deployment declares, resolving
#     `valueFrom.secretKeyRef` against the chart's Secret.
# The one value that is synthesized is the DSN, which by design comes from a
# Secret the operator owns and this repository therefore does not carry.
#
# What it does NOT do: reach a database, an issuer, a broker or an object store.
# `config check` loads, merges and validates; it opens no socket. A live install
# is the k8s-test skill (.claude/skills/k8s-test/).
#
# Usage:
#   deploy/helm/ci/boot-check.sh                       # every deployment overlay
#   deploy/helm/ci/boot-check.sh path/to/values.yaml   # just this one
#   FERROEHR_IMAGE=ferroehr:local deploy/helm/ci/boot-check.sh
#
# FERROEHR_SKEW_UNSET is the one escape, for judging the chart against a
# RELEASED image between releases: a comma-separated list of `config.*` paths to
# unset, for keys the chart carries that the image predates. It is loud, it is
# never used by CI (which builds the image from the tree, so no key can be too
# new), and it can only REMOVE configuration — it cannot make a bad value pass.
#   FERROEHR_IMAGE=ghcr.io/ferrohealth/ferroehr:3.17.5 \
#   FERROEHR_SKEW_UNSET=config.db.migrate deploy/helm/ci/boot-check.sh
#
# Requires docker and helm. The CI lane is `chart-boot` in .github/workflows/ci.yml.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHART_DIR="$(cd "${SCRIPT_DIR}/../ferroehr" && pwd)"
CI_DIR="$SCRIPT_DIR"
IMAGE="${FERROEHR_IMAGE:-ghcr.io/ferrohealth/ferroehr:main}"
RELEASE_NAME="ferroehr"
NAMESPACE="ferroehr"

# Overlays that are NOT deployments and are therefore not booted. Each one is
# named with its reason: an unaccounted values file is an error below, so a new
# overlay cannot be added and silently skipped — which is the recurrence this
# whole lane exists to prevent.
#
#   secret-leak-values.yaml — a search fixture for validate.sh's secret-leak
#   gate. Its values are deliberately unusable sentinels (`$argon2id$SENTINEL…`
#   is not a PHC string), because the gate greps for them; making it bootable
#   would mean giving it real credentials and losing the attribution the
#   sentinels buy.
declare -a NOT_A_DEPLOYMENT=(
  "secret-leak-values.yaml"
)

declare -a SKEW_ARGS=()
if [[ -n "${FERROEHR_SKEW_UNSET:-}" ]]; then
  IFS=',' read -r -a SKEW_PATHS <<< "$FERROEHR_SKEW_UNSET"
  for skew_path in "${SKEW_PATHS[@]}"; do
    SKEW_ARGS+=(--set "${skew_path}=null")
  done
fi

red()   { printf '\033[31m%s\033[0m\n' "$*"; }
green() { printf '\033[32m%s\033[0m\n' "$*"; }
bold()  { printf '\033[1m%s\033[0m\n' "$*"; }

command -v helm   >/dev/null 2>&1 || { red "helm not found on PATH";   exit 1; }
command -v docker >/dev/null 2>&1 || { red "docker not found on PATH"; exit 1; }

# ── Split a rendered Secret/ConfigMap into one file per key ──────────────────
# Handles both scalar (`key: "value"`) and block (`key: |`) entries, keyed by the
# object's metadata.name so an external Secret is never confused with the
# chart's own. There is no Python in this repo and no yq on a bare runner.
split_data() {
  local rendered="$1" want_name="$2" outdir="$3"
  mkdir -p "$outdir"
  awk -v want="$want_name" -v outdir="$outdir" '
    function unquote(s) {
      if (s ~ /^".*"$/) {
        s = substr(s, 2, length(s) - 2)
        gsub(/\\"/, "\"", s)
        gsub(/\\\\/, "\\", s)
      }
      return s
    }
    /^---[[:space:]]*$/ { name = ""; indata = 0; key = ""; next }
    /^  name: / && !indata { name = $2; next }
    /^(stringData|data):[[:space:]]*$/ { indata = (name == want); key = ""; next }
    # Any other top-level key closes the data block.
    /^[^[:space:]]/ { indata = 0; key = "" }
    indata {
      # A comment inside the data block is not a key. Only inside: a comment in
      # a block scalar is content and is indented past this test.
      if (!block && $0 ~ /^  #/) { next }
      if (match($0, /^  [^ #][^:]*:/)) {
        k = substr($0, 3, RLENGTH - 3)
        rest = substr($0, RLENGTH + 1)
        sub(/^[[:space:]]+/, "", rest)
        if (rest == "|" || rest == "|-" || rest == "|+" || rest == ">" || rest == ">-") {
          key = k; printf "" > (outdir "/" key)
          block = 1
        } else {
          key = k; block = 0
          printf "%s", unquote(rest) > (outdir "/" key)
          close(outdir "/" key)
          key = ""
        }
        next
      }
      if (key != "" && block && match($0, /^    /)) {
        print substr($0, 5) >> (outdir "/" key)
        next
      }
      if (key != "") { close(outdir "/" key); key = "" }
    }
  ' "$rendered"
}

boot_one() {
  local values="$1"
  local label; label="$(basename "$values" .yaml)"
  bold "── ${label} ──────────────────────────────────────────────"

  local work; work="$(mktemp -d)"
  local cfgdir="${work}/etc-ferroehr"
  local secdir="${work}/etc-ferroehr-secrets"
  mkdir -p "$cfgdir" "$secdir"

  local all="${work}/all.yaml"
  if ! helm template "$RELEASE_NAME" "$CHART_DIR" -n "$NAMESPACE" -f "$values" \
       ${SKEW_ARGS[@]+"${SKEW_ARGS[@]}"} > "$all" 2>"${work}/render.err"; then
    red "  render FAILED — this is a chart/values error, not a boot failure:"
    sed 's/^/    /' "${work}/render.err"
    return 1
  fi

  # The env replay below must see the CDR Deployment's environment and NOTHING
  # else. The chart renders a second workload (the viewer, an OPTIONAL
  # separate binary with its own config root and its own `FERROEHR_VIEWER__…`
  # grammar — app/ferroehr-viewer/src/config.rs), and handing one workload's
  # environment to another image is not a boot check of anything: the CDR's
  # strict sweep refuses `FERROEHR_VIEWER__CDR__BASE_URL` as an unknown key and
  # reports a crash-loop for a deployment that runs correctly.
  local cdr="${work}/cdr-deployment.yaml"
  if ! helm template "$RELEASE_NAME" "$CHART_DIR" -n "$NAMESPACE" -f "$values" \
       ${SKEW_ARGS[@]+"${SKEW_ARGS[@]}"} --show-only templates/deployment.yaml \
       > "$cdr" 2>/dev/null; then
    red "  the chart rendered no templates/deployment.yaml — nothing to boot"
    return 1
  fi

  # /etc/ferroehr — the ConfigMap's ferroehr.toml, or the Secret's copy when the
  # configuration holds something the chart could not route out of it.
  split_data "$all" "$RELEASE_NAME" "$cfgdir"
  split_data "$all" "${RELEASE_NAME}-config" "$cfgdir"
  if [[ ! -s "${cfgdir}/ferroehr.toml" ]]; then
    red "  no ferroehr.toml was rendered by either the ConfigMap or the config Secret"
    return 1
  fi

  # /etc/ferroehr-secrets — the chart's own file-shaped secret keys, at their
  # real rendered values. A `FERROEHR__…`-spelled key is environment, not a file.
  split_data "$all" "${RELEASE_NAME}-env" "$secdir"
  local f
  for f in "$secdir"/FERROEHR__*; do
    [[ -e "$f" ]] && rm -f "$f"
  done

  # The Deployment's declared environment. `value:` literals pass through;
  # `valueFrom.secretKeyRef` resolves against the chart Secret already split
  # above, so a key that only exists as environment still arrives.
  #
  # Any OTHER env source is refused rather than ignored: the strict boot-time
  # sweep refuses an unknown FERROEHR_ variable, so an unreplayed source is a
  # variable this check never presents — a silent hole of exactly the kind that
  # let #2159 through. Adding a source here means teaching this parser about it.
  if grep -qE '^ +(envFrom:|.*(configMapKeyRef|fieldRef|resourceFieldRef):)' "$cdr"; then
    red "  the Deployment declares an environment source this check cannot replay"
    red "  (envFrom / configMapKeyRef / fieldRef); teach boot-check.sh about it"
    return 1
  fi
  local envlist="${work}/env"
  awk '
    /^          env:/ { inenv = 1; next }
    inenv && /^          [a-zA-Z]/ { inenv = 0 }
    !inenv { next }
    /^            - name: / { name = $3; pending = "" ; next }
    /^              value: / {
      v = $0; sub(/^              value: /, "", v)
      if (v ~ /^".*"$/) { v = substr(v, 2, length(v) - 2) }
      print name "\tliteral\t" v; name = ""; next
    }
    /^                  key: / { if (name != "") { print name "\tsecret\t" $2; name = "" } }
  ' "$cdr" > "$envlist"

  local -a docker_env=()
  local name kind value
  while IFS=$'\t' read -r name kind value; do
    [[ -n "$name" ]] || continue
    case "$kind" in
      literal) docker_env+=(-e "${name}=${value}") ;;
      secret)
        if [[ -f "${secdir}/${value}" ]]; then
          docker_env+=(-e "${name}=$(cat "${secdir}/${value}")")
          rm -f "${secdir}/${value}"
        else
          # An EXTERNAL Secret the operator owns; the chart never rendered it.
          docker_env+=(-e "${name}=ci-boot-check-external-secret-value")
        fi
        ;;
      *)
        red "  ${name}: unknown env source '${kind}'; teach boot-check.sh about it"
        return 1
        ;;
    esac
  done < "$envlist"

  # Every `*_FILE` path the environment or the TOML points at must exist. The
  # DSN is the one the chart deliberately does not carry.
  local paths
  paths="$( { grep -oE '/etc/ferroehr-secrets/[A-Za-z0-9._-]+' "$envlist" "${cfgdir}/ferroehr.toml" || true; } | sed 's#.*/etc/ferroehr-secrets/#/etc/ferroehr-secrets/#' | sort -u)"
  local p base
  for p in $paths; do
    base="$(basename "$p")"
    [[ -e "${secdir}/${base}" ]] && continue
    case "$base" in
      db.url) printf 'postgres://ferroehr_clinical:pw@postgres:5432/ferroehr' > "${secdir}/${base}" ;;
      # A storage domain's own DSN (#3343). Without a stand-in here, no ci
      # overlay could set database.<domain>.existingSecret without failing this
      # check, so the branch that gives each domain its own credential would
      # stay unexercised by construction.
      storage.clinical.url|storage.party.url|storage.linkage.url|storage.audit.url)
        printf 'postgres://ferroehr_%s:pw@postgres:5432/ferroehr' \
          "$(printf '%s' "$base" | cut -d. -f2)" > "${secdir}/${base}" ;;
      # The credential that prepares the schema (#3224), for the same reason:
      # an overlay setting database.migrateExistingSecret would otherwise fail
      # this check, leaving the branch that separates preparation from the
      # runtime credentials unexercised.
      db.migrate_url)
        printf 'postgres://ferroehr_migrator:pw@postgres:5432/ferroehr' > "${secdir}/${base}" ;;
      *)
        red "  ${p} is referenced but no rendered Secret key supplies it"
        return 1
        ;;
    esac
  done

  echo "  config files:  $(find "$cfgdir" -type f -exec basename {} \; | tr '\n' ' ')"
  echo "  secret files:  $(find "$secdir" -type f -exec basename {} \; | tr '\n' ' ')"
  echo "  environment:   $(cut -f1 "$envlist" | tr '\n' ' ')"

  local log="${work}/check.log"
  # `${a[@]+"${a[@]}"}`: macOS ships bash 3.2, where expanding an EMPTY array
  # under `set -u` is an unbound-variable error rather than nothing.
  if docker run --rm \
       -v "${cfgdir}:/etc/ferroehr:ro" \
       -v "${secdir}:/etc/ferroehr-secrets:ro" \
       ${docker_env[@]+"${docker_env[@]}"} \
       --entrypoint /usr/local/bin/ferroehr "$IMAGE" config check > "$log" 2>&1; then
    green "  ${IMAGE} accepts ${label}"
    sed 's/^/    /' "$log"
    rm -rf "$work"
    return 0
  fi
  red "  ${IMAGE} REFUSES ${label} — a deployment with these values crash-loops:"
  sed 's/^/    /' "$log"
  if grep -q 'unknown configuration key' "$log"; then
    red "  'unknown configuration key' is usually SKEW, not a bad overlay: the chart"
    red "  carries a key this image predates. Re-run against an image built from this"
    red "  tree before changing the chart."
  fi
  red "  rendered configuration kept at ${work}"
  return 1
}

bold "image: ${IMAGE}"
if [[ "${#SKEW_ARGS[@]}" -gt 0 ]]; then
  red "SKEW MODE: unsetting ${FERROEHR_SKEW_UNSET} — those keys are NOT judged."
  red "  Only correct against an image that genuinely predates them; CI never does this."
fi
docker image inspect "$IMAGE" >/dev/null 2>&1 || docker pull -q "$IMAGE" >/dev/null || {
  red "could not obtain ${IMAGE}; set FERROEHR_IMAGE to an image that exists"
  exit 1
}

declare -a TARGETS=()
if [[ $# -gt 0 ]]; then
  TARGETS=("$@")
else
  # Enumerate rather than list: a values file added to ci/ is booted, or is
  # declared not-a-deployment above with a reason. Nothing is skipped silently.
  for f in "$CI_DIR"/*-values.yaml; do
    skip=0
    for excluded in "${NOT_A_DEPLOYMENT[@]}"; do
      [[ "$(basename "$f")" == "$excluded" ]] && skip=1
    done
    [[ "$skip" -eq 1 ]] || TARGETS+=("$f")
  done
  echo "not booted (declared not a deployment): ${NOT_A_DEPLOYMENT[*]}"
fi
[[ "${#TARGETS[@]}" -gt 0 ]] || { red "no values files to boot"; exit 1; }

FAILED=()
for values in "${TARGETS[@]}"; do
  boot_one "$values" || FAILED+=("$(basename "$values")")
done

echo
if [[ "${#FAILED[@]}" -eq 0 ]]; then
  green "ALL ${#TARGETS[@]} OVERLAY(S) BOOT"
  exit 0
fi
red "REFUSED BY THE SERVER: ${FAILED[*]}"
exit 1
