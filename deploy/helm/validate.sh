#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# Validate the ferroehr Helm chart:
#   1. helm lint      — default + all-features value sets (must be clean)
#   2. helm template  — render both; assert the output is valid multi-doc YAML
#   3. security gate   — assert the Kubernetes Pod Security Standards
#                        "Restricted" fields are pinned on EVERY container of
#                        EVERY rendered workload — Deployment, Job and CronJob
#                        alike (runAsNonRoot, seccompProfile
#                        RuntimeDefault, drop ALL caps,
#                        allowPrivilegeEscalation:false), plus our
#                        readOnlyRootFilesystem hardening
#                        https://kubernetes.io/docs/concepts/security/pod-security-standards/
#   4. golden render   — compare against deploy/helm/golden/ (or --update it),
#                        under the helm version pinned in deploy/helm/.tool-versions
#                        (`helm template` output is NOT byte-stable across helm
#                        releases, so the goldens are only reproducible on the pin)
#   5. kubeconform     — schema-validate the manifests IF kubeconform is on PATH
#                        (optional; skipped with a note when absent/offline)
#   6. values schema   — assert values.schema.json REFUSES what it claims to
#                        (typos, wrong types, bad enums, out-of-range ports) and
#                        still ACCEPTS what must stay open (the server's own
#                        config vocabulary, a parent chart's `global`)
#   7. policy postures  — assert the two sanctioned NetworkPolicy ingress
#                        postures render with the source posture each of them
#                        claims (the absent-selector refusals themselves are
#                        step 8)
#   8. template refusals — every `fail` in templates/ is probed: the values that
#                        must be refused, the message asserted, and the
#                        ENUMERATION checked both ways, so a `fail` added with
#                        no probe fails this script
#   9. chart pins      — the `helm install … --version` pins in README.md and the
#                        book's Kubernetes page follow Chart.yaml's version
#                        (--update rewrites them)
#
# What it does NOT check is printed at the end of every run, and is not a
# footnote: every one of those properties has failed while this script was green
# (#2159 — all three overlays rendered, linted and matched their goldens while
# producing configurations the server refuses to start on). A green run here
# means the MANIFESTS are well-formed, hardened and unchanged. It says nothing
# about whether the deployment works.
#
# Usage:
#   deploy/helm/validate.sh            # validate (fails on lint/render/golden drift)
#   deploy/helm/validate.sh --update   # regenerate the golden renders, the chart
#                                     # README and the published chart-version
#                                     # pins, then validate
#
# No cluster and no network are required for steps 1–4.
# The CI gate is the `helm-golden` job in .github/workflows/ci.yml.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
CHART_DIR="${SCRIPT_DIR}/ferroehr"
CI_DIR="${SCRIPT_DIR}/ci"
GOLDEN_DIR="${SCRIPT_DIR}/golden"
RELEASE_NAME="ferroehr"
NAMESPACE="ferroehr"

UPDATE=0
[[ "${1:-}" == "--update" ]] && UPDATE=1

red()   { printf '\033[31m%s\033[0m\n' "$*"; }
green() { printf '\033[32m%s\033[0m\n' "$*"; }
bold()  { printf '\033[1m%s\033[0m\n' "$*"; }

command -v helm >/dev/null 2>&1 || { red "helm not found on PATH"; exit 1; }
bold "helm: $(helm version --short)"

# The golden renders are byte-exact output of ONE helm version (see
# .tool-versions). A skewed helm produces a whitespace-only diff that reads as
# chart drift, so the skew is reported as itself instead.
PIN_FILE="${SCRIPT_DIR}/.tool-versions"
PINNED_HELM="$(sed -nE 's/^helm[[:space:]]+v?([0-9][^[:space:]]*)[[:space:]]*$/\1/p' "$PIN_FILE" | head -1)"
[[ -n "$PINNED_HELM" ]] || { red "no helm version declared in ${PIN_FILE}"; exit 1; }
ACTUAL_HELM="$(helm version --short | sed -E 's/^v?([0-9][^+]*).*$/\1/')"
GOLDEN_SKEW=0
if [[ "$ACTUAL_HELM" != "$PINNED_HELM" ]]; then
  GOLDEN_SKEW=1
  red "helm version skew: pinned ${PINNED_HELM} (${PIN_FILE}), running ${ACTUAL_HELM}"
fi

# The value sets to validate: <label>:<values-file>
# The viewer overlay is here because its ABSENCE was a hole, not a saving:
# the viewer is the second pod-bearing workload, the restricted-profile gate is
# per-container, and until this line the gate never saw a render containing it.
# The chart's own docs claimed the gate checked both workloads; it could not.
declare -a CASES=(
  "default:${CI_DIR}/default-values.yaml"
  "all-features:${CI_DIR}/all-features-values.yaml"
  "basic-auth:${CI_DIR}/basic-auth-values.yaml"
  "viewer:${CI_DIR}/viewer-values.yaml"
  "terminology:${CI_DIR}/terminology-values.yaml"
  "audit-categories:${CI_DIR}/audit-categories-values.yaml"
)

# ── Rendered-manifest structure check (awk; there is no Python in this repo) ───
# WELL-FORMEDNESS is not checked here because it is already checked twice, by
# tools that must be present anyway — verified by feeding a deliberately broken
# template through both: `helm template` exits 1, and `helm lint` reports
# "unable to parse YAML: error converting YAML to JSON". Both run above.
#
# What those do NOT catch is a document that is valid YAML but not a Kubernetes
# object: a template rendering a bare map produces only a helm-lint WARNING and a
# successful `helm template` (verified the same way). So that is what this checks,
# and it is the coverage the previous PyYAML implementation uniquely added — kept
# rather than narrowed: every rendered document must carry a top-level
# `apiVersion` and `kind`, and at least one document must exist.
yaml_ok() {
  local file="$1"
  awk '
    function flush() {
      if (has_content) {
        docs++
        if (!has_api || !has_kind) {
          printf "  document %d (ending line %d) is missing %s%s\n", docs, NR,
                 (has_api ? "" : "apiVersion "), (has_kind ? "" : "kind") > "/dev/stderr"
          bad++
        }
      }
      has_content = 0; has_api = 0; has_kind = 0
    }
    /^---[[:space:]]*$/ { flush(); next }
    # A top-level key has no leading whitespace; comments and blanks are neither
    # content nor keys.
    /^[[:space:]]*$/ { next }
    /^[[:space:]]*#/ { next }
    { has_content = 1 }
    /^apiVersion:/ { has_api = 1 }
    /^kind:/       { has_kind = 1 }
    END {
      flush()
      if (docs == 0) { print "  no YAML documents rendered" > "/dev/stderr"; exit 1 }
      if (bad > 0)   { printf "  %d document(s) are not Kubernetes objects\n", bad > "/dev/stderr"; exit 1 }
      printf "  %d rendered document(s), each with apiVersion + kind\n", docs
    }
  ' "$file"
}


