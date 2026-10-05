#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The deployment-conformance harness, Kubernetes platform (#2178).
#
# The compose harness (scripts/deploy-probe.sh) measures whether the software
# works when deployed. This one measures something compose cannot see: whether
# the CLUSTER applies what the chart asked for. `securityContext` is a request,
# admission is a separate judge, and a mounted secret that is never read looks
# exactly like one that is — so each of those is observed at its own far end,
# from the container runtime, the API server and the EndpointSlice rather than
# from the manifest we wrote.
#
# The database is a docker compose PostgreSQL on the host that the chart is
# pointed at (owner ruling): the chart provisions no database and takes an
# external DSN, so a host database is the faithful shape, and an in-cluster pod
# with no volume turns any restart into an empty database.
#
# Usage:
#   scripts/deploy-probe-k8s.sh                # install, probe, tear down
#   scripts/deploy-probe-k8s.sh --keep-up      # leave the release for poking at
#
# Env:
#   PROBE_K8S_IMAGE   repo:tag of the server image under probe. Defaults to a
#                     `dev-local` build if the node already has one, else the
#                     chart's appVersion. The chart's `config:` defaults track
#                     main while appVersion names the last release, so a key
#                     added since that release makes the appVersion image refuse
#                     to boot — that is a real finding, and P-K8S-BOOT reports it
#                     against the chart rather than hiding it.
#   PROBE_K8S_VIEWER_IMAGE
#                     repo:tag of the viewer image. Defaults to the
#                     chart's own (appVersion), which is what an operator
#                     enabling viewer.enabled gets.
#   K8S_TERM_INDEX_CLAIM
#                     name of an EXISTING PersistentVolumeClaim holding a
#                     FerroTERM index built off-cluster from a release you hold
#                     a licence for. Unset, the three P-K8S-TERM-INDEX-* probes
#                     are declared not exercised by name rather than run against
#                     an empty directory, which is not an index.
#   PROBE_K8S_NS      namespace (default ferroehr-probe; created and deleted).
#   PROBE_OUT         where the machine-readable record lands.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

KEEP_UP=0
[[ "${1:-}" = "--keep-up" ]] && KEEP_UP=1

PROBE_TMP="$(mktemp -d)"
export PROBE_TMP
PROBE_OUT="${PROBE_OUT:-docs/conformance/deployment/kubernetes.json}"
# shellcheck disable=SC2034  # read by the sourced kubernetes.sh helpers
COMPOSE_PROJECT="ferroehr-probe-k8s"

# shellcheck source=scripts/deploy-probes/lib.sh
. scripts/deploy-probes/lib.sh
# shellcheck source=scripts/deploy-probes/kubernetes.sh
. scripts/deploy-probes/kubernetes.sh

cleanup() {
  if [[ "$KEEP_UP" -eq 0 ]]; then
    k8s_teardown
  else
    k8s_pf_stop
    k8s_term_pf_stop
    dim "── release left installed (--keep-up): kubectl -n $K8S_NS get pods"
  fi
  rm -rf "$PROBE_TMP"
}
trap cleanup EXIT

for tool in kubectl helm docker jq; do
  command -v "$tool" >/dev/null || { red "$tool not found on PATH"; exit 1; }
done
kubectl cluster-info >/dev/null 2>&1 || {
  red "no reachable cluster — this harness needs one (Docker Desktop Kubernetes is enough)"
  exit 1
}

# Which image to probe. A locally built one is a legitimate SUT for behaviour
# and hardening; it is NOT legitimate for a provenance claim, since it carries
# no attestation — nothing here makes one.
if [[ -n "${PROBE_K8S_IMAGE:-}" ]]; then
  PROBE_K8S_IMAGE_REPO="${PROBE_K8S_IMAGE%:*}"
  PROBE_K8S_IMAGE_TAG="${PROBE_K8S_IMAGE##*:}"
else
  PROBE_K8S_IMAGE_REPO="ghcr.io/ferrohealth/ferroehr"
  # Detection goes through the cluster's node name, not `docker ps` — Docker
  # Desktop hides its node from the container list while `docker exec` into it
  # works, so a ps-based check reports "no node" on the commonest dev cluster.
  node="$(k8s_node_container || true)"
  if [[ -n "$node" ]] && [[ "$(docker exec "$node" crictl images 2>/dev/null \
       | grep -cE 'ferrohealth/ferroehr +dev-local')" != "0" ]]; then
    PROBE_K8S_IMAGE_TAG="dev-local"
  else
    PROBE_K8S_IMAGE_TAG="$(grep '^appVersion:' deploy/helm/ferroehr/Chart.yaml | awk '{print $2}' | tr -d '"')"
  fi
fi

bold "── deployment probes: kubernetes ───────────────────────────"
echo "  context:   $(kubectl config current-context)"
echo "  node:      $(kubectl get nodes -o jsonpath='{.items[0].status.nodeInfo.kubeletVersion}' 2>/dev/null)"
echo "  image:     $PROBE_K8S_IMAGE_REPO:$PROBE_K8S_IMAGE_TAG"
echo "  namespace: $K8S_NS"
echo

bold "bringing up the host database the chart will be pointed at"
k8s_db_up || { red "FATAL: the compose PostgreSQL never became ready"; exit 1; }
K8S_DB_HOST="$(k8s_resolve_db_host)"
if [[ -z "$K8S_DB_HOST" ]]; then
  red "FATAL: no address reached the host database from inside a pod"
  echo "  tried host.docker.internal, gateway.docker.internal and the node's default gateway"
  exit 1
fi
echo "  pods reach the database at $K8S_DB_HOST:$K8S_DB_PORT"
k8s_namespace

if probes_k8s_boot; then
  probes_k8s_runtime_posture
  probes_k8s_psa
  probes_k8s_service_links
  probes_k8s_secrets
  probes_k8s_readiness
  probes_k8s_viewer
  probes_k8s_terminology
else
  red "the release never served — the probes that need a running CDR were not run"
  uncovered "every probe after P-K8S-SERVE" \
    "the release did not come up, so nothing downstream could be observed"
fi

# ── The honest half ───────────────────────────────────────────────────────────
uncovered "NetworkPolicy enforcement, for the server policy and for the viewer policy this run installs narrowed" \
  "whether a policy is enforced is a property of the CNI, so this run proves the narrowed viewer recipe installs and still serves the peer it names, never that a peer outside ingressFrom is refused"
uncovered "load balancing across replicas" \
  "kube-proxy distributes connections, which needs conntrack on the node — readable only on a local-container node"
uncovered "horizontal autoscaling" \
  "the HPA needs a metrics API that a stock local cluster does not ship"
uncovered "Ingress and TLS termination, for the API and for the viewer" \
  "no controller is installed by this harness; both Ingress objects the chart renders are never reached"

probe_report "$PROBE_OUT" "kubernetes"
