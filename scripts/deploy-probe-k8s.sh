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
#   scripts/deploy-probe-k8s.sh --production-only  # only the P-K8S-PROD-* family
#   scripts/deploy-probe-k8s.sh --render-check # no cluster: render the production
#                                              # install this harness performs and
#                                              # assert it wires the four domains
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
#   PROBE_K8S_VALUES  an extra values file layered over every install, for a
#                     cluster that cannot run a chart default (a kind node in a
#                     podman VM cannot create the user-namespaced pods
#                     `hostUsers: false` asks for). The header prints it, and the
#                     posture probes then measure the overridden release, so a
#                     run with it is never a record of the chart's defaults.
#   PROBE_K8S_PROD_PORT_BASE
#                     first host port of the four PostgreSQL clusters the
#                     production probes bring up, one per storage domain
#                     (default 15441; the next three follow it).
#   PROBE_K8S_DB_HOST the address pods reach the host databases at, when none of
#                     the candidates the harness tries answers (a kind node on
#                     podman reaches the podman machine through the gateway of
#                     its `kind` network: `podman network inspect kind`).
#   PROBE_OUT         where the machine-readable record lands.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

KEEP_UP=0
RENDER_CHECK=0
PRODUCTION_ONLY=0
for arg in "$@"; do
  case "$arg" in
    --keep-up) KEEP_UP=1 ;;
    --render-check) RENDER_CHECK=1 ;;
    --production-only) PRODUCTION_ONLY=1 ;;
    *) echo "usage: $0 [--keep-up] [--production-only] | --render-check" >&2; exit 2 ;;
  esac
done

PROBE_TMP="$(mktemp -d)"
export PROBE_TMP
PROBE_OUT="${PROBE_OUT:-docs/conformance/deployment/kubernetes.json}"
# shellcheck disable=SC2034  # read by the sourced kubernetes.sh helpers
COMPOSE_PROJECT="ferroehr-probe-k8s"

# shellcheck source=scripts/deploy-probes/lib.sh
. scripts/deploy-probes/lib.sh
# shellcheck source=scripts/deploy-probes/kubernetes.sh
. scripts/deploy-probes/kubernetes.sh

# ── The production overlay against four database clusters ────────────────────
# deploy/helm/ci/boot-check.sh has the server judge values-production.yaml from
# its configuration alone, which cannot see the two facts the server measures
# over live connections: `shared_cluster` (each domain pool's
# pg_control_system().system_identifier) and the database a relocated domain is
# prepared and verified in. These probes install the overlay against four
# compose PostgreSQL clusters, one per storage domain, read the posture back
# with `ferroehr report` inside the running pod, then point all four domains at one
# cluster and watch the boot refuse. No openEHR spec governs deployment
# posture; this is our own design.
K8S_PROD_NS="${K8S_NS}-production"
K8S_PROD_RELEASE="ferroehr"
K8S_PROD_PORT_BASE="${PROBE_K8S_PROD_PORT_BASE:-15441}"
K8S_PROD_DOMAINS=(clinical party linkage audit)
K8S_PROD_VALUES="deploy/helm/ferroehr/values-production.yaml"
K8S_PROD_OVERRIDES="$PROBE_TMP/production-probe-values.yaml"
K8S_PROD_PASSWORD="probe-only-not-a-secret"

kp() { kubectl -n "$K8S_PROD_NS" "$@"; }

# The compose project, host port, login role and the group role it joins, per
# domain. The audit set grants to ferroehr_clinical
# (app/ferroehr/migrations/audit/0006_grants.sql), so the audit pool's login
# role joins that group.
k8s_prod_project() { printf '%s-prod-%s' "$COMPOSE_PROJECT" "$1"; }
k8s_prod_port() {
  local i
  for i in "${!K8S_PROD_DOMAINS[@]}"; do
    [[ "${K8S_PROD_DOMAINS[$i]}" == "$1" ]] && { printf '%s' "$((K8S_PROD_PORT_BASE + i))"; return 0; }
  done
  return 1
}
k8s_prod_login() { printf 'probe_%s' "$1"; }
k8s_prod_group() {
  case "$1" in
    audit) printf 'ferroehr_clinical' ;;
    *) printf 'ferroehr_%s' "$1" ;;
  esac
}