# ── Selector immutability: the one field that breaks `helm upgrade` ──────────
# A Deployment's spec.selector.matchLabels is IMMUTABLE
# (https://kubernetes.io/docs/concepts/workloads/controllers/deployment/), so a
# change here is not a diff to review — it is an upgrade that fails on every
# existing release with an error most operators read as a chart bug. The obvious
# way to separate the viewer's pods from the server's is to add `component` to
# the shared selectorLabels helper, which is exactly the change this forbids.
#
# Pinned as an explicit expectation rather than left to the golden diff: a
# golden churns for a dozen innocent reasons and this field must never move
# quietly among them.
# Run one gate program over a rendered manifest.
#
# `yq -o=json` turns the multi-document YAML into one JSON document per line
# and `jq -s` slurps them into an array, which is the shape every gate program
# expects. A gate prints one line per violation and nothing when clean, so the
# exit status is decided here rather than repeated in each program.
#
# yq and jq rather than a YAML parser in another language: this repository ships
# no Python (.claude/rules/rust-style.md §No Python).
gate_jq() {
  local file="$1" program="$2" out
  out="$(yq -o=json -I0 '.' "$file" | jq -s -r -f "$(dirname "$0")/gates/$program")" || return 1
  [[ -z "$out" ]] || { printf '%s\n' "$out" | sed 's/^/  /'; return 1; }
  return 0
}

assert_selector_stable() {
  local file="$1"
  gate_jq "$file" selector.jq || { red "  selector immutability gate FAILED for ${file}"; exit 1; }
  echo "  selector.matchLabels unchanged, and no selector matches two workloads"
}

# ── Restricted-profile gate: EVERY pod, control by control ───────────────────
# The Kubernetes Pod Security Standards "restricted" profile
# (https://kubernetes.io/docs/concepts/security/pod-security-standards/) is
# checked per CONTAINER, not by grepping the file. A substring search passes as
# soon as one container carries a field, so with two workloads in a render — the
# server and the optional viewer — a compliant server would vouch for a
# non-compliant viewer. That is compliance by luck, and this render is the only
# place it can be caught before a cluster refuses the pod.
assert_security() {
  local file="$1"
  gate_jq "$file" restricted.jq || { red "  restricted-profile gate FAILED for ${file}"; exit 1; }
  echo "  restricted profile satisfied by every pod in the render (per container)"
  echo "  pod isolation agrees across workloads, and every multi-replica workload spreads"
}

mkdir -p "$GOLDEN_DIR"
FAIL=0

# ── A refusal probe must carry a value no release can equal ──────────────────
# A version-shaped probe value is one a later bump can make the ACCEPTED value:
# `terminology.image.tag=0.1.4` stopped proving its refusal the day FerroTERM
# 0.1.4 became the chart's pin (#3476). So every probe value is a sentinel or
# malformed by construction, and this check refuses any `key=value` word whose
# value parses as a version, which keeps the class from coming back.
release_shaped_probe() {
  local -a words
  local word hit=0
  read -r -a words <<<"$1"
  for word in "${words[@]}"; do
    [[ "$word" == *=* ]] || continue
    if [[ "${word#*=}" =~ ^v?[0-9]+(\.[0-9]+)+([-+].*)?$ ]]; then
      red "  VERSION-SHAPED probe value: ${word} — a release can equal it, and then"
      red "  the probe renders instead of refusing. Use a value no release can carry."
      hit=1
    fi
  done
  return "$hit"
}

# ── Secret-leak gate: no credential may reach the ConfigMap ───────────────────
# Two halves, because there are two ways to get this wrong. (1) Every secret the
# chart CARRIES is set to a unique sentinel, which must appear in a Secret and in
# no ConfigMap — absence alone would also be satisfied by dropping the secret on
# the floor. (2) Every secret-bearing CONFIG key must be refused outright, so a
# future secret key cannot leak by default — the last probe is a key that does
# not exist in the server's config tree at all, which is what proves the guard
# classifies by name shape rather than from a list of today's keys.
secret_leak_gate() {
  bold "── secret-leak gate ─────────────────────────────────────"
  local leak_values="${CI_DIR}/secret-leak-values.yaml"
  local cm secrets misdelivered=0
  cm="$(helm template "$RELEASE_NAME" "$CHART_DIR" -n "$NAMESPACE" \
        -f "$leak_values" -s templates/configmap.yaml)"
  secrets="$(helm template "$RELEASE_NAME" "$CHART_DIR" -n "$NAMESPACE" \
        -f "$leak_values" -s templates/secret.yaml)"
  # No `mapfile`: it is bash 4+, and macOS ships bash 3.2 as /usr/bin/env bash.
  local -a sentinels=()
  while IFS= read -r sentinel; do
    sentinels+=("$sentinel")
  done < <(grep -oE 'SENTINEL_[A-Z0-9]+_[0-9a-f]{4}' "$leak_values" | sort -u)
  for sentinel in "${sentinels[@]}"; do
    if grep -qF -- "$sentinel" <<<"$cm"; then
      red "  LEAKED into the ConfigMap: ${sentinel}"
      misdelivered=1
    fi
    if ! grep -qF -- "$sentinel" <<<"$secrets"; then
      red "  NOT DELIVERED by any Secret: ${sentinel}"
      misdelivered=1
    fi
  done
  if [[ "$misdelivered" -eq 0 ]]; then
    echo "  all ${#sentinels[@]} carried secrets delivered by a Secret, none in the ConfigMap"
  else
    FAIL=1
  fi

  # ROUTED secrets: the chart carries each through a `secrets:` key, so a value
  # under `config:` is an operator mistake and must be refused BY NAME. The
  # value is the SENTINEL_PROBE literal, which no release can carry.
  local -a routed_paths=(
    "db.url"
    "events.url"
    "fhir.outbound.url"
    "audit.fhir_feed.url"
    "auth.oidc.hmac_secret"
    "signing.key_passphrase"
    "multimedia.secret_access_key"
    "terminology.external.oauth2_clients.tx.client_secret"
    "auth.basic.users[0].password_hash"
  )
  local refused=0
  for path in "${routed_paths[@]}"; do
    local out
    if out="$(helm template "$RELEASE_NAME" "$CHART_DIR" -n "$NAMESPACE" \
              -f "${CI_DIR}/default-values.yaml" \
              --set-string "config.${path}=SENTINEL_PROBE" 2>&1)"; then
      red "  NOT REFUSED: config.${path} rendered instead of failing"
      refused=1
    elif ! grep -qF -- "config.${path}" <<<"$out"; then
      red "  config.${path} was refused, but the message does not name it:"
      echo "$out" | head -3
      refused=1
    fi
  done
  if [[ "$refused" -eq 0 ]]; then
    echo "  all ${#routed_paths[@]} routed secret paths refused, each naming its secrets: key"
  else
    FAIL=1
  fi

  # UNROUTED secrets: no `secrets:` key carries them, so the whole rendered TOML
  # must move into the Secret and NO ConfigMap may exist. The second path is not
  # a real server key at all — it is the deny-by-default probe, and it is what
  # proves the guard classifies by name shape rather than from a fixed list.
  # Nothing the server models is unroutable any more, so the only input that
  # reaches this branch is a secret-shaped key the config tree does not define —
  # which is exactly the deny-by-default case the branch exists for.
  local -a unrouted_paths=(
    "server.future_api_key"
  )
  local moved=0
  for path in "${unrouted_paths[@]}"; do
    local render
    if ! render="$(helm template "$RELEASE_NAME" "$CHART_DIR" -n "$NAMESPACE" \
                   -f "${CI_DIR}/default-values.yaml" \
                   --set-string "config.${path}=SENTINEL_PROBE" 2>&1)"; then
      red "  config.${path} failed to render; it should move the config into the Secret:"
      echo "$render" | head -3
      moved=1
      continue
    fi
    if grep -qE '^kind: ConfigMap$' <<<"$render"; then
      red "  config.${path}: a ConfigMap was still rendered"
      moved=1
    fi
    if ! grep -qF "SENTINEL_PROBE" <<<"$render"; then
      red "  config.${path}: the value vanished from the render entirely"
      moved=1
    fi
  done
  if [[ "$moved" -eq 0 ]]; then
    echo "  all ${#unrouted_paths[@]} unrouted secret paths delivered by Secret, with no ConfigMap"
  else
    FAIL=1
  fi
}
secret_leak_gate