# k8s_prod_psql <cluster> <sql>: run SQL as the bootstrap superuser.
k8s_prod_psql() {
  docker compose -p "$(k8s_prod_project "$1")" exec -T ferroehr-postgres \
    psql -v ON_ERROR_STOP=1 -U postgres -d "${PG_INIT_DB:-ferroehr}" -tAc "$2"
}

# k8s_prod_dsn <login> <password> <cluster>
k8s_prod_dsn() {
  printf 'postgres://%s:%s@%s:%s/%s' "$1" "$2" "$K8S_DB_HOST" "$(k8s_prod_port "$3")" "${PG_INIT_DB:-ferroehr}"
}

# The owner role the image's init script creates holds ferroehr_migrator and
# owns the database, so it is the credential that prepares a cluster.
k8s_prod_owner_dsn() {
  k8s_prod_dsn "${PG_INIT_USER:-ferroehr}" "${PG_INIT_PASSWORD:-ferroehr}" "$1"
}

# Four clusters, each initialised on its own, so four system identifiers.
k8s_prod_db_up() {
  local domain
  for domain in "${K8S_PROD_DOMAINS[@]}"; do
    FERROEHR_BIND_HOST=0.0.0.0 FERROEHR_DB_PORT="$(k8s_prod_port "$domain")" \
      docker compose -p "$(k8s_prod_project "$domain")" \
      -f docker-compose.yml -f docker-compose.db-publish.yml \
      up -d ferroehr-postgres >/dev/null 2>&1 || return 1
  done
  for domain in "${K8S_PROD_DOMAINS[@]}"; do
    k8s_pg_ready "$(k8s_prod_project "$domain")" || return 1
  done
}

k8s_prod_db_down() {
  local domain
  for domain in "${K8S_PROD_DOMAINS[@]}"; do
    docker compose -p "$(k8s_prod_project "$domain")" down -v --remove-orphans >/dev/null 2>&1 || true
  done
}

# One login role per domain, a member of its domain's group role and of nothing
# else. Every login also exists on the clinical cluster, for the shared layout.
k8s_prod_roles() {
  local domain cluster login
  for domain in "${K8S_PROD_DOMAINS[@]}"; do
    for cluster in $(printf '%s\n' "$domain" clinical | sort -u); do
      login="$(k8s_prod_login "$domain")"
      k8s_prod_psql "$cluster" "DO \$\$ BEGIN
          IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = '${login}') THEN
            CREATE ROLE ${login} LOGIN PASSWORD '${K8S_PROD_PASSWORD}' IN ROLE $(k8s_prod_group "$domain");
          END IF;
        END \$\$;" >/dev/null || return 1
    done
  done
}

# k8s_prod_secret <name> <dsn>: one DSN under the key `dsn`, replaced in place.
k8s_prod_secret() {
  kp create secret generic "$1" --from-literal=dsn="$2" --dry-run=client -o yaml \
    | kp apply -f - >/dev/null
}

# k8s_prod_secrets <separate|shared>: the eight credentials the overlay names,
# a runtime login per domain, the clinical cluster's migrator, and the migrator
# of each other domain's cluster. `separate` puts each domain on its own
# cluster; `shared` puts all four on the clinical cluster, each still on its
# own login role.
k8s_prod_secrets() {
  local domain cluster
  for domain in "${K8S_PROD_DOMAINS[@]}"; do
    cluster="$domain"
    [[ "$1" == shared ]] && cluster=clinical
    k8s_prod_secret "probe-db-${domain}" \
      "$(k8s_prod_dsn "$(k8s_prod_login "$domain")" "$K8S_PROD_PASSWORD" "$cluster")" || return 1
    [[ "$domain" == clinical ]] && continue
    k8s_prod_secret "probe-db-${domain}-migrate" "$(k8s_prod_owner_dsn "$cluster")" || return 1
  done
  k8s_prod_secret probe-db-migrate "$(k8s_prod_owner_dsn clinical)"
}