# ── NetworkPolicy: the shipped ingress posture is a CHECKED claim ─────────────
# A NetworkPolicy rule with no `from`/`to` admits or permits EVERYTHING
# (https://kubernetes.io/docs/concepts/services-networking/network-policies/),
# while `kubectl get networkpolicy` and every prose summary of it read as
# default-deny. The absent-selector states are refused at render, and those
# refusals are probed with every other one in the refusal registry below.
#
# This gate is the other half, which a refusal gate alone never covers: the
# sanctioned paths must still render, AND the rendered policy must have the
# source posture claimed for it — the open default has no `from`, the narrowed
# one has one. Asserting the open default explicitly is not redundant with the
# golden: this is where the SHIPPED posture is a checked claim rather than
# whatever the template happens to emit.
#
# BOTH pod-bearing workloads are covered. The viewer carries the same ingress
# shape, and it is the HUMAN-facing surface — the §3 lesson applies here too: a
# gate that only ever saw the server's policy would let the viewer's regress
# while reporting the property as checked.
network_policy_gate() {
  bold "── NetworkPolicy ingress postures ───────────────────────"
  local base="${CI_DIR}/default-values.yaml" viewer="${CI_DIR}/viewer-values.yaml"
  local out values

  # The sanctioned postures, read back off the rendered policy — per workload.
  # <label>|<values file>|<policy name>|<the values key prefix>|<the template>
  local -a workloads=(
    "server|${base}|ferroehr|networkPolicy|templates/networkpolicy.yaml"
    "viewer|${viewer}|ferroehr-viewer|viewer.networkPolicy|templates/viewer.yaml"
  )
  local label name prefix template policy posture=0
  for case in "${workloads[@]}"; do
    label="$(cut -d'|' -f1 <<<"$case")"
    values="$(cut -d'|' -f2 <<<"$case")"
    name="$(cut -d'|' -f3 <<<"$case")"
    prefix="$(cut -d'|' -f4 <<<"$case")"
    template="$(cut -d'|' -f5 <<<"$case")"

    # The shipped posture: open, and saying so.
    if ! out="$(helm template "$RELEASE_NAME" "$CHART_DIR" -n "$NAMESPACE" -f "$values" \
                -s "$template" 2>&1)"; then
      red "  ${label}: the shipped values no longer render at all:"
      printf '%s\n' "$out" | head -4
      posture=1
      continue
    fi
    policy="$(printf '%s' "$out" | yq -r "select(.kind == \"NetworkPolicy\" and .metadata.name == \"${name}\")")"
    if [[ -z "$policy" ]]; then
      red "  ${label}: no NetworkPolicy named ${name} in the render — this gate checked nothing"
      posture=1
      continue
    fi
    if [[ "$(printf '%s' "$policy" | yq -r '.spec.ingress[0] | has("from")')" != "false" ]]; then
      red "  ${label}: the shipped values render a \`from\` — update this gate and the book if that is the new posture"
      posture=1
    fi
    if [[ "$(printf '%s' "$policy" | yq -r '.metadata.annotations["kubernetes.io/description"]')" != *"from EVERY source"* ]]; then
      red "  ${label}: the open default no longer SAYS it admits every source in the object's description"
      posture=1
    fi

    # The hardened posture: refusal armed AND the peers reaching the policy.
    if ! out="$(helm template "$RELEASE_NAME" "$CHART_DIR" -n "$NAMESPACE" -f "$values" \
                --set "${prefix}.ingressAllowAll=false" \
                --set "${prefix}.ingressFrom[0].namespaceSelector.matchLabels.kubernetes\\.io/metadata\\.name=ingress-nginx" \
                -s "$template" 2>&1)"; then
      red "  ${label}: a narrowed ingressFrom must render (it is the hardened path):"
      printf '%s\n' "$out" | head -4
      posture=1
      continue
    fi
    policy="$(printf '%s' "$out" | yq -r "select(.kind == \"NetworkPolicy\" and .metadata.name == \"${name}\")")"
    if [[ "$(printf '%s' "$policy" | yq -r '.spec.ingress[0].from[0].namespaceSelector.matchLabels["kubernetes.io/metadata.name"]')" != "ingress-nginx" ]]; then
      red "  ${label}: a narrowed ingressFrom rendered without reaching the policy's \`from\`"
      posture=1
    fi
  done
  if [[ "$posture" -eq 0 ]]; then
    echo "  server and viewer: the shipped default renders no \`from\` and says so; a set ingressFrom reaches the policy"
  else
    FAIL=1
  fi

  # ── FerroTERM: the one policy that is narrowed by DEFAULT ──────────────────
  # The two above ship an ingress rule with no `from` and say so. FerroTERM's
  # must NEVER be in that state: its caller is known (the CDR's pods), and a
  # published FHIR terminology endpoint answers $expand and $lookup to whoever
  # reaches it, which is the extraction the SNOMED CT Affiliate Licence's
  # clauses 2.2.4 and 2.7 are about. So this is asserted from both sides — the
  # rendered policy carries a `from` naming the CDR, and there is no key that
  # could remove it.
  local terminology="${CI_DIR}/terminology-values.yaml" narrowed=0
  if ! out="$(helm template "$RELEASE_NAME" "$CHART_DIR" -n "$NAMESPACE" -f "$terminology" \
              -s templates/terminology.yaml 2>&1)"; then
    red "  terminology: the shipped values no longer render at all:"
    printf '%s\n' "$out" | head -4
    narrowed=1
  else
    policy="$(printf '%s' "$out" | yq -r 'select(.kind == "NetworkPolicy" and .metadata.name == "ferroehr-terminology")')"
    if [[ -z "$policy" ]]; then
      red "  terminology: no NetworkPolicy named ferroehr-terminology in the render — this gate checked nothing"
      narrowed=1
    else
      if [[ "$(printf '%s' "$policy" | yq -r '.spec.ingress[0] | has("from")')" != "true" ]]; then
        red "  terminology: the ingress rule carries NO \`from\`, so it admits EVERY source to a terminology endpoint"
        narrowed=1
      fi
      if [[ "$(printf '%s' "$policy" | yq -r '.spec.ingress[0].from[0].podSelector.matchLabels["app.kubernetes.io/name"]')" != "ferroehr" ]]; then
        red "  terminology: the admitted peer is not the CDR's pods"
        narrowed=1
      fi
      if [[ "$(printf '%s' "$policy" | yq -r '.spec.egress | length')" != "1" ]]; then
        red "  terminology: egress is no longer DNS alone — FerroTERM opens no outbound connection of its own"
        narrowed=1
      fi
    fi
  fi
  # The absent open-everything BOOLEAN, asserted rather than assumed: the schema
  # refuses one, so no values file can reach the state the two policies above
  # ship through a flag. (A PEER that opens the port is a different route to the
  # same place and is closed in the schema instead — an empty peer and an empty
  # selector inside one are refused; schema_gate probes both.)
  if out="$(helm template "$RELEASE_NAME" "$CHART_DIR" -n "$NAMESPACE" -f "$terminology" \
            --set terminology.networkPolicy.ingressAllowAll=true 2>&1)"; then
    red "  terminology: terminology.networkPolicy.ingressAllowAll was ACCEPTED — an admit-everything key is exactly what this policy must not have"
    narrowed=1
  fi
  # And an operator's extra peer still reaches the rule, beside the CDR.
  if out="$(helm template "$RELEASE_NAME" "$CHART_DIR" -n "$NAMESPACE" -f "$terminology" \
            --set "terminology.networkPolicy.extraIngressFrom[0].namespaceSelector.matchLabels.kubernetes\\.io/metadata\\.name=ferrockm" \
            -s templates/terminology.yaml 2>&1)"; then
    policy="$(printf '%s' "$out" | yq -r 'select(.kind == "NetworkPolicy" and .metadata.name == "ferroehr-terminology")')"
    if [[ "$(printf '%s' "$policy" | yq -r '.spec.ingress[0].from[1].namespaceSelector.matchLabels["kubernetes.io/metadata.name"]')" != "ferrockm" ]]; then
      red "  terminology: extraIngressFrom rendered without reaching the policy's \`from\`"
      narrowed=1
    fi
  else
    red "  terminology: an extraIngressFrom peer must render:"
    printf '%s\n' "$out" | head -4
    narrowed=1
  fi
  if [[ "$narrowed" -eq 0 ]]; then
    echo "  terminology: ingress narrowed to the CDR's pods by default, no boolean opens it, egress is DNS alone, an extraIngressFrom peer is admitted as written (and an empty one is refused by the schema)"
  else
    FAIL=1
  fi
}
network_policy_gate

# ── Every `fail` in a template is a refusal, and every refusal is PROBED ──────
# A `fail` decides something for an operator, and it ships as a string nothing
# executes: the render that would trigger it is precisely the render nobody
# runs. The egress refusal shipped a whole release guarded by nothing, which is
# not a lapse in diligence — it is the DEFAULT outcome for a branch with no
# probe, so the class is closed structurally rather than case by case.
#
# The registry below is the chart's complete refusal set — one record per `fail`
# call under templates/ — and it is checked in BOTH directions:
#
#   * every record's probe is rendered and must be REFUSED, with the message
#     naming the values an operator has to change. A refusal that fires without
#     naming its remedy is a crash with prose attached, and the message text is
#     the entire user interface of a render-time refusal.
#   * every `fail` found by grepping templates/ must have a record. This is the
#     half that stops the class from regrowing: a refusal added without a probe
#     fails this script on the commit that adds it, not a release later.
#
# The reverse direction is checked too — a record whose `fail` no longer exists
# fails as well, because a probe that can never fire is a gate reporting a
# property nothing has.
#
# Every probe value is one no release can equal (release_shaped_probe above):
# secrets are SENTINEL_PROBE, hosts are under the reserved example.com, and the
# FerroTERM tag is not a version at all, so no future pin can make it accepted.
#
# <template>|<substring unique to that fail's message>|<values file>|<--set probe>|<what the refusal must name, `;`-separated>
refusal_registry_gate() {
  bold "── template refusals: every \`fail\` is probed ────────────"
  local base="${CI_DIR}/default-values.yaml"
  local basic="${CI_DIR}/basic-auth-values.yaml"
  local viewer="${CI_DIR}/viewer-values.yaml"
  local -a registry=(
    "deployment.yaml|no authentication mechanism is configured|${base}|--set config.auth.oidc.issuer=null|config.auth.basic.users;config.auth.oidc.issuer"
    "migration-job.yaml|requires migrations.job.existingSecret|${base}|--set migrations.job.enabled=true|migrations.job.existingSecret"
    "hpa.yaml|with every target utilisation at 0|${base}|--set autoscaling.enabled=true --set autoscaling.targetCPUUtilizationPercentage=0 --set autoscaling.targetMemoryUtilizationPercentage=0|autoscaling.targetCPUUtilizationPercentage;autoscaling.targetMemoryUtilizationPercentage"
    "_helpers.tpl|multi-tenancy no longer exists|${base}|--set-string config.tenancy.enabled=true|config.tenancy;separate instances"
    "_helpers.tpl|refusing to render a secret into the ConfigMap|${base}|--set-string config.db.url=SENTINEL_PROBE|config.db.url;database.existingSecret"
    "_helpers.tpl|has no matching entry at config.auth.basic.users[]|${basic}|--set-string secrets.basicUserPasswordHashes.ghost=SENTINEL_PROBE|secrets.basicUserPasswordHashes.ghost"
    "_helpers.tpl|has no client declared at config.terminology.external.oauth2_clients|${base}|--set-string secrets.terminologyOauth2ClientSecrets.ghost=SENTINEL_PROBE|config.terminology.external.oauth2_clients.ghost"
    "_helpers.tpl|already declares config.terminology.external.providers.default|${base}|--set terminology.enabled=true --set config.terminology.external.providers.default.type=fhir --set config.terminology.external.providers.default.url=https://tx.example.com/fhir|config.terminology.external.providers.default;terminology.wireCdr"
    "_helpers.tpl|which inject enabled=true|${base}|--set terminology.enabled=true --set config.terminology.external.enabled=false|config.terminology.external.enabled;terminology.wireCdr"
    "_helpers.tpl|contradicts terminology.failOnError|${base}|--set terminology.enabled=true --set config.terminology.external.fail_on_error=true|config.terminology.external.fail_on_error;terminology.failOnError"
    "_helpers.tpl|while config.terminology.external.routes is empty|${base}|--set terminology.enabled=true --set config.terminology.external.providers.tx.type=fhir --set config.terminology.external.providers.tx.url=https://tx.example.com/fhir|config.terminology.external.routes;terminology.wireCdr"
    "_helpers.tpl|while terminology.image.digest is non-empty|${base}|--set terminology.enabled=true --set terminology.image.tag=never-a-release|terminology.image.tag;terminology.image.digest"
    "_helpers.tpl|the chart injects both|${base}|--set config.usage_report.enabled=false|config.usage_report.enabled;usageReport.enabled"
    "_helpers.tpl|the chart injects both|${base}|--set-string config.usage_report.deployment=compose|config.usage_report.deployment;usageReport.enabled"
    "networkpolicy.yaml|networkPolicy.ingressAllowAll=false with an empty|${base}|--set networkPolicy.ingressAllowAll=false|networkPolicy.ingressFrom;hardening-network-policy.md"
    "networkpolicy.yaml|with no destination for the database|${base}|--set networkPolicy.egress.enabled=true|networkPolicy.egress.database.to;hardening-network-policy.md"
    "viewer.yaml|viewer.networkPolicy.ingressAllowAll=false with an empty|${viewer}|--set viewer.networkPolicy.ingressAllowAll=false|viewer.networkPolicy.ingressFrom;hardening-network-policy.md"
    "backup-cronjob.yaml|persistentVolumeClaim is empty|${base}|--set backup.enabled=true --set backup.party.persistentVolumeClaim=party-dumps --set backup.linkage.persistentVolumeClaim=linkage-dumps|backup.clinical.persistentVolumeClaim"
    "backup-cronjob.yaml|persistentVolumeClaim is empty|${base}|--set backup.enabled=true --set backup.clinical.persistentVolumeClaim=clinical-dumps --set backup.linkage.persistentVolumeClaim=linkage-dumps|backup.party.persistentVolumeClaim"
    "backup-cronjob.yaml|persistentVolumeClaim is empty|${base}|--set backup.enabled=true --set backup.clinical.persistentVolumeClaim=clinical-dumps --set backup.party.persistentVolumeClaim=party-dumps|backup.linkage.persistentVolumeClaim"
    "backup-cronjob.yaml|name the same claim|${base}|--set backup.enabled=true --set backup.clinical.persistentVolumeClaim=one-claim --set backup.party.persistentVolumeClaim=one-claim --set backup.linkage.persistentVolumeClaim=linkage-dumps|backup.clinical.persistentVolumeClaim;backup.party.persistentVolumeClaim"
    "backup-cronjob.yaml|name the same claim|${base}|--set backup.enabled=true --set backup.clinical.persistentVolumeClaim=clinical-dumps --set backup.party.persistentVolumeClaim=one-claim --set backup.linkage.persistentVolumeClaim=one-claim|backup.party.persistentVolumeClaim;backup.linkage.persistentVolumeClaim"
    "backup-cronjob.yaml|existingSecret is empty|${base}|--set backup.enabled=true --set backup.clinical.persistentVolumeClaim=clinical-dumps --set backup.party.persistentVolumeClaim=party-dumps --set backup.linkage.persistentVolumeClaim=linkage-dumps|backup.clinical.existingSecret;silently partial"
    "backup-cronjob.yaml|existingSecret is empty|${base}|--set backup.enabled=true --set backup.clinical.persistentVolumeClaim=clinical-dumps --set backup.party.persistentVolumeClaim=party-dumps --set backup.linkage.persistentVolumeClaim=linkage-dumps --set backup.clinical.existingSecret=clinical-backup-dsn|backup.party.existingSecret;its own role"
    "backup-cronjob.yaml|existingSecret is empty|${base}|--set backup.enabled=true --set backup.clinical.persistentVolumeClaim=clinical-dumps --set backup.party.persistentVolumeClaim=party-dumps --set backup.linkage.persistentVolumeClaim=linkage-dumps --set backup.clinical.existingSecret=clinical-backup-dsn --set backup.party.existingSecret=party-backup-dsn|backup.linkage.existingSecret;narrowest of the three"
    # The audit domain joins the same three refusals, and only when
    # database.audit gives it a database of its own: without that set, the
    # three probes above already pass with no audit claim and no audit secret,
    # which is what proves the fourth job is conditional rather than always on.
    "backup-cronjob.yaml|persistentVolumeClaim is empty|${base}|--set database.audit.existingSecret=audit-dsn --set backup.enabled=true --set backup.clinical.persistentVolumeClaim=clinical-dumps --set backup.party.persistentVolumeClaim=party-dumps --set backup.linkage.persistentVolumeClaim=linkage-dumps|backup.audit.persistentVolumeClaim;nothing backs up the access log"
    "backup-cronjob.yaml|name the same claim|${base}|--set database.audit.existingSecret=audit-dsn --set backup.enabled=true --set backup.clinical.persistentVolumeClaim=clinical-dumps --set backup.party.persistentVolumeClaim=party-dumps --set backup.linkage.persistentVolumeClaim=one-claim --set backup.audit.persistentVolumeClaim=one-claim|backup.linkage.persistentVolumeClaim;backup.audit.persistentVolumeClaim"
    "backup-cronjob.yaml|existingSecret is empty|${base}|--set database.audit.existingSecret=audit-dsn --set backup.enabled=true --set backup.clinical.persistentVolumeClaim=clinical-dumps --set backup.party.persistentVolumeClaim=party-dumps --set backup.linkage.persistentVolumeClaim=linkage-dumps --set backup.audit.persistentVolumeClaim=audit-dumps --set backup.clinical.existingSecret=clinical-backup-dsn --set backup.party.existingSecret=party-backup-dsn --set backup.linkage.existingSecret=linkage-backup-dsn|backup.audit.existingSecret;not where this trail lives"
  )

  local record values probe wants want out refused=0
  for record in "${registry[@]}"; do
    values="$(cut -d'|' -f3 <<<"$record")"
    probe="$(cut -d'|' -f4 <<<"$record")"
    wants="$(cut -d'|' -f5 <<<"$record")"
    release_shaped_probe "$probe" || refused=1
    # shellcheck disable=SC2086  # the probe is a deliberate multi-word --set list
    if out="$(helm template "$RELEASE_NAME" "$CHART_DIR" -n "$NAMESPACE" -f "$values" $probe 2>&1)"; then
      red "  NOT REFUSED: ${probe} rendered instead of failing"
      refused=1
      continue
    fi
    while IFS= read -r want; do
      [[ -n "$want" ]] || continue
      grep -qF -- "$want" <<<"$out" && continue
      red "  ${probe} was refused, but the message does not name '${want}':"
      printf '%s\n' "$out" | head -4
      refused=1
    done < <(tr ';' '\n' <<<"$wants")
  done
  if [[ "$refused" -eq 0 ]]; then
    echo "  all ${#registry[@]} refusals fire, each naming the values that fix them"
  else
    FAIL=1
  fi

  # The enumeration. `fail` as a template FUNCTION is `fail "…"`, `fail (…)` or
  # `fail $…`; prose about failing ("fail to mount", "fail the render") is not
  # followed by one of those, which is what keeps the comment-heavy templates
  # out of this scan.
  local sites site file content anchor covered missing=0 orphaned=0 count=0
  sites="$(grep -rnE '(^|[^[:alnum:]_.])fail[[:space:]]*[("$]' "${CHART_DIR}/templates" || true)"
  while IFS= read -r site; do
    [[ -n "$site" ]] || continue
    count=$((count + 1))
    file="${site%%:*}"
    file="${file##*/}"
    content="${site#*:}"
    content="${content#*:}"
    covered=0
    for record in "${registry[@]}"; do
      [[ "$(cut -d'|' -f1 <<<"$record")" == "$file" ]] || continue
      anchor="$(cut -d'|' -f2 <<<"$record")"
      case "$content" in *"$anchor"*) covered=1 ;; *) ;; esac
    done
    if [[ "$covered" -eq 0 ]]; then
      red "  UNPROBED \`fail\`: ${site%%:*}:$(cut -d: -f2 <<<"$site")"
      red "  Add a record to refusal_registry_gate: the values that must be refused,"
      red "  and what the message has to name. A refusal nothing renders is a comment."
      missing=1
    fi
  done <<<"$sites"

  for record in "${registry[@]}"; do
    file="$(cut -d'|' -f1 <<<"$record")"
    anchor="$(cut -d'|' -f2 <<<"$record")"
    covered=0
    while IFS= read -r site; do
      [[ -n "$site" ]] || continue
      [[ "${site%%:*}" == *"/${file}" ]] || continue
      content="${site#*:}"
      content="${content#*:}"
      case "$content" in *"$anchor"*) covered=1 ;; *) ;; esac
    done <<<"$sites"
    if [[ "$covered" -eq 0 ]]; then
      red "  STALE probe: no \`fail\` in ${file} says '${anchor}' any more — drop or re-anchor the record"
      orphaned=1
    fi
  done

  if [[ "$missing" -eq 0 && "$orphaned" -eq 0 ]]; then
    echo "  ${count} \`fail\` call(s) in templates/, each with a probe; no probe without its \`fail\`"
  else
    FAIL=1
  fi
}
refusal_registry_gate