# The values the probe layers over values-production.yaml: the overlay's
# example Secret names replaced by the ones created above, the image under
# probe, and no usage report from a probe instance. Every separation the
# overlay makes stays its own.
k8s_prod_overrides() {
  cat > "$K8S_PROD_OVERRIDES" <<YAML
usageReport:
  enabled: false
image:
  repository: ${PROBE_K8S_IMAGE_REPO}
  tag: "${PROBE_K8S_IMAGE_TAG}"
  pullPolicy: IfNotPresent
database:
  existingSecret: probe-db-clinical
  existingSecretKey: dsn
  party: {existingSecret: probe-db-party, existingSecretKey: dsn,
    migrateExistingSecret: probe-db-party-migrate, migrateExistingSecretKey: dsn}
  linkage: {existingSecret: probe-db-linkage, existingSecretKey: dsn,
    migrateExistingSecret: probe-db-linkage-migrate, migrateExistingSecretKey: dsn}
  audit: {existingSecret: probe-db-audit, existingSecretKey: dsn,
    migrateExistingSecret: probe-db-audit-migrate, migrateExistingSecretKey: dsn}
  migrateExistingSecret: probe-db-migrate
  migrateExistingSecretKey: dsn
migrations:
  job:
    existingSecret: probe-db-migrate
    existingSecretKey: dsn
YAML
}

k8s_prod_helm_args() {
  printf '%s\n' "$K8S_PROD_RELEASE" deploy/helm/ferroehr -n "$K8S_PROD_NS" \
    -f "$K8S_PROD_VALUES" -f "$K8S_PROD_OVERRIDES"
  [[ -n "${PROBE_K8S_VALUES:-}" ]] && printf '%s\n' -f "$PROBE_K8S_VALUES"
  return 0
}

# The newest server pod's log, the previous container's when it restarted: a
# refusing boot exits, and the log that names the refusal is the previous one.
k8s_prod_newest_log() {
  local pod
  pod="$(kp get pod -l "app.kubernetes.io/name=ferroehr,app.kubernetes.io/instance=${K8S_PROD_RELEASE}" \
    --sort-by=.metadata.creationTimestamp -o name 2>/dev/null | tail -1)"
  [[ -n "$pod" ]] || return 0
  { kp logs "$pod" --previous 2>/dev/null; kp logs "$pod" 2>/dev/null; } | tail -60
}

k8s_prod_teardown() {
  helm uninstall "$K8S_PROD_RELEASE" -n "$K8S_PROD_NS" >/dev/null 2>&1 || true
  kubectl delete namespace "$K8S_PROD_NS" --wait=false >/dev/null 2>&1 || true
  k8s_prod_db_down
}