# ── The values schema must refuse and permit exactly what it claims to ────────
# A schema that is present but vacuous is worse than none: `helm template` stays
# green, Artifact Hub renders a Values-schema tab, and an operator reads both as
# evidence their values file was checked. So the file is exercised from both
# sides — every refusal it promises, and every place it must stay OPEN.
#
# It has to stay open in two directions. `config:` is a 1:1 passthrough of the
# SERVER's TOML tree, adjudicated at boot by the binary that owns that
# vocabulary; typing it here would fork the config tree into the chart and refuse
# keys the server accepts. And `global:` is what a parent chart injects when this
# chart is used as a dependency — refusing it would make the chart unusable as a
# subchart, with an error that names nothing the operator wrote.
schema_gate() {
  bold "── values schema ────────────────────────────────────────"
  local schema="${CHART_DIR}/values.schema.json"
  if [[ ! -f "$schema" ]]; then
    red "  missing: ${schema} — nothing validates an operator's values file, and"
    red "  Artifact Hub reports the chart as having no values schema."
    FAIL=1
    return
  fi
  # Not a preference: helm validates through a multi-draft library, but Artifact
  # Hub's renderer is typed against json-schema draft 4/6/7, so a newer dialect
  # renders wrong on the page that consumes the file.
  if ! grep -q 'json-schema.org/draft-07/schema' "$schema"; then
    red "  ${schema} does not declare the draft-07 dialect"
    FAIL=1
  fi

  # Each value is malformed by construction (a typo'd key, a wrong type, a
  # non-hex digest, an out-of-range port), never a version a release could ship.
  #
  # <helm --set argument>|<the refusal must name this>
  local -a refusals=(
    "autoscalng.enabled=true|additional properties 'autoscalng' not allowed"
    "probes.startup.periodSecond=5|/probes/startup"
    "replicaCount=two|/replicaCount"
    "image.pullPolicy=Sometimes|/image/pullPolicy"
    "image.digest=sha256:nothex|/image/digest"
    "service.port=70000|/service/port"
    "metrics.serviceMonitor.interval=30|/metrics/serviceMonitor/interval"
    "networkPolicy.ingressAllowAll=maybe|/networkPolicy/ingressAllowAll"
    "viewer.networkPolicy.ingressAllowAll=maybe|/viewer/networkPolicy/ingressAllowAll"
    "termnology.enabled=true|additional properties 'termnology' not allowed"
    "terminology.enabled=maybe|/terminology/enabled"
    "terminology.service.port=70000|/terminology/service/port"
    "terminology.image.pullPolicy=Sometimes|/terminology/image/pullPolicy"
    "terminology.image.digest=sha256:nothex|/terminology/image/digest"
    "terminology.logFormat=verbose|/terminology/logFormat"
    "terminology.index.mountPath=data/index|/terminology/index/mountPath"
    "backup.clinicl.schedule=@daily|additional properties 'clinicl' not allowed"
    "backup.enabled=maybe|/backup/enabled"
    "backup.clinical.schedule=17|/backup/clinical/schedule"
    "backup.party.schedule=daily|/backup/party/schedule"
    "backup.linkage.schedule=weekly|/backup/linkage/schedule"
    "backup.audit.schedule=nightly|/backup/audit/schedule"
    "backup.backoffLimit=-1|/backup/backoffLimit"
    "config.audit.store.retention_years=three|/config/audit/store/retention_years"
    "config.deployment_profile=true|/config/deployment_profile"
  )
  local refused=0 probe want out
  for case in "${refusals[@]}"; do
    probe="${case%%|*}"
    want="${case##*|}"
    release_shaped_probe "$probe" || refused=1
    if out="$(helm template "$RELEASE_NAME" "$CHART_DIR" -n "$NAMESPACE" \
              -f "${CI_DIR}/default-values.yaml" --set "$probe" 2>&1)"; then
      red "  NOT REFUSED: --set ${probe} rendered instead of failing schema validation"
      refused=1
    elif ! grep -qF -- "$want" <<<"$out"; then
      red "  --set ${probe} was refused, but the message does not mention '${want}':"
      printf '%s\n' "$out" | head -4
      refused=1
    fi
  done
  if [[ "$refused" -eq 0 ]]; then
    echo "  all ${#refusals[@]} malformed-values probes refused, each naming the offending path"
  else
    FAIL=1
  fi

  # The peer-shaped refusals, which `--set` cannot express: an empty map and an
  # empty selector are the values under test, and `--set x={}` sets a string.
  # They matter more than their spelling suggests — FerroTERM's policy has no
  # admit-everything key ON PURPOSE, and `namespaceSelector: {}` reaches that
  # state anyway ("if present but empty, it selects all namespaces",
  # https://kubernetes.io/docs/reference/kubernetes-api/policy-resources/network-policy-v1/,
  # NetworkPolicyPeer), so the schema is the only thing standing between an
  # operator adding a peer and a terminology endpoint open to the cluster.
  #
  # <helm --set-json argument>|<the refusal must name this>
  local -a json_refusals=(
    'terminology.networkPolicy.extraIngressFrom=[{"namespaceSelector":{}}]|/terminology/networkPolicy/extraIngressFrom/0/namespaceSelector'
    'terminology.networkPolicy.extraIngressFrom=[{"podSelector":{}}]|/terminology/networkPolicy/extraIngressFrom/0/podSelector'
    'terminology.networkPolicy.extraIngressFrom=[{"namespaceSelector":{"matchLabels":{}}}]|/terminology/networkPolicy/extraIngressFrom/0/namespaceSelector'
    'terminology.networkPolicy.extraIngressFrom=[{}]|/terminology/networkPolicy/extraIngressFrom/0'
  )
  local json_refused=0
  for case in "${json_refusals[@]}"; do
    probe="${case%%|*}"
    want="${case##*|}"
    if out="$(helm template "$RELEASE_NAME" "$CHART_DIR" -n "$NAMESPACE" \
              -f "${CI_DIR}/terminology-values.yaml" --set-json "$probe" 2>&1)"; then
      red "  NOT REFUSED: --set-json ${probe} rendered a peer that admits every source"
      json_refused=1
    elif ! grep -qF -- "$want" <<<"$out"; then
      red "  --set-json ${probe} was refused, but the message does not mention '${want}':"
      printf '%s\n' "$out" | head -4
      json_refused=1
    fi
  done
  # And the narrowing peer the book documents still renders, so the constraint
  # above is a narrowing of the wrong shapes rather than of the feature.
  if ! out="$(helm template "$RELEASE_NAME" "$CHART_DIR" -n "$NAMESPACE" \
              -f "${CI_DIR}/terminology-values.yaml" \
              --set-json 'terminology.networkPolicy.extraIngressFrom=[{"namespaceSelector":{"matchLabels":{"kubernetes.io/metadata.name":"ferrockm"}}}]' 2>&1)"; then
    red "  WRONGLY REFUSED: a peer naming a namespace must still render:"
    printf '%s\n' "$out" | head -4
    json_refused=1
  fi
  if [[ "$json_refused" -eq 0 ]]; then
    echo "  all ${#json_refusals[@]} admit-everything peer shapes refused, and a peer that names a source still renders"
  else
    FAIL=1
  fi

  local -a acceptances=(
    "config.server.future_api_key=probe"
    "config.telemetry.otlp_endpoint=http://otel-collector:4317"
    "global.imageRegistry=registry.example.com"
    "config.deployment_profile=production"
    "config.audit.store.retention_years=5"
  )
  local accepted=0
  for probe in "${acceptances[@]}"; do
    if ! out="$(helm template "$RELEASE_NAME" "$CHART_DIR" -n "$NAMESPACE" \
                -f "${CI_DIR}/default-values.yaml" --set "$probe" 2>&1)"; then
      red "  WRONGLY REFUSED: --set ${probe} must be accepted (see the note above this gate):"
      printf '%s\n' "$out" | head -4
      accepted=1
    fi
  done
  if [[ "$accepted" -eq 0 ]]; then
    echo "  all ${#acceptances[@]} must-stay-open probes accepted (server config vocabulary, parent-chart global)"
  else
    FAIL=1
  fi
}
schema_gate