probes_k8s_production() {
  bold "the production overlay against four database clusters"
  uncovered "the database hop's TLS under the production overlay" \
    "the four compose clusters serve plain TCP, so the DSNs carry no sslmode=verify-full; the overlay's advice to set it is not exercised"
  if ! k8s_prod_db_up; then
    uncovered "P-K8S-PROD-BOOT and P-K8S-PROD-SHARED" \
      "the four compose PostgreSQL clusters never became ready, a harness problem rather than a finding"
    return 0
  fi
  local domain ids
  ids="$(for domain in "${K8S_PROD_DOMAINS[@]}"; do
    k8s_prod_psql "$domain" "SELECT system_identifier FROM pg_control_system()"
  done | sort -u | grep -c .)"
  if [[ "$ids" -ne 4 ]] || ! k8s_prod_roles; then
    uncovered "P-K8S-PROD-BOOT and P-K8S-PROD-SHARED" \
      "the harness could not stand up four clusters with four system identifiers and the domain login roles (identifiers seen: ${ids})"
    return 0
  fi
  kubectl create namespace "$K8S_PROD_NS" >/dev/null 2>&1 || true
  k8s_prod_overrides
  k8s_prod_secrets separate

  probe "P-K8S-PROD-BOOT" "working" "server" "#3692" \
    "the chart's migration Job prepares four clusters on their own migrators, and values-production.yaml then boots under production with migrate = verify, its only gap plaintext_listener, accepted by name"
  local -a helm_args=()
  local line
  while IFS= read -r line; do helm_args+=("$line"); done < <(k8s_prod_helm_args)
  local report gaps accepted installed=0
  if helm status "$K8S_PROD_RELEASE" -n "$K8S_PROD_NS" >/dev/null 2>&1; then
    helm uninstall "$K8S_PROD_RELEASE" -n "$K8S_PROD_NS" --wait >/dev/null 2>&1
  fi
  if ! helm upgrade --install "${helm_args[@]}" --timeout 6m >"$PROBE_TMP/prod-helm.log" 2>&1; then
    probe_fail "the release installed, its migration Job having prepared all four clusters" \
      "$(tail -4 "$PROBE_TMP/prod-helm.log")" \
      "a failed pre-install Job lands here: kubectl -n $K8S_PROD_NS logs job/${K8S_PROD_RELEASE}-migrate"
    kp logs "job/${K8S_PROD_RELEASE}-migrate" --tail=8 2>&1 | sed 's/^/      job: /'
  else
    installed=1
  fi
  if [[ "$installed" -eq 1 ]]; then
    if ! kp rollout status "deploy/$K8S_PROD_RELEASE" --timeout=240s >/dev/null 2>&1; then
      probe_fail "a rolled-out Deployment under production" "$(k8s_prod_newest_log | tail -6)" \
        "the server names every open gap and every preparation refusal on its way out"
    else
      report="$(kp exec "deploy/$K8S_PROD_RELEASE" -c ferroehr -- /usr/local/bin/ferroehr report --output - 2>/dev/null)"
      gaps="$(jq -r '.deployment.gaps | join(" ")' <<<"$report" 2>/dev/null)"
      accepted="$(jq -r '.deployment.accepted | join(" ")' <<<"$report" 2>/dev/null)"
      assert_eq "production" "$(jq -r '.deployment.profile' <<<"$report" 2>/dev/null)" \
        "ferroehr report inside the running pod, over its live pools"
      assert_eq "plaintext_listener" "$gaps" \
        "the measured gaps over four clusters: shared_cluster must not be among them"
      assert_eq "plaintext_listener" "$accepted" \
        "the overlay accepts exactly this one gap by name"
    fi
  fi
  probe_done

  probe "P-K8S-PROD-SHARED" "broken" "server" "#3692" \
    "with all four domains on one cluster, each on its own role, the production boot refuses and names shared_cluster"
  if [[ "$installed" -eq 0 ]]; then
    probe_fail "an installed release to re-point" "the install failed above"
    probe_done
    return 0
  fi
  # Re-pointed at the clinical cluster, the upgrade's Job prepares every set
  # there, so the boot reaches the posture check instead of stopping at verify.
  k8s_prod_secrets shared
  if ! helm upgrade "${helm_args[@]}" --timeout 6m >"$PROBE_TMP/prod-helm-shared.log" 2>&1; then
    probe_fail "the upgrade's migration Job prepared the shared cluster" \
      "$(tail -4 "$PROBE_TMP/prod-helm-shared.log")"
    probe_done
    return 0
  fi
  kp rollout restart "deploy/$K8S_PROD_RELEASE" >/dev/null 2>&1
  local log="" _i
  for _i in $(seq 1 45); do
    log="$(k8s_prod_newest_log)"
    case "$log" in *shared_cluster*) break ;; *) ;; esac
    sleep 4
  done
  assert_contains "$log" "shared_cluster" \
    "two pools on one cluster must refuse the production boot by name"
  assert_not_contains "$log" "shared_credential" \
    "the domains still connect on four login roles; only the cluster is shared"
  if kp rollout status "deploy/$K8S_PROD_RELEASE" --timeout=30s >/dev/null 2>&1; then
    probe_fail "the restarted Deployment never becomes ready" "the rollout completed"
  fi
  probe_done
}

# --render-check: the production install above, rendered rather than installed.
# It proves the probe's overrides and the overlay agree on every credential the
# render consumes, so a renamed Secret or key fails here and not on a cluster.
render_check() {
  command -v helm >/dev/null || { red "helm not found on PATH"; exit 1; }
  PROBE_K8S_IMAGE_REPO="ghcr.io/ferrohealth/ferroehr"
  PROBE_K8S_IMAGE_TAG="render-check"
  k8s_prod_overrides
  local -a helm_args=()
  local line
  while IFS= read -r line; do helm_args+=("$line"); done < <(k8s_prod_helm_args)
  local rendered failed=0 want
  if ! rendered="$(helm template "${helm_args[@]}" 2>&1)"; then
    red "the production install does not render:"
    printf '%s\n' "$rendered" | head -8
    exit 1
  fi
  for want in \
    'deployment_profile = "production"' \
    'deployment_accepts = ["plaintext_listener"]' \
    'migrate = "verify"' \
    "name: probe-db-migrate" \
    "name: probe-db-clinical" \
    "name: probe-db-party" \
    "name: probe-db-linkage" \
    "name: probe-db-audit" \
    "name: probe-db-party-migrate" \
    "name: probe-db-linkage-migrate" \
    "name: probe-db-audit-migrate" \
    'value: "/etc/ferroehr-secrets/storage.party.url"' \
    'value: "/etc/ferroehr-secrets/storage.linkage.url"' \
    'value: "/etc/ferroehr-secrets/storage.audit.url"' \
    'value: "/etc/ferroehr-secrets/db.migrate_url"' \
    'value: "/etc/ferroehr-secrets/storage.migrate.party.url"' \
    'value: "/etc/ferroehr-secrets/storage.migrate.linkage.url"' \
    'value: "/etc/ferroehr-secrets/storage.migrate.audit.url"' \
    "name: ${K8S_PROD_RELEASE}-migrate"; do
    grep -qF -- "$want" <<<"$rendered" || { red "  the production install render lacks: ${want}"; failed=1; }
  done
  [[ "$(grep -c -- '- key: dsn' <<<"$rendered")" -ge 12 ]] \
    || { red "  fewer than twelve projections read the probe's 'dsn' key (the Deployment: four domains, three domain migrators, the migrator; the Job: the migrator and three domain migrators)"; failed=1; }
  [[ "$failed" -eq 0 ]] || exit 1
  green "render-check: the production install wires four domain DSNs, four migrators and the Job to the probe's Secrets"
}