# ── The live-test fixture must still track the chart ──────────────────────────
# WHY a `.claude/` path appears in a deploy gate: `.claude/skills/k8s-test/` holds
# the values overlay every cluster run installs with, so it is a consumer of this
# chart exactly like the overlays in ci/ — and the only thing that ever exercised
# it was a human running the skill, which happens when they are testing something
# ELSE. That is how it broke: the secret-routing change made `password_hash` under
# `config:` a refusal, the fixture still set it there, and nothing failed until the
# next live run would have. Same shape as the golden gate that ran in no workflow.
#
# This RENDERS, it does not deep-verify. The property is "the fixture still tracks
# the chart", not "the fixture is correct" — a live run checks the second.
fixture_gate() {
  bold "── live-test fixture ────────────────────────────────────"
  local repo_root fixture
  repo_root="$(cd "${SCRIPT_DIR}/../.." && pwd)"
  fixture="${repo_root}/.claude/skills/k8s-test/test-values.yaml"
  if [[ ! -f "$fixture" ]]; then
    red "  missing: .claude/skills/k8s-test/test-values.yaml — the k8s-test skill installs with it"
    FAIL=1
    return
  fi
  local rendered out
  rendered="$(mktemp)"
  if ! out="$(helm template "$RELEASE_NAME" "$CHART_DIR" -n "$NAMESPACE" -f "$fixture" 2>&1)"; then
    red "  the k8s-test fixture NO LONGER RENDERS against this chart."
    red "  This is the FIXTURE being stale, not your chart change being wrong:"
    red "  .claude/skills/k8s-test/test-values.yaml needs the same edit your values change implies."
    printf '%s
' "$out" | head -6
    FAIL=1
    rm -f "$rendered"
    return
  fi
  printf '%s' "$out" > "$rendered"
  # The objects a live run actually drives; a fixture that renders but produces no
  # Deployment would pass a bare render check and fail the run.
  local missing=0 kind
  for kind in Deployment Service ServiceAccount NetworkPolicy; do
    grep -qE "^kind: ${kind}$" "$rendered" || { red "  fixture renders no ${kind}"; missing=1; }
  done
  # The live run authenticates, so the fixture must still configure a usable
  # mechanism — and the hash must arrive by the mounted route, not in the ConfigMap.
  grep -q 'password_hash_file' "$rendered" || { red "  fixture configures no Basic user via the mounted route (password_hash_file absent)"; missing=1; }
  # yq document selection, not a fixed grep window: the ConfigMap grows with
  # the config tree, and a `grep -A<n>` ceiling silently stops covering the
  # tail the day the document passes n lines (#2391).
  if yq eval 'select(.kind == "ConfigMap")' "$rendered" | grep -q 'argon2id'; then
    red "  fixture puts an Argon2id hash in the ConfigMap"
    missing=1
  fi
  if [[ "$missing" -eq 0 ]]; then
    echo "  the k8s-test fixture still renders a Deployment/Service/SA/NetworkPolicy with a mounted hash"
  else
    FAIL=1
  fi
  rm -f "$rendered"
}
fixture_gate

# ── The declared posture and the audit horizon reach ferroehr.toml ────────────
# A golden records whatever the render produced; this asserts what it must
# produce. The default render declares its deployment profile explicitly, and
# the audit-categories overlay renders the calendar-year horizon and a category
# map whose keys need TOML quoting (spaces, dots). The values are read from the
# ConfigMap's ferroehr.toml, the file the server actually loads.
rendered_toml() {
  helm template "$RELEASE_NAME" "$CHART_DIR" -n "$NAMESPACE" -f "$1" \
    --show-only templates/configmap.yaml | yq '.data."ferroehr.toml"'
}
config_keys_gate() {
  bold "── declared posture + audit horizon ─────────────────────"
  local toml missing=0 want
  toml="$(rendered_toml "${CI_DIR}/default-values.yaml")"
  grep -qx 'deployment_profile = "sandbox"' <<<"$toml" \
    || { red "  the default render does not declare deployment_profile = \"sandbox\""; missing=1; }
  toml="$(rendered_toml "${CI_DIR}/audit-categories-values.yaml")"
  for want in \
    '^ *retention_years = 3(\.0)?$' \
    '^ *sgb_v_309_controller = true$' \
    '^ *\[audit\.categories\.templates\]$' \
    '^ *"International Patient Summary" = \["patient-summary"\]$' \
    '^ *\[audit\.categories\.archetypes\]$' \
    '^ *"openEHR-EHR-COMPOSITION\.encounter\.v1" = \["none"\]$'; do
    grep -qE "$want" <<<"$toml" || { red "  audit-categories render lacks a line matching: ${want}"; missing=1; }
  done
  if [[ "$missing" -eq 0 ]]; then
    echo "  the default render declares sandbox; the audit overlay renders retention_years = 3 and a quoted category map"
  else
    FAIL=1
  fi
}
config_keys_gate

for case in "${CASES[@]}"; do
  label="${case%%:*}"
  values="${case##*:}"
  bold "── ${label} ───────────────────────────────────────────────"

  echo "helm lint:"
  helm lint "$CHART_DIR" -f "$values"

  rendered="$(mktemp)"
  helm template "$RELEASE_NAME" "$CHART_DIR" -n "$NAMESPACE" -f "$values" > "$rendered"

  echo "YAML validity:"
  yaml_ok "$rendered"

  echo "security gate:"
  assert_security "$rendered"
  assert_selector_stable "$rendered"

  if command -v kubeconform >/dev/null 2>&1; then
    echo "kubeconform:"
    kubeconform -strict -summary -ignore-missing-schemas "$rendered" || FAIL=1
  else
    echo "kubeconform: not installed — skipping schema validation (optional)"
  fi

  golden="${GOLDEN_DIR}/${label}.yaml"
  if [[ "$GOLDEN_SKEW" -eq 1 ]]; then
    red "golden ${label}: SKIPPED — running helm ${ACTUAL_HELM}, goldens are pinned to ${PINNED_HELM}."
    red "  Install helm ${PINNED_HELM}, or bump ${PIN_FILE} AND re-run with --update in the same change."
    FAIL=1
  elif [[ "$UPDATE" -eq 1 ]]; then
    cp "$rendered" "$golden"
    green "golden updated: ${golden}"
  elif [[ -f "$golden" ]]; then
    if diff -u "$golden" "$rendered" > /tmp/golden.diff 2>&1; then
      green "golden matches: ${golden}"
    else
      red "golden DRIFT for ${label} — re-run with --update if intended:"
      head -40 /tmp/golden.diff
      FAIL=1
    fi
  else
    red "golden missing: ${golden} — run: deploy/helm/validate.sh --update"
    FAIL=1
  fi
  rm -f "$rendered"
done