if [[ "$RENDER_CHECK" -eq 1 ]]; then
  render_check
  rm -rf "$PROBE_TMP"
  exit 0
fi

cleanup() {
  if [[ "$KEEP_UP" -eq 0 ]]; then
    k8s_prod_teardown
    k8s_teardown
  else
    k8s_pf_stop
    k8s_term_pf_stop
    dim "── release left installed (--keep-up): kubectl -n $K8S_NS get pods"
  fi
  rm -rf "$PROBE_TMP"
}
trap cleanup EXIT

# A podman host has no `docker` CLI: kind runs its node as a podman container
# and compose runs through docker-compose against the podman machine's socket.
# The harness speaks `docker compose` and `docker exec`, so both are routed here
# and nothing changes where a real docker CLI exists.
if ! command -v docker >/dev/null 2>&1 && command -v podman >/dev/null 2>&1; then
  docker() {
    if [[ "$1" == compose ]]; then
      shift
      docker-compose "$@"
    else
      podman "$@"
    fi
  }
  if [[ -z "${DOCKER_HOST:-}" ]]; then
    DOCKER_HOST="unix://$(podman machine inspect --format '{{.ConnectionInfo.PodmanSocket.Path}}' 2>/dev/null)"
    export DOCKER_HOST
  fi
  command -v docker-compose >/dev/null || { red "docker-compose not found on PATH (podman host)"; exit 1; }
fi

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
if [[ -n "${PROBE_K8S_VALUES:-}" ]]; then
  echo "  overrides: $PROBE_K8S_VALUES (not the chart's defaults)"
fi
echo

if [[ "$PRODUCTION_ONLY" -eq 1 ]]; then
  bold "bringing up the four host databases the production probes point at"
  k8s_prod_db_up || { red "FATAL: the four compose PostgreSQL clusters never became ready"; exit 1; }
  K8S_DB_PORT="$(k8s_prod_port clinical)"
else
  bold "bringing up the host database the chart will be pointed at"
  k8s_db_up || { red "FATAL: the compose PostgreSQL never became ready"; exit 1; }
fi
K8S_DB_HOST="${PROBE_K8S_DB_HOST:-$(k8s_resolve_db_host)}"
if [[ -z "$K8S_DB_HOST" ]]; then
  red "FATAL: no address reached the host database from inside a pod"
  echo "  tried host.docker.internal, gateway.docker.internal and the node's default gateway"
  exit 1
fi
echo "  pods reach the database at $K8S_DB_HOST:$K8S_DB_PORT"

if [[ "$PRODUCTION_ONLY" -eq 1 ]]; then
  probes_k8s_production
  probe_report "$PROBE_OUT" "kubernetes"
  exit $?
fi

k8s_namespace
if probes_k8s_boot; then
  probes_k8s_runtime_posture
  probes_k8s_psa
  probes_k8s_service_links
  probes_k8s_secrets
  probes_k8s_readiness
  probes_k8s_viewer
  probes_k8s_terminology
  probes_k8s_bridge
  # Last: it uninstalls the release and drops the database, which is the
  # procedure under test.
  probes_k8s_original_state
else
  red "the release never served — the probes that need a running CDR were not run"
  uncovered "every probe after P-K8S-SERVE" \
    "the release did not come up, so nothing downstream could be observed"
fi
# Its own namespace and its own four clusters, so it runs whatever the release
# above did.
probes_k8s_production

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