# ── The Artifact Hub metadata set is DECIDED, so a change must be deliberate ──
# Every supported annotation was adjudicated (#2206): six are set here, three are
# injected per release by the publish lane, and the rest are declined in a
# comment block above `annotations:` with the reason. That record is only worth
# keeping if it cannot drift, so the set is pinned: adding an annotation without
# recording the decision, or losing one silently, fails here.
bold "Artifact Hub metadata is the decided set"
expected_annotations="artifacthub.io/category
artifacthub.io/images
artifacthub.io/license
artifacthub.io/links
artifacthub.io/maintainers
artifacthub.io/screenshots"
actual_annotations="$(yq -r '.annotations | keys | .[]' "$CHART_DIR/Chart.yaml" | sort)"
if [[ "$actual_annotations" != "$expected_annotations" ]]; then
  red "  Artifact Hub annotation set changed — decide it on the record (#2206), then update this list"
  diff <(printf '%s\n' "$expected_annotations") <(printf '%s\n' "$actual_annotations") || true
  exit 1
fi
# The listing logo is a Chart.yaml field, not an annotation, and its absence is
# invisible except on the published page.
[[ -n "$(yq -r '.icon // ""' "$CHART_DIR/Chart.yaml")" ]] || {
  red "  Chart.yaml has no icon — Artifact Hub renders a placeholder without one"
  exit 1
}
echo "  annotations are the decided set, and the listing icon is present"

# ── the chart README is GENERATED, so it is drift-checked, not reviewed ───────
# helm-docs renders README.md from README.md.gotmpl + the `# --` comments in
# values.yaml. A hand-edited README, or a new value documented nowhere, is drift:
# the values table is the chart's published reference (it is the front page on a
# registry and on Artifact Hub), and a table that disagrees with values.yaml is
# worse than no table. Skipped with a note when helm-docs is absent, so the rest
# of this script still runs locally. Under --update the regenerated README is
# kept, like the goldens; without it the drift is reported and undone.
echo
if command -v helm-docs >/dev/null 2>&1; then
  README="${CHART_DIR}/README.md"
  if [[ -f "$README" ]]; then
    before="$(mktemp)"; cp "$README" "$before"
    helm-docs --chart-search-root "$CHART_DIR" --template-files README.md.gotmpl \
      >/dev/null 2>&1
    if diff -u "$before" "$README" > /tmp/readme.diff 2>&1; then
      green "chart README matches values.yaml"
    elif [[ "$UPDATE" -eq 1 ]]; then
      green "chart README regenerated: ${README}"
    else
      red "chart README DRIFT — README.md is generated; regenerate it with:"
      red "  helm-docs --chart-search-root ${CHART_DIR} --template-files README.md.gotmpl"
      head -40 /tmp/readme.diff
      cp "$before" "$README"
      FAIL=1
    fi
    rm -f "$before"
  else
    red "chart README missing: ${README} — it is published metadata, and Artifact"
    red "Hub renders it as the listing body; generate it with helm-docs."
    FAIL=1
  fi
else
  # Locally a missing helm-docs is a note; in CI it is a FAILURE (#2806): the
  # self-skip is how the v4.0.5 README shipped a release stale — the lane ran
  # green with the one check that would have caught it silently absent.
  if [[ "${CI:-}" == "true" ]]; then
    red "helm-docs: not installed — CI must install the pinned helm-docs"
    red "(deploy/helm/.tool-versions); a silent skip is how a stale README ships."
    FAIL=1
  else
    printf '%s\n' "helm-docs: not installed — skipping the chart README drift check"
  fi
fi

# ── the published chart-version pins follow the chart version ────────────────
# `helm install … --version X.Y.Z` is pinned in README.md and in the book's
# Kubernetes page. A chart bump that leaves them behind teaches an operator to
# pull the previous chart. `scripts/checks/docs-claims.sh` caught that, but only
# as a separate CI job, so every chart bump of this cycle went red there after
# this script had said green (#3405). The pins are now part of what `--update`
# regenerates, and a stale one fails here.
echo
CHART_VERSION="$(awk '$1=="version:"{print $2; exit}' "${CHART_DIR}/Chart.yaml")"
[[ -n "$CHART_VERSION" ]] || { red "no version in ${CHART_DIR}/Chart.yaml"; exit 1; }
for page in "${REPO_ROOT}/README.md" "${REPO_ROOT}/website/book/src/installation/kubernetes.md"; do
  rel="${page#"${REPO_ROOT}/"}"
  [[ -f "$page" ]] || { red "chart-version pin page missing: ${rel}"; FAIL=1; continue; }
  stale="$(grep -ohE -- '--version +[0-9]+\.[0-9]+\.[0-9]+' "$page" | awk '{print $2}' | sort -u | grep -vx "$CHART_VERSION" || true)"
  if [[ -z "$stale" ]]; then
    green "${rel}: chart pins are ${CHART_VERSION}"
  elif [[ "$UPDATE" -eq 1 ]]; then
    tmp="$(mktemp)"
    sed -E "s/(--version )[0-9]+\.[0-9]+\.[0-9]+/\1${CHART_VERSION}/g" "$page" > "$tmp"
    mv "$tmp" "$page"
    green "${rel}: rewrote the chart pins $(echo "$stale" | tr '\n' ' ')to ${CHART_VERSION}"
  else
    red "${rel}: pins chart --version $(echo "$stale" | tr '\n' ' ')— Chart.yaml says ${CHART_VERSION}"
    red "  fix: deploy/helm/validate.sh --update"
    FAIL=1
  fi
done

# ── What a green run above does NOT mean ─────────────────────────────────────
# Printed unconditionally, on success and on failure. It is the direct answer to
# #2159: every committed overlay was rendering, linting and matching its golden
# while none of them produced a configuration the server accepts, and the reason
# nobody noticed is that a green run here reads like a working deployment.
bold "── not checked by this script ───────────────────────────"
cat <<'BOUNDARY'
  This script renders. It never runs the server, and never talks to a cluster.
  It therefore cannot tell you any of the following, and a green result above
  must not be read as evidence of them:

    * that the server ACCEPTS the rendered ferroehr.toml — the keys it carries,
      their types, and the semantic rules validated at boot (an auth mechanism,
      the RFC 8725 32-byte HMAC floor, a real Argon2id PHC string, the SMART
      requirements). Run: deploy/helm/ci/boot-check.sh   [CI: chart-boot]
    * that the image the chart selects UNDERSTANDS those keys. `appVersion` and
      the chart's config defaults move on different clocks, so a key added
      in-tree can be rejected by the released image.
      Run: FERROEHR_IMAGE=ghcr.io/ferrohealth/ferroehr:<tag> \
             deploy/helm/ci/boot-check.sh   [CI: the release pipeline's chart leg, on a tag]
    * that a pod STARTS, migrates the database, passes its probes and serves a
      request — config validation opens no socket. Run: the k8s-test skill
      (.claude/skills/k8s-test/) against a real cluster.
    * that the values are semantically sensible for a real deployment: that the
      issuer exists, the broker is reachable, the S3 bucket resolves, the
      NetworkPolicy egress rules actually match the peers in use.
    * anything about the CHART's behaviour under `helm upgrade`, or about
      resource sizing, scheduling or autoscaling behaviour under load.
BOUNDARY

echo
if [[ "$FAIL" -eq 0 ]]; then
  green "ALL RENDER CHECKS PASSED (see 'not checked' above)"
else
  red "VALIDATION FAILED"
  exit 1
fi
