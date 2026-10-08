#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The Kubernetes probe family (#2178's second platform).
#
# The compose family answers "does the software work when it is deployed". This
# one answers a question compose cannot reach at all: **does the cluster apply
# what the chart asked for?** Those are different claims, and only the second is
# what an operator running the chart actually gets. `helm template` proves the
# YAML we generate; the API server, the kubelet and the container runtime each
# get a say afterwards, and every one of them can decline.
#
# So every probe here observes at the far end of THAT chain:
#
#   - the security posture is read out of the container runtime's own spec, not
#     out of the pod manifest — an empty capability bounding set is a fact about
#     the kernel, whereas `drop: [ALL]` is a request;
#   - Pod Security admission is judged by the API server refusing or admitting,
#     not by us re-reading the profile;
#   - a mounted secret is proven READ by breaking its path and watching the boot
#     refuse, because a secret that is mounted and never read looks identical in
#     `kubectl describe`;
#   - readiness is proven to GATE by removing the database and watching the
#     EndpointSlice drop the pod, not by a probe that has only ever returned UP.
#
# The database runs in docker compose on the host and the chart is pointed at it
# (owner ruling): the chart provisions no database and takes an external DSN, so
# a host database is the faithful shape rather than a convenience — and an
# in-cluster `kubectl create deployment pg` has no volume, which turns any
# restart into an empty database and a diagnosis cycle.
#
# Sourced by scripts/deploy-probe-k8s.sh; never run directly.

K8S_NS="${PROBE_K8S_NS:-ferroehr-probe}"
K8S_RELEASE="ferroehr"
K8S_PORT="${PROBE_K8S_PORT:-18081}"
K8S_CDR="http://127.0.0.1:${K8S_PORT}"
K8S_API="$K8S_CDR/ferroehr/rest/openehr/v1"
# The identity the documented Basic-auth overlay ships. The chart REFUSES to
# render with authentication on and no mechanism configured, so every install
# below carries deploy/helm/ci/basic-auth-values.yaml — which is the file the
# chart's own error message points an operator at.
K8S_BASIC="clinician:ferroehr"
K8S_AUTH_VALUES="deploy/helm/ci/basic-auth-values.yaml"
K8S_PF_PID=""

# The viewer workload the chart renders behind `viewer.enabled` — a second
# Deployment, Service, ServiceAccount and NetworkPolicy from a second image.
# The name is what the chart's viewerFullname helper produces for this release,
# and the label is the viewer's OWN app.kubernetes.io/name: the two workloads
# deliberately do not share a selector, so a probe that reused the server's
# label would silently measure the server.
K8S_UI="ferroehr-viewer"
K8S_UI_LABEL="app.kubernetes.io/name=$K8S_UI"
# The label the narrowed-ingress probe puts on its client pod, and which the
# viewer NetworkPolicy's `from` selector then names.
K8S_UI_CLIENT_LABEL="ferroehr.probe/client=viewer"
# The viewer image under probe. Empty means the chart's own default, which is
# what an operator installing this chart gets; set PROBE_K8S_VIEWER_IMAGE to
# probe a locally built viewer instead.
K8S_UI_IMAGE="${PROBE_K8S_VIEWER_IMAGE:-}"

# The FerroTERM workload the chart renders behind `terminology.enabled` (#3305,
# #3328) — a third Deployment, Service, ServiceAccount, ConfigMap and
# NetworkPolicy from a third image, with its own copy of the hardened posture.
# The name is what the chart's terminologyFullname helper produces, and the
# label is that workload's OWN app.kubernetes.io/name.
K8S_TERM="ferroehr-terminology"
K8S_TERM_LABEL="app.kubernetes.io/name=$K8S_TERM"
K8S_TERM_PORT="${PROBE_K8S_TERM_PORT:-18092}"
K8S_TERM_URL="http://127.0.0.1:${K8S_TERM_PORT}"
K8S_TERM_PF_PID=""
# The shaped seed the chart ships as a ConfigMap: synthetic content under the
# reserved example.test domain, the same the conformance lane binds to. 1000001
# is a member of sct-shaped-disorders, 1000003 deliberately is not.
K8S_TERM_SYSTEM="http://cnf.example.test/fhir/CodeSystem/sct-shaped"
K8S_TERM_CODE="1000001"
K8S_TERM_TEMPLATE="corpus/templates/dt_coded_text_binding_sct.opt"
K8S_TERM_MEMBER="corpus/fixtures/composition/terminology_binding_sct_member.json"
K8S_TERM_NON_MEMBER="corpus/fixtures/composition/terminology_binding_sct_non_member.json"
# An EXISTING PersistentVolumeClaim holding an index built off-cluster from a
# release the operator holds a licence for. Unset means the index branch is
# declared not exercised rather than faked: an empty directory is not an index.
K8S_TERM_INDEX_CLAIM="${K8S_TERM_INDEX_CLAIM:-}"

# The compose database this cluster is pointed at. It binds 0.0.0.0 because a
# pod reaches the host through the Docker Desktop gateway, which a loopback bind
# would refuse; the stack is thrown away at the end of the run.
K8S_DB_PORT="${PROBE_DB_PORT:-15432}"
K8S_DB_HOST=""

kc() { kubectl -n "$K8S_NS" "$@"; }

# The node as a local CONTAINER, when it is one — which is what makes the
# runtime posture readable at all.
#
# Derived from the cluster's own node names and confirmed with `docker exec`,
# never from `docker ps`: Docker Desktop keeps its Kubernetes node out of the
# user's container list while `docker exec` into it works perfectly. Detecting
# by `docker ps` therefore reports "no local node" on the single most common
# development cluster, and the strongest probe in this file would quietly
# downgrade itself to "not exercised" — a false gap, which is worse than a red
# row because it reads as honesty.
k8s_node_container() {
  local n
  for n in $(kubectl get nodes -o jsonpath='{range .items[*]}{.metadata.name}{"\n"}{end}' 2>/dev/null); do
    if docker exec "$n" true >/dev/null 2>&1; then printf '%s' "$n"; return 0; fi
  done
  return 1
}

# ── Bring-up ──────────────────────────────────────────────────────────────────

# The host address a POD can reach, measured rather than assumed: Docker
# Desktop publishes `host.docker.internal` and a kind-style node also routes its
# default gateway, but which one answers is a property of the cluster, not
# something a script may declare. Echoes the winner; empty means neither works,
# which is a harness problem and not a finding about the chart.
k8s_resolve_db_host() {
  # Retried, because the far end is EVENTUALLY reachable: `pg_isready` answers
  # from inside the container before Docker has necessarily finished publishing
  # the port to the host, and a single attempt then reports "no address works"
  # about a database that is simply not published yet. Asking once cost a run.
  local _try host
  for _try in 1 2 3 4 5; do
    host="$(k8s_try_db_hosts)"
    [[ -n "$host" ]] && { printf '%s' "$host"; return 0; }
    sleep 4
  done
  return 1
}

k8s_try_db_hosts() {
  # Create → wait for completion → read LOGS, never `--attach`: a short-lived
  # container finishes before the attach upgrades and `--rm` deletes the pod,
  # so the output is lost and reads like a database that never answered (the
  # same race that false-flagged P-K8S-UI-SERVE).
  local name="probe-hostcheck-$$-$RANDOM" out phase _i
  # shellcheck disable=SC2016 # $h and $(ip route) belong to the busybox shell, not this one
  kubectl run "$name" --image=busybox:1.37 --restart=Never --command -- sh -c '
      for h in host.docker.internal gateway.docker.internal $(ip route | awk "/default/ {print \$3}"); do
        nc -w3 -z "$h" '"$K8S_DB_PORT"' && { echo "HOST=$h"; break; }
      done' >/dev/null 2>&1
  for _i in $(seq 1 15); do
    phase="$(kubectl get pod "$name" -o jsonpath='{.status.phase}' 2>/dev/null)"
    case "$phase" in Succeeded | Failed) break ;; *) ;; esac
    sleep 2
  done
  out="$(kubectl logs "$name" 2>/dev/null)"
  kubectl delete pod "$name" --ignore-not-found --wait=false >/dev/null 2>&1
  printf '%s' "$out" | sed -n 's/.*HOST=\([^ ]*\).*/\1/p' | tr -d '\r' | head -1
}

k8s_db_up() {
  # The db-publish overlay is required here (#2879): the quickstart file
  # publishes no database port, and this probe is the one consumer that
  # genuinely needs one (cluster pods reach the compose postgres through the
  # Docker Desktop gateway).
  FERROEHR_BIND_HOST=0.0.0.0 FERROEHR_DB_PORT="$K8S_DB_PORT" \
    docker compose -p "$COMPOSE_PROJECT" \
    -f docker-compose.yml -f docker-compose.db-publish.yml \
    up -d ferroehr-postgres >/dev/null 2>&1
  local _i
  for _i in $(seq 1 40); do
    docker compose -p "$COMPOSE_PROJECT" exec -T ferroehr-postgres \
      pg_isready -U "${PG_INIT_USER:-ferroehr}" >/dev/null 2>&1 && return 0
    sleep 2
  done
  return 1
}

k8s_db_stop()  { docker compose -p "$COMPOSE_PROJECT" stop ferroehr-postgres >/dev/null 2>&1; }
k8s_db_start() { docker compose -p "$COMPOSE_PROJECT" start ferroehr-postgres >/dev/null 2>&1; }

k8s_namespace() {
  kubectl create namespace "$K8S_NS" >/dev/null 2>&1 || true
  kc delete secret ferroehr-db >/dev/null 2>&1 || true
  kc create secret generic ferroehr-db \
    --from-literal=FERROEHR__DB__URL="postgres://${PG_INIT_USER:-ferroehr}:${PG_INIT_PASSWORD:-ferroehr}@${K8S_DB_HOST}:${K8S_DB_PORT}/${PG_INIT_DB:-ferroehr}" \
    >/dev/null 2>&1
}

# Install with the chart's OWN defaults plus the things no default can carry —
# where the database is, which image is under probe, and that a probe instance
# sends no usage report. Deliberately nothing else: an overlay tuned until it
# works is exactly what this instrument exists to stop being the only evidence.
k8s_install() {
  helm upgrade --install "$K8S_RELEASE" deploy/helm/ferroehr -n "$K8S_NS" \
    -f "$K8S_AUTH_VALUES" \
    --set usageReport.enabled=false \
    --set database.existingSecret=ferroehr-db \
    --set image.repository="$PROBE_K8S_IMAGE_REPO" \
    --set image.tag="$PROBE_K8S_IMAGE_TAG" \
    --set image.pullPolicy=IfNotPresent \
    ${PROBE_K8S_VALUES:+-f "$PROBE_K8S_VALUES"} \
    "$@" >/dev/null 2>&1
}

k8s_rollout() { kc rollout status "deploy/$K8S_RELEASE" --timeout="${1:-180s}" >/dev/null 2>&1; }

# The same install with the viewer switched on. Everything else stays at the
# chart's defaults, including the viewer's image: an overlay that pins a
# hand-built viewer would stop measuring what `viewer.enabled=true` gives an
# operator.
k8s_ui_install() {
  local args=(--set viewer.enabled=true)
  if [[ -n "$K8S_UI_IMAGE" ]]; then
    args+=(--set viewer.image.repository="${K8S_UI_IMAGE%:*}"
           --set viewer.image.tag="${K8S_UI_IMAGE##*:}")
  fi
  k8s_install "${args[@]}" "$@"
}

# 300s by default: the viewer's own startup probe allows two minutes before it
# gives up, and a first run also pulls the image.
k8s_ui_rollout() { kc rollout status "deploy/$K8S_UI" --timeout="${1:-300s}" >/dev/null 2>&1; }

# An HTTP GET issued from INSIDE the cluster, through the viewer's Service.
#
# Deliberately not a port-forward: a forward is the kubelet reaching a pod
# directly, which skips Service resolution, cluster DNS and every ingress rule
# between two pods — the hops a human's request actually takes. The client pod
# carries the label the narrowed-ingress policy names, so the same call measures
# the stock and the narrowed posture.
#
# The response is read from the finished pod's LOGS, never from `run --attach`:
# a short-lived container regularly completes before the attach is established
# ("couldn't attach … falling back to streaming logs: unable to upgrade
# connection"), and the body then arrives EMPTY, which reads exactly like a
# viewer that did not answer. That cost a red row here before it was understood.
k8s_ui_get() {
  local name="probe-ui-$$-$RANDOM" out phase _i
  kubectl -n "$K8S_NS" run "$name" --image=busybox:1.37 --restart=Never \
    --labels="$K8S_UI_CLIENT_LABEL" \
    --command -- wget -q -O - "http://$K8S_UI:3000${1:-/login}" >/dev/null 2>&1
  for _i in $(seq 1 30); do
    phase="$(kc get pod "$name" -o jsonpath='{.status.phase}' 2>/dev/null)"
    case "$phase" in Succeeded | Failed) break ;; *) ;; esac
    sleep 2
  done
  out="$(kc logs "$name" 2>/dev/null)"
  kc delete pod "$name" --ignore-not-found --wait=false >/dev/null 2>&1
  printf '%s' "$out"
}

# A READY pod, preferred over merely the first one.
#
# `.items[0]` is whichever pod the API server lists first, which after a probe
# that deliberately breaks the boot is the crash-looping one still terminating.
# Port-forwarding to that pod waits for a container that will never serve, and
# reports it as a finding about readiness. Falls back to any pod so a genuine
# "nothing is ready" still surfaces rather than returning empty.
k8s_pod() {
  local ready
  ready="$(kc get pod -l app.kubernetes.io/name=ferroehr \
    -o jsonpath='{range .items[?(@.status.containerStatuses[0].ready==true)]}{.metadata.name}{"\n"}{end}' \
    2>/dev/null | head -1)"
  [[ -n "$ready" ]] && { printf '%s' "$ready"; return 0; }
  kc get pod -l app.kubernetes.io/name=ferroehr -o jsonpath='{.items[0].metadata.name}' 2>/dev/null
}

k8s_pf_start() {
  kc port-forward "svc/$K8S_RELEASE" "${K8S_PORT}:8080" >/dev/null 2>&1 &
  K8S_PF_PID=$!
  wait_http "$K8S_CDR/health/liveness" 45
}

# A forward bound to ONE POD rather than the Service.
#
# Needed for exactly one probe, for a reason worth stating: `port-forward
# svc/…` resolves the Service to a READY endpoint, so the moment the pods go
# unready it has nothing to forward to and every request fails to connect. That
# is the precise state the readiness probe exists to observe, so measuring it
# through a Service forward reports a connection failure and calls it a finding
# about the server.
k8s_pf_pod_start() {
  k8s_pf_stop
  kc port-forward "pod/$1" "${K8S_PORT}:8080" >/dev/null 2>&1 &
  K8S_PF_PID=$!
  wait_http "$K8S_CDR/health/liveness" 30
}

k8s_pf_stop() {
  # `disown` first: killing a backgrounded job otherwise makes the shell print
  # a "Terminated" notice into the report, which reads like a failure in an
  # instrument whose whole value is that its output means something.
  if [[ -n "$K8S_PF_PID" ]]; then
    disown "$K8S_PF_PID" 2>/dev/null || true
    kill "$K8S_PF_PID" >/dev/null 2>&1 || true
    wait "$K8S_PF_PID" 2>/dev/null || true
  fi
  K8S_PF_PID=""
}

k8s_teardown() {
  k8s_pf_stop
  k8s_term_pf_stop
  helm uninstall "$K8S_RELEASE" -n "$K8S_NS" >/dev/null 2>&1 || true
  kubectl delete namespace "$K8S_NS" --wait=false >/dev/null 2>&1 || true
  docker compose -p "$COMPOSE_PROJECT" down -v --remove-orphans >/dev/null 2>&1 || true
}

# ── The probes ────────────────────────────────────────────────────────────────

probes_k8s_boot() {
  bold "the chart on a real cluster"

  # A stock install must fail where an operator can read it. Authentication is
  # on by default and no mechanism is configured by default, and the failure
  # mode that costs an afternoon is a pod that starts, crash-loops, and explains
  # itself only in a log nobody has tailed yet. So the chart refuses at RENDER —
  # and the refusal has to name the ways out, or it is just a different wall.
  probe "P-K8S-AUTH-GUARD" "broken" "chart" "-" \
    "a stock install is refused at render, naming every configured way forward"
  local guard
  guard="$(helm template "$K8S_RELEASE" deploy/helm/ferroehr -n "$K8S_NS" \
             --set database.existingSecret=ferroehr-db 2>&1)"
  assert_contains "$guard" "no authentication mechanism is configured" \
    "without this the stock install boots into a crash loop and explains itself in a log"
  assert_contains "$guard" "config.auth.basic.users" \
    "a refusal that does not name the Basic route leaves an operator guessing"
  assert_contains "$guard" "config.auth.oidc.issuer" \
    "the OIDC route is the other real answer and must be named too"
  assert_contains "$guard" "config.auth.enabled=false" \
    "the development escape must be stated as such, not discovered"
  probe_done

  # #2159's class on this platform: our own values files rendered and linted for
  # a whole release cycle while none of them produced a bootable configuration.
  # Rendering is not installing, and only one of the two is what an operator does.
  probe "P-K8S-BOOT" "working" "chart" "#2159" \
    "the chart's default values install and the Deployment rolls out"
  if ! k8s_install; then
    probe_fail "a successful helm upgrade --install" "helm refused the release" \
      "the render itself failed; re-run the helm command without -q to see it"
    probe_done
    return 1
  fi
  if ! k8s_rollout; then
    probe_fail "a rolled-out Deployment" \
      "$(kc logs -l app.kubernetes.io/name=ferroehr --tail=6 --all-containers 2>&1 | tail -6)" \
      "a config key the chart renders that this image does not accept lands here"
    probe_done
    return 1
  fi
  probe_done

  # The proof an operator cares about: not that pods are Running, but that the
  # deployment answers openEHR. Everything below this needs the port-forward.
  probe "P-K8S-SERVE" "working" "server" "-" \
    "the deployed CDR serves a real openEHR write and read"
  if ! k8s_pf_start; then
    probe_fail "a reachable Service" "port-forward never answered /health/liveness"
    probe_done
    return 1
  fi
  local headers ehr
  headers="$(curl -s -u "$K8S_BASIC" -X POST -D - -o /dev/null "$K8S_API/ehr")"
  ehr="$(printf '%s' "$headers" | grep -i '^location' | tr -d '\r' | awk -F/ '{print $NF}')"
  if [[ -z "$ehr" ]]; then
    probe_fail "a Location header naming the new EHR" "$(printf '%s' "$headers" | head -3)"
  else
    assert_contains "$(curl -s -u "$K8S_BASIC" "$K8S_API/ehr/$ehr")" "\"$ehr\"" \
      "the EHR just created must read back from the deployed server"
  fi
  probe_done
  return 0
}

# The hardened posture, asserted against a container runtime spec — shared by
# the server and the viewer because a second workload is exactly where such a
# posture is quietly lost, and a weaker check for the second one would hide it.
k8s_assert_hardened() {
  local spec="$1"
  assert_eq "65532" "$(printf '%s' "$spec" | jq -r '.process.user.uid')" \
    "the image's nonroot uid must be what the process actually runs as"
  assert_eq "true" "$(printf '%s' "$spec" | jq -r '.process.noNewPrivileges')" \
    "without this a setuid binary inside the image could still escalate"
  # An EMPTY bounding set, not "the default fourteen minus the ones we dropped":
  # drop: [ALL] is only honoured if the runtime clears the bounding set, and
  # nothing short of reading it proves that.
  assert_eq "0" "$(printf '%s' "$spec" | jq -r '.process.capabilities.bounding | length')" \
    "a non-empty bounding set means drop: [ALL] did not take effect"
  assert_eq "true" "$(printf '%s' "$spec" | jq -r '.root.readonly')" \
    "a writable root filesystem makes the container's own binaries replaceable"
  assert_eq "SCMP_ACT_ERRNO" "$(printf '%s' "$spec" | jq -r '.linux.seccomp.defaultAction')" \
    "RuntimeDefault must resolve to a deny-by-default filter, not to no filter at all"
}

# What the kernel actually got, read from the container runtime rather than from
# the manifest. This is the whole reason to run on a cluster: `securityContext`
# is a REQUEST, and a request that a runtime quietly declines looks identical in
# `kubectl get pod -o yaml`.
probes_k8s_runtime_posture() {
  bold "the security posture, read out of the container runtime"

  local node
  node="$(k8s_node_container)"
  if [[ -z "$node" ]]; then
    uncovered "the applied runtime posture (uid, capabilities, seccomp, read-only root)" \
      "this cluster's node is not a local container, so its runtime spec is not readable from here"
    return 0
  fi

  probe "P-K8S-RUNTIME" "working" "chart" "-" \
    "the runtime applied non-root, no-new-privileges, no capabilities, read-only root, seccomp"
  local cid spec
  cid="$(docker exec "$node" crictl ps --name ferroehr -q 2>/dev/null | head -1)"
  if [[ -z "$cid" ]]; then
    probe_fail "a running ferroehr container on the node" "crictl listed none"
    probe_done
    return 0
  fi
  spec="$(docker exec "$node" crictl inspect "$cid" 2>/dev/null | jq -c '.info.runtimeSpec')"

  k8s_assert_hardened "$spec"
  probe_done

  # The mounts, from the same source. `/etc/ferroehr` holding the configuration
  # and `/etc/ferroehr-secrets` holding key material must both be read-only:
  # a writable configuration mount is a way to change the server's behaviour
  # from inside a compromised container.
  probe "P-K8S-MOUNTS" "working" "chart" "-" \
    "configuration and secret mounts are read-only in the runtime spec"
  local writable
  writable="$(printf '%s' "$spec" | jq -r '
    .mounts[] | select(.destination | test("^/etc/ferroehr"))
    | select((.options // []) | index("ro") | not) | .destination' | tr '\n' ' ')"
  if [[ -n "${writable// /}" ]]; then
    probe_fail "every /etc/ferroehr* mount carrying 'ro'" "writable: $writable" \
      "configuration and key material must not be rewritable from inside the container"
  fi
  probe_done
}

# The API server as the judge. Labelling the namespace and rolling the workload
# is the only way to learn whether the chart's pods are admissible under the
# Restricted profile — reading the profile ourselves and grading our own
# manifest is the check that cannot fail honestly.
probes_k8s_psa() {
  bold "Pod Security admission (the API server judges, not us)"

  probe "P-K8S-PSA" "working" "chart" "-" \
    "the chart's pods are admitted under the Restricted profile"
  kubectl label namespace "$K8S_NS" \
    pod-security.kubernetes.io/enforce=restricted --overwrite >/dev/null 2>&1
  kc rollout restart "deploy/$K8S_RELEASE" >/dev/null 2>&1
  if ! k8s_rollout 180s; then
    probe_fail "an admitted rollout under enforce=restricted" \
      "$(kc get events --sort-by=.lastTimestamp 2>/dev/null | grep -i 'violate\|forbidden' | tail -3)" \
      "the chart claims Restricted compliance; admission is what settles it"
  fi
  probe_done

  # Acceptance must not be able to pass vacuously: if the label were silently
  # ignored, the probe above would pass on a cluster enforcing nothing. So a pod
  # that MUST be refused is offered, and its refusal is what proves the judge
  # was awake.
  probe "P-K8S-PSA-ARMED" "broken" "compose" "-" \
    "a privileged pod is REFUSED, proving enforcement is really on"
  local out
  out="$(kc run probe-privileged --image=busybox:1.37 --restart=Never \
      --overrides='{"spec":{"containers":[{"name":"bad","image":"busybox:1.37","command":["sleep","5"],"securityContext":{"privileged":true}}]}}' \
      2>&1)"
  assert_contains "$out" "violates PodSecurity" \
    "without a refusal here the admission probe above proves nothing about this cluster"
  kc delete pod probe-privileged --ignore-not-found >/dev/null 2>&1
  probe_done

  kubectl label namespace "$K8S_NS" pod-security.kubernetes.io/enforce- >/dev/null 2>&1
}

# Service links are a real trap on this platform: the kubelet injects
# <SVC>_SERVICE_HOST and friends for every Service in the namespace, and for a
# Service named `ferroehr*` those land inside the server's reserved FERROEHR_
# namespace, where its strict environment sweep refuses to boot. The chart pins
# enableServiceLinks: false; this proves the pin still holds where it matters.
probes_k8s_service_links() {
  bold "service-link injection (the trap that stops this server booting)"

  probe "P-K8S-SVCLINKS" "working" "chart" "-" \
    "a new ferroehr-named Service does not poison the pod's environment"
  kc create service clusterip ferroehr-probe-link --tcp=8080:8080 >/dev/null 2>&1 || true
  kc rollout restart "deploy/$K8S_RELEASE" >/dev/null 2>&1
  if ! k8s_rollout 180s; then
    probe_fail "a pod that boots with another ferroehr-named Service present" \
      "$(kc logs -l app.kubernetes.io/name=ferroehr --tail=5 --all-containers 2>&1 | tail -5)" \
      "this is what enableServiceLinks: false prevents; a boot refusal here means the pin is gone"
  fi
  kc delete service ferroehr-probe-link --ignore-not-found >/dev/null 2>&1
  probe_done
}

probes_k8s_secrets() {
  bold "secrets: where they are, and whether they are actually read"

  # A DSN is a credential. It must reach the process as a PATH, never as a value
  # in the pod spec — `kubectl describe pod` and every controller that logs a
  # spec would otherwise carry it.
  probe "P-K8S-SECRET-ENV" "working" "chart" "-" \
    "no credential value appears in the pod environment or the ConfigMap"
  local env_dump cm_dump
  env_dump="$(kc get pod "$(k8s_pod)" \
    -o jsonpath='{range .spec.containers[0].env[*]}{.name}={.value}{"\n"}{end}' 2>/dev/null)"
  assert_contains "$env_dump" "FERROEHR__DB__URL_FILE=" \
    "the DSN must arrive as a file path the server reads, not as an environment value"
  assert_not_contains "$env_dump" "postgres://" \
    "a DSN in the pod spec is readable by anyone who can describe the pod"
  cm_dump="$(kc get configmap -o yaml 2>/dev/null)"
  assert_not_contains "$cm_dump" "argon2id" \
    "a password hash in a ConfigMap is a secret stored where secrets are not"
  assert_not_contains "$cm_dump" "postgres://" \
    "the ConfigMap is not a place for a DSN"
  probe_done

  # Mounted and READ are different claims, and only one of them is worth
  # anything. Breaking the path is the only way to tell them apart: a secret
  # that is projected and never opened looks identical from outside.
  probe "P-K8S-SECRET-READ" "broken" "chart" "-" \
    "a *_file key pointing at a missing path makes the boot REFUSE, naming the file"
  k8s_install --set config.signing.enabled=true \
              --set config.signing.mode=pgp \
              --set config.signing.key_passphrase_file=/etc/ferroehr-secrets/absent
  # The rollout is EXPECTED to fail here; what matters is the reason it gives.
  k8s_rollout 90s
  local bad_logs
  bad_logs="$(kc logs -l app.kubernetes.io/name=ferroehr --tail=25 --all-containers 2>&1 \
              | grep -i 'absent\|secret file' | tail -3)"
  assert_contains "$bad_logs" "/etc/ferroehr-secrets/absent" \
    "the refusal must name the unreadable path, or an operator cannot act on it"
  probe_done

  # Back to the shipped posture for everything that follows.
  k8s_install
  k8s_rollout 180s
}

# Readiness that has only ever returned UP is not a health check, it is a
# decoration. The far end here is the EndpointSlice: a pod that reports itself
# unready must actually leave the Service, and liveness must NOT restart it —
# restarting a healthy process because its database is down turns an outage
# into a crash loop.
probes_k8s_readiness() {
  bold "readiness under a broken dependency (the state least often tested)"

  probe "P-K8S-READY-GATES" "broken" "server" "-" \
    "with the database gone, readiness fails, the pod leaves the Service, and liveness does not restart it"
  local pod restarts_before
  pod="$(k8s_pod)"
  restarts_before="$(kc get pod "$pod" -o jsonpath='{.status.containerStatuses[0].restartCount}' 2>/dev/null)"
  if ! k8s_pf_pod_start "$pod"; then
    probe_fail "a forward to the pod under test" "it never answered /health/liveness"
    probe_done
    return 0
  fi

  k8s_db_stop
  if ! wait_status "$K8S_CDR/health/readiness" "503" 40; then
    probe_fail "readiness answering 503 once the database is unreachable" \
      "$(curl -s -o /dev/null -w '%{http_code}' "$K8S_CDR/health/readiness")" \
      "a readiness probe that stays UP through a dead database gates nothing"
    k8s_db_start
    probe_done
    return 0
  fi
  assert_contains "$(curl -s "$K8S_CDR/health/readiness")" '"db"' \
    "the readiness body must name the component that failed, not just report DOWN"
  assert_eq "200" "$(http_code "$K8S_CDR/health/liveness")" \
    "liveness must stay green: the process is healthy, its dependency is not"

  # The two far ends, in order of authority. The KUBELET's verdict comes from
  # its own probe rather than ours, and the EndpointSlice is what actually
  # stops kube-proxy sending traffic here — a pod that reports itself unready
  # and stays in the Service is still serving production requests. Both are
  # eventually consistent, so each is waited for rather than sampled once.
  local _i ready="unknown" kubelet="unknown"
  for _i in $(seq 1 20); do
    kubelet="$(kc get pod "$pod" -o jsonpath='{.status.containerStatuses[0].ready}' 2>/dev/null)"
    [[ "$kubelet" = "false" ]] && break
    sleep 3
  done
  assert_eq "false" "$kubelet" \
    "the kubelet's own readiness probe must agree; ours answering 503 is only half the claim"
  for _i in $(seq 1 20); do
    ready="$(kc get endpointslice -l "kubernetes.io/service-name=$K8S_RELEASE" \
             -o jsonpath='{.items[0].endpoints[0].conditions.ready}' 2>/dev/null)"
    [[ "$ready" = "false" ]] && break
    sleep 3
  done
  assert_eq "false" "$ready" \
    "an unready pod that stays in the EndpointSlice still receives production traffic"

  k8s_db_start
  if ! wait_status "$K8S_CDR/health/readiness" "200" 60; then
    probe_fail "readiness recovering once the database returns" "still not 200 after 120s" \
      "recovery without a restart is the point; a probe that latches DOWN needs a rollout to clear"
  fi
  local restarts_after
  restarts_after="$(kc get pod "$pod" -o jsonpath='{.status.containerStatuses[0].restartCount}' 2>/dev/null)"
  assert_eq "${restarts_before:-0}" "${restarts_after:-x}" \
    "liveness restarted a process whose only problem was its dependency"
  probe_done
}

# The viewer, the chart's optional second workload (#2354).
#
# It is off by default, which is exactly why it needs probing: the chart renders
# a full Deployment, Service, ServiceAccount and NetworkPolicy behind
# `viewer.enabled`, from a second image, with its own copy of the hardened
# security context — and a posture carried in a second place is a posture that
# is quietly lost. Everything below runs LAST, because it changes the release's
# shape and the probes above measure the shipped default.
probes_k8s_viewer() {
  bold "the viewer workload (viewer.enabled)"

  # The OFF state, on the release the probes above have been measuring: the gate
  # must render nothing at all, not merely a scaled-down viewer.
  probe "P-K8S-UI-OFF" "off" "chart" "-" \
    "the shipped default renders no viewer object at all"
  local off
  off="$(kc get deploy,svc,serviceaccount,networkpolicy -l "$K8S_UI_LABEL" \
         -o name 2>/dev/null | tr '\n' ' ')"
  assert_eq "" "${off// /}" \
    "viewer.enabled defaults to false, so anything found here means the gate leaks"
  probe_done

  # The BROKEN state of the viewer's ingress posture: asking for a narrowed
  # ingress while naming no peer must be REFUSED at render, because the
  # alternative is a NetworkPolicy that reads as default-deny and admits every
  # source to a login page.
  probe "P-K8S-UI-NETPOL-GUARD" "broken" "chart" "-" \
    "a viewer with ingressAllowAll=false and no ingressFrom is refused at render"
  local guard
  guard="$(helm template "$K8S_RELEASE" deploy/helm/ferroehr -n "$K8S_NS" \
             -f "$K8S_AUTH_VALUES" --set database.existingSecret=ferroehr-db \
             --set viewer.enabled=true \
             --set viewer.networkPolicy.ingressAllowAll=false 2>&1)"
  assert_contains "$guard" "viewer.networkPolicy.ingressFrom" \
    "a refusal that does not name the key that narrows the sources is just a wall"
  probe_done

  probe "P-K8S-UI-BOOT" "working" "chart" "-" \
    "viewer.enabled installs the viewer workload and it rolls out"
  if ! k8s_ui_install; then
    probe_fail "a successful helm upgrade --install with the viewer on" \
      "helm refused the release" \
      "re-run k8s_ui_install without the output redirect to see the render error"
    probe_done
    uncovered "every viewer probe after P-K8S-UI-BOOT" \
      "the viewer workload never installed, so nothing about it could be observed"
    return 0
  fi
  if ! k8s_ui_rollout; then
    probe_fail "a rolled-out viewer Deployment" \
      "$(kc logs -l "$K8S_UI_LABEL" --tail=6 --all-containers 2>&1 | tail -6
         kc get pod -l "$K8S_UI_LABEL" -o jsonpath='{.items[0].status.containerStatuses[0].state.waiting.reason} {.items[0].status.containerStatuses[0].state.waiting.message}' 2>/dev/null)" \
      "an unpullable viewer image and a viewer that cannot start under the hardened context both land here"
    probe_done
    uncovered "every viewer probe after P-K8S-UI-BOOT" \
      "the viewer workload never became ready, so nothing about it could be observed"
    return 0
  fi
  probe_done

  probes_k8s_ui_runtime
  probes_k8s_ui_psa
  probes_k8s_ui_serve
  probes_k8s_ui_netpol
}

# The viewer's applied posture, read from the container runtime for the same
# reason the server's is: `securityContext` is a request, and the viewer's copy
# of it is a second place for that request to go unhonoured.
probes_k8s_ui_runtime() {
  local node cid spec
  node="$(k8s_node_container)"
  if [[ -z "$node" ]]; then
    uncovered "the viewer's applied runtime posture (uid, capabilities, seccomp, read-only root)" \
      "this cluster node is not a local container, so its runtime spec is not readable from here"
    return 0
  fi
  probe "P-K8S-UI-RUNTIME" "working" "chart" "-" \
    "the viewer container runs under the same hardened posture as the server"
  cid="$(docker exec "$node" crictl ps --name viewer -q 2>/dev/null | head -1)"
  if [[ -z "$cid" ]]; then
    probe_fail "a running viewer container on the node" "crictl listed none"
  else
    spec="$(docker exec "$node" crictl inspect "$cid" 2>/dev/null | jq -c '.info.runtimeSpec')"
    k8s_assert_hardened "$spec"
  fi
  probe_done
}

# Admission, for the second workload. The server passing under enforce=restricted
# says nothing about the viewer: the profile is judged per pod, and the viewer
# carries its own security context rather than inheriting the server's.
probes_k8s_ui_psa() {
  probe "P-K8S-UI-PSA" "working" "chart" "-" \
    "the viewer pod is admitted under the Restricted profile"
  kubectl label namespace "$K8S_NS" \
    pod-security.kubernetes.io/enforce=restricted --overwrite >/dev/null 2>&1
  kc rollout restart "deploy/$K8S_UI" >/dev/null 2>&1
  if ! k8s_ui_rollout 240s; then
    probe_fail "an admitted viewer rollout under enforce=restricted" \
      "$(kc get events --sort-by=.lastTimestamp 2>/dev/null | grep -i 'violate\|forbidden' | tail -3)" \
      "the chart holds both workloads to Restricted; admission is what settles it for the second one"
  fi
  # Removed again before any client pod runs: the probe pods below are plain
  # busybox and would themselves be refused under the profile.
  kubectl label namespace "$K8S_NS" pod-security.kubernetes.io/enforce- >/dev/null 2>&1
  probe_done
}

probes_k8s_ui_serve() {
  probe "P-K8S-UI-SERVE" "working" "server" "-" \
    "the viewer serves its login page through its Service, to a client inside the cluster"
  local page
  page="$(k8s_ui_get /login)"
  assert_contains "$page" 'name="username"' \
    "the sign-in form must be in the first response: /login renders SsrMode::Async so it works without JavaScript"
  assert_contains "$page" "FerroEHR Viewer" \
    "the page served must be the viewer's own, not an error page from something else on that port"
  probe_done

  # What a served login page does NOT prove, said here rather than left to the
  # reader: login_modes falls back to the viewer's own configuration when the
  # CDR is unreachable, so the form renders either way.
  uncovered "the viewer reaching the CDR over the in-cluster Service" \
    "the login page renders from the viewer configuration alone when the CDR is unreachable, so serving it is not evidence of the REST hop"
  uncovered "the viewer screens behind a session, and its OIDC sign-in path" \
    "scripts/ui-e2e.sh drives the browser journeys against compose, and OIDC needs an identity provider plus a client secret this harness does not run"
}

# The narrowed ingress posture, which no run had ever installed: the chart's
# viewer policy admits every source unless `ingressFrom` names peers, and the
# narrowing path had only ever been rendered.
probes_k8s_ui_netpol() {
  cat > "$PROBE_TMP/viewer-narrow.yaml" <<YAML
viewer:
  networkPolicy:
    ingressAllowAll: false
    ingressFrom:
      - podSelector:
          matchLabels:
            ${K8S_UI_CLIENT_LABEL%=*}: ${K8S_UI_CLIENT_LABEL#*=}
YAML

  probe "P-K8S-UI-NETPOL-NARROW" "working" "chart" "-" \
    "the narrowed viewer ingress installs, is stored as written, and still admits the peer it names"
  if ! k8s_ui_install -f "$PROBE_TMP/viewer-narrow.yaml"; then
    probe_fail "an install with ingressAllowAll=false and a non-empty ingressFrom" \
      "helm refused the release" \
      "the guard above must fire only on an EMPTY ingressFrom; refusing this one makes the narrowing path unusable"
    probe_done
    return 0
  fi
  local stored
  stored="$(kc get networkpolicy "$K8S_UI" -o json 2>/dev/null \
            | jq -r --arg k "${K8S_UI_CLIENT_LABEL%=*}" '.spec.ingress[0].from[0].podSelector.matchLabels[$k] // ""')"
  assert_eq "${K8S_UI_CLIENT_LABEL#*=}" "$stored" \
    "the API server must have stored the from selector; a pruned field would leave the rule admitting everything"
  assert_contains "$(k8s_ui_get /login)" 'name="username"' \
    "a narrowed policy that also stops the peer it names is a broken recipe, not a hardened one"
  probe_done
}

# ── FerroTERM, the chart's terminology workload (#3305, #3328) ────────────────
#
# The compose twin (scripts/deploy-probes/terminology.sh, P-FT-*) measures
# whether the CDR resolves a binding against FerroTERM. This family measures the
# cluster's half of the same claim: a second hardened posture the API server and
# the runtime each get a say over, a seed that reaches the process through a
# ConfigMap volume the kubelet builds out of symlinks, and a NetworkPolicy whose
# whole value is a refusal.
#
# It runs LAST for the same reason the viewer family does: it changes the shape
# of the release the probes above measured.

k8s_term_install() {
  # The chart's own terminology defaults, plus the CDR's read-only terminology
  # routes — which is the surface an operator watches a resolution through, and
  # the only way to see a call leave the CDR pod rather than infer it.
  k8s_install --set terminology.enabled=true \
              --set config.terminology.api_enabled=true "$@"
}

# 300s: the terminology container's own startup probe allows five minutes, and a
# first run also pulls the image.
k8s_term_rollout() { kc rollout status "deploy/$K8S_TERM" --timeout="${1:-300s}" >/dev/null 2>&1; }

k8s_term_pf_start() {
  k8s_term_pf_stop
  kc port-forward "svc/$K8S_TERM" "${K8S_TERM_PORT}:8080" >/dev/null 2>&1 &
  K8S_TERM_PF_PID=$!
  wait_http "$K8S_TERM_URL/health" 30
}

k8s_term_pf_stop() {
  if [[ -n "$K8S_TERM_PF_PID" ]]; then
    disown "$K8S_TERM_PF_PID" 2>/dev/null || true
    kill "$K8S_TERM_PF_PID" >/dev/null 2>&1 || true
    wait "$K8S_TERM_PF_PID" 2>/dev/null || true
  fi
  K8S_TERM_PF_PID=""
}

# TCP reachability to the terminology port from a throwaway pod in <namespace>.
# Echoes the exit status busybox's `nc` reported: 0 admitted, 1 refused.
#
# Read from the finished pod's LOGS rather than `run --attach`, the race that
# cost this file a red row twice: a short-lived container regularly completes
# before the attach is established, and the output is then lost.
k8s_term_nc() {
  local ns="$1" ip="$2" name="probe-term-nc-$$-$RANDOM" out phase _i
  # shellcheck disable=SC2016 # $? belongs to the busybox shell, not this one
  kubectl -n "$ns" run "$name" --image=busybox:1.37 --restart=Never \
    --env="TERM_IP=$ip" --command -- sh -c 'nc -w4 -z "$TERM_IP" 8080; echo exit=$?' \
    >/dev/null 2>&1
  for _i in $(seq 1 25); do
    phase="$(kubectl -n "$ns" get pod "$name" -o jsonpath='{.status.phase}' 2>/dev/null)"
    case "$phase" in Succeeded | Failed) break ;; *) ;; esac
    sleep 2
  done
  out="$(kubectl -n "$ns" logs "$name" 2>/dev/null)"
  kubectl -n "$ns" delete pod "$name" --ignore-not-found --wait=false >/dev/null 2>&1
  printf '%s' "$out" | tr -d '\r' | sed -n 's/.*exit=\([0-9]*\).*/\1/p' | head -1
}

k8s_term_template() {
  local code
  code="$(curl -s -u "$K8S_BASIC" -o /dev/null -w '%{http_code}' -X POST \
    -H 'Content-Type: application/xml' --data-binary "@$K8S_TERM_TEMPLATE" \
    "$K8S_API/definition/template/adl1.4")"
  case "$code" in 201 | 204 | 409) return 0 ;; *) return 1 ;; esac
}

# The status code a commit of the given fixture into a fresh EHR answers.
k8s_term_commit_code() {
  local hdr="$PROBE_TMP/k8s-term-ehr.txt" ehr
  curl -s -u "$K8S_BASIC" -X POST -D "$hdr" -o /dev/null "$K8S_API/ehr" || { printf '000'; return 0; }
  ehr="$(grep -i '^location' "$hdr" 2>/dev/null | tr -d '\r' | awk -F/ '{print $NF}')"
  [[ -n "$ehr" ]] || { printf '000'; return 0; }
  curl -s -u "$K8S_BASIC" -o /dev/null -w '%{http_code}' -X POST \
    -H 'Content-Type: application/json' -H 'Prefer: return=minimal' \
    --data-binary "@$1" "$K8S_API/ehr/$ehr/composition"
}

probes_k8s_terminology() {
  bold "the terminology workload (terminology.enabled)"

  # The OFF state, on the release the probes above have been measuring. The
  # ConfigMap is named separately: a seed left behind by a disabled workload is
  # a mount nobody is watching.
  probe "P-K8S-TERM-OFF" "off" "chart" "#3328" \
    "the shipped default renders no terminology object at all"
  local off
  off="$(kc get deploy,svc,configmap,networkpolicy,serviceaccount -l "$K8S_TERM_LABEL" \
         -o name 2>/dev/null | tr '\n' ' ')"
  assert_eq "" "${off// /}" \
    "terminology.enabled defaults to false, so anything found here means the gate leaks"
  probe_done

  probe "P-K8S-TERM-BOOT" "working" "chart" "#3328" \
    "terminology.enabled installs the FerroTERM workload and it rolls out"
  if ! k8s_term_install; then
    probe_fail "a successful helm upgrade --install with terminology on" \
      "helm refused the release" \
      "re-run k8s_term_install without the output redirect to see the render error"
    probe_done
    uncovered "every terminology probe after P-K8S-TERM-BOOT" \
      "the terminology workload never installed, so nothing about it could be observed"
    return 0
  fi
  if ! k8s_term_rollout; then
    probe_fail "a rolled-out terminology Deployment" \
      "$(kc logs -l "$K8S_TERM_LABEL" --tail=6 --all-containers 2>&1 | tail -6
         kc get pod -l "$K8S_TERM_LABEL" -o jsonpath='{.items[0].status.containerStatuses[0].state.waiting.reason} {.items[0].status.containerStatuses[0].state.waiting.message}' 2>/dev/null)" \
      "an unpullable image and a server that cannot start under the hardened context both land here"
    probe_done
    uncovered "every terminology probe after P-K8S-TERM-BOOT" \
      "the terminology workload never became ready, so nothing about it could be observed"
    return 0
  fi
  # The CDR rolls too: the injected [terminology.external] rewrites its
  # ConfigMap, whose checksum is a pod annotation. Its Service forward is
  # therefore rebuilt before anything below asks the CDR a question.
  k8s_rollout 180s
  k8s_pf_stop
  k8s_pf_start || true
  probe_done

  probes_k8s_term_runtime
  probes_k8s_term_seed
  probes_k8s_term_netpol
  probes_k8s_term_resolve
  probes_k8s_term_index

  uncovered "a licensed terminology edition served by this workload" \
    "the SNOMED CT and LOINC releases are licensed content this repository may not ship and no lane may fetch, so every probe here runs against the chart's synthetic shaped seed"
  uncovered "FerroTERM unreachable, under each of the CDR's fail postures" \
    "terminology.failOnError decides whether an unresolvable binding commits or is refused, and the compose family (P-FT-DOWN-OPEN, P-FT-DOWN-CLOSED) is where that pair is driven"
}

# The third copy of the hardened posture, read from the container runtime rather
# than from the manifest — the reason being the reason it is read for the server
# and the viewer: `securityContext` is a request a runtime may decline, and a
# third workload is a third place for it to go unhonoured.
probes_k8s_term_runtime() {
  local node cid spec
  node="$(k8s_node_container)"
  if [[ -z "$node" ]]; then
    uncovered "the terminology container's applied runtime posture (uid, capabilities, seccomp, read-only root)" \
      "this cluster node is not a local container, so its runtime spec is not readable from here"
    return 0
  fi
  probe "P-K8S-TERM-RUNTIME" "working" "chart" "#3328" \
    "the terminology container runs under the same hardened posture as the server"
  cid="$(docker exec "$node" crictl ps --name terminology -q 2>/dev/null | head -1)"
  if [[ -z "$cid" ]]; then
    probe_fail "a running terminology container on the node" "crictl listed none"
  else
    spec="$(docker exec "$node" crictl inspect "$cid" 2>/dev/null | jq -c '.info.runtimeSpec')"
    k8s_assert_hardened "$spec"
  fi
  probe_done
}

# The seed, at the far end. The chart renders a ConfigMap and mounts it; the
# kubelet projects it as a directory of symlinks into a `..data` snapshot, which
# is a shape a naive directory scan skips entirely — so "the ConfigMap exists"
# and "the process is serving what is in it" are different claims, and only the
# second is worth a row.
probes_k8s_term_seed() {
  probe "P-K8S-TERM-SEED" "working" "server" "#3328" \
    "the shaped seed reaches the process from the ConfigMap volume and is served"
  if ! k8s_term_pf_start; then
    probe_fail "a reachable terminology Service" "port-forward never answered /health"
    probe_done
    return 0
  fi
  local logs
  logs="$(kc logs -l "$K8S_TERM_LABEL" --tail=80 --all-containers 2>/dev/null)"
  # The chart's default terminology.logFormat is `json`, so the boot line is a
  # JSON field rather than the key=value form a pretty log prints.
  assert_contains "$logs" '"code_systems":' \
    "FerroTERM reports the number of code systems it loaded at boot; without the field nothing says the directory was read"
  assert_not_contains "$logs" '"code_systems":0' \
    "a boot that read the volume and found nothing in it starts and serves, and looks identical from outside"
  assert_contains "$logs" "$K8S_TERM_SYSTEM" \
    "the seed's own code system must be among the ones served, or the volume delivered something else"
  local lookup
  lookup="$(curl -s --get \
    --data-urlencode "system=$K8S_TERM_SYSTEM" \
    --data-urlencode "code=$K8S_TERM_CODE" \
    "$K8S_TERM_URL/r4b/CodeSystem/\$lookup")"
  assert_contains "$lookup" '"resourceType"' \
    "a \$lookup on a seed code must answer a FHIR resource; a loaded-but-unserved seed answers an error here"
  probe_done
}

# The one policy in this chart whose ingress is narrowed by DEFAULT, so the claim
# it makes is a REFUSAL — and a refusal is the only kind of network claim that
# cannot pass vacuously on a cluster enforcing nothing: an unenforcing CNI turns
# both calls below into exit=0 and fails the row.
probes_k8s_term_netpol() {
  probe "P-K8S-TERM-NETPOL" "working" "chart" "#3328" \
    "the terminology port refuses a pod outside the CDR's selector, in this namespace and in another, while the CDR itself is admitted"
  local ip
  ip="$(kc get pod -l "$K8S_TERM_LABEL" -o jsonpath='{.items[0].status.podIP}' 2>/dev/null)"
  if [[ -z "$ip" ]]; then
    probe_fail "a terminology pod with an address" "none was reported" \
      "without an address the refusals below would pass for the wrong reason"
    probe_done
    return 0
  fi
  assert_eq "1" "$(k8s_term_nc "$K8S_NS" "$ip")" \
    "a pod in the release's own namespace that does not carry the CDR's labels is not a caller the policy names"
  assert_eq "1" "$(k8s_term_nc default "$ip")" \
    "the policy's podSelector is namespace-scoped, so another namespace must be refused too"
  # The admitted half, observed from the CDR's own pod rather than inferred: the
  # CDR's read-only terminology route resolves the code THROUGH the Service, so
  # a 200 here is a packet that left the CDR pod and arrived.
  local enc code
  enc="$(printf '%s' "$K8S_TERM_SYSTEM" | jq -sRr @uri)"
  code="$(http_code -u "$K8S_BASIC" "$K8S_API/terminology/$enc/term/$K8S_TERM_CODE")"
  assert_eq "200" "$code" \
    "a policy that also refuses the caller it was written for is a broken recipe, not a hardened one"
  probe_done
}

# The claim an operator actually buys the workload for, mirroring P-FT-RESOLVE
# on the compose side: a coded value the seed knows commits, and one the value
# set excludes is refused — which is only answerable if the binding was resolved
# through FerroTERM.
probes_k8s_term_resolve() {
  probe "P-K8S-TERM-RESOLVE" "working" "server" "#3328" \
    "the CDR resolves a value-set binding through the terminology Service: a member commits, a non-member is refused"
  if ! k8s_term_template; then
    probe_fail "the binding template accepted by the deployed CDR" \
      "the OPT upload did not answer 201, 204 or 409" \
      "every commit below is against that template; without it the outcomes mean nothing"
    probe_done
    return 0
  fi
  assert_eq "201" "$(k8s_term_commit_code "$K8S_TERM_MEMBER")" \
    "1000002 is in sct-shaped-disorders, so the commit is accepted"
  assert_eq "422" "$(k8s_term_commit_code "$K8S_TERM_NON_MEMBER")" \
    "1000003 is in the code system but not in the value set, so the binding refuses it"
  probe_done
}

# The index branch: a built index for a licensed release, mounted read-only from
# an EXISTING claim. Three things about it are unobserved until someone runs it,
# and two of them have no documented answer at all — so this is written to be
# run, and declared unexercised by name when there is no claim to run it against.
probes_k8s_term_index() {
  if [[ -z "$K8S_TERM_INDEX_CLAIM" ]]; then
    uncovered "P-K8S-TERM-INDEX-STRATEGY (a named index claim renders strategy: Recreate)" \
      "no claim was named; set K8S_TERM_INDEX_CLAIM to an existing PersistentVolumeClaim holding an index built off-cluster with ferroterm-build"
    uncovered "P-K8S-TERM-INDEX-MOUNT (the read-only index mount under fsGroup 65532 and fsGroupChangePolicy OnRootMismatch)" \
      "no claim was named, and the Kubernetes security-context documentation does not say what fsGroup does to a volume mounted readOnly, so only a run can answer it"
    uncovered "P-K8S-TERM-INDEX-READY (whether GET /health answers 200 before a mounted index has finished opening)" \
      "no claim was named; the seed path settles nothing about it, because the seed is a few megabytes read from a local volume and an edition is not"
    return 0
  fi

  probe "P-K8S-TERM-INDEX-STRATEGY" "working" "chart" "#3328" \
    "a named index claim makes the Deployment Recreate, so a ReadWriteOnce volume cannot wedge the rollout"
  if ! k8s_term_install --set "terminology.index.persistentVolumeClaim=$K8S_TERM_INDEX_CLAIM"; then
    probe_fail "an install with an index claim named" "helm refused the release"
    probe_done
    uncovered "every index probe after P-K8S-TERM-INDEX-STRATEGY" \
      "the install with the claim was refused, so the mounted index was never observed"
    return 0
  fi
  assert_eq "Recreate" "$(kc get "deploy/$K8S_TERM" -o jsonpath='{.spec.strategy.type}' 2>/dev/null)" \
    "a rolling update surges a second pod, and a ReadWriteOnce claim cannot attach to it on another node"
  local rolled=0
  k8s_term_rollout 420s && rolled=1
  probe_done

  if [[ "$rolled" -eq 0 ]]; then
    uncovered "P-K8S-TERM-INDEX-MOUNT and P-K8S-TERM-INDEX-READY" \
      "the pod carrying the claim never became ready, so neither the mount nor the readiness ordering could be read: $(kc get pod -l "$K8S_TERM_LABEL" -o jsonpath='{.items[0].status.containerStatuses[0].state.waiting.reason}' 2>/dev/null)"
    return 0
  fi

  probe "P-K8S-TERM-INDEX-MOUNT" "working" "chart" "#3328" \
    "the index arrives read-only under the requested fsGroup, and what the kubelet did to its ownership is recorded"
  local pod fsg fspol
  pod="$(kc get pod -l "$K8S_TERM_LABEL" -o jsonpath='{.items[0].metadata.name}' 2>/dev/null)"
  fsg="$(kc get pod "$pod" -o jsonpath='{.spec.securityContext.fsGroup}' 2>/dev/null)"
  fspol="$(kc get pod "$pod" -o jsonpath='{.spec.securityContext.fsGroupChangePolicy}' 2>/dev/null)"
  assert_eq "65532" "$fsg" \
    "without the group the process runs as, a volume whose files are owned by anyone else is unreadable"
  assert_eq "OnRootMismatch" "$fspol" \
    "Always recurses the whole volume at every start, which on an edition-sized index is a boot that looks wedged"
  local node cid spec mount
  node="$(k8s_node_container)"
  if [[ -n "$node" ]]; then
    cid="$(docker exec "$node" crictl ps --name terminology -q 2>/dev/null | head -1)"
    spec="$(docker exec "$node" crictl inspect "$cid" 2>/dev/null | jq -c '.info.runtimeSpec')"
    mount="$(printf '%s' "$spec" | jq -r '.mounts[] | select(.destination == "/data/index")')"
    assert_contains "$mount" '"ro"' \
      "an index a compromised container can rewrite is a terminology answer an attacker chooses"
    # Ownership is OBSERVED, not asserted: no Kubernetes documentation states
    # what fsGroup does to a volume mounted readOnly, which is the open question
    # this probe exists to answer for whoever reads the record next.
    dim "    observed: $(docker exec "$node" sh -c "ls -ldn $(printf '%s' "$mount" | jq -r '.source')" 2>&1 | head -1)"
  fi
  probe_done

  # The ordering question, answered from the log rather than from a stopwatch:
  # if the line reporting the listener follows the line reporting the index,
  # /health cannot have answered before the index was open.
  probe "P-K8S-TERM-INDEX-READY" "working" "server" "#3328" \
    "the health route starts answering only after the mounted index has finished opening"
  local logs idx_line listen_line
  logs="$(kc logs -l "$K8S_TERM_LABEL" --tail=200 --all-containers 2>/dev/null)"
  idx_line="$(printf '%s\n' "$logs" | grep -n -i 'index' | head -1 | cut -d: -f1)"
  listen_line="$(printf '%s\n' "$logs" | grep -n -i 'listening' | head -1 | cut -d: -f1)"
  if [[ -z "$idx_line" ]] || [[ -z "$listen_line" ]]; then
    probe_fail "a boot log naming the index and the listener" \
      "$(printf '%s\n' "$logs" | tail -5)" \
      "the ordering is what settles whether a startup probe can pass while the index is still opening"
  elif [[ "$idx_line" -ge "$listen_line" ]]; then
    probe_fail "the index reported open BEFORE the listener starts" \
      "index at line $idx_line, listener at line $listen_line" \
      "the startup probe then passes on a server that cannot yet answer a lookup; the 5s x 60 budget is what stands between that and a restart loop"
  fi
  probe_done
}

# The Helm half of the book's reset procedure (website/book/src/operations-reset.md
# § Kubernetes and Helm), run as written: uninstall the release, drop and
# recreate the database, install again with only the values every deployment
# supplies. CRA Annex I Part I(2)(b) asks for "the possibility to reset the
# product to its original state" (docs/law/eu/cra/text.html). Destructive: it
# removes the release and the database, so it runs last.
probes_k8s_original_state() {
  bold "returning to the original state (the book's Helm reset)"
  local db="${PG_INIT_DB:-ferroehr}" owner="${PG_INIT_USER:-ferroehr}" hdr ehr="" out
  # The book's step 2 runs as "the bootstrap superuser or the database owner";
  # the owner here holds no CREATEDB, so the bootstrap superuser it is.
  local superuser="${POSTGRES_USER:-postgres}"

  k8s_pf_start || true
  hdr="$(curl -s -u "$K8S_BASIC" -X POST -D - -o /dev/null "$K8S_API/ehr" || true)"
  ehr="$(printf '%s' "$hdr" | grep -i '^location' | tr -d '\r' | awk -F/ '{print $NF}')"
  k8s_pf_stop
  if [[ -z "$ehr" ]]; then
    uncovered "the Helm reset procedure" \
      "no EHR could be written before the reset, so nothing would show the reset removed it"
    return
  fi

  probe "P-K8S-RESET-UNINSTALL" "working" "chart" "#3677" \
    "helm uninstall removes the release and keeps the DSN Secret the operator created"
  helm uninstall "$K8S_RELEASE" -n "$K8S_NS" --wait >/dev/null 2>&1 \
    || probe_fail "a clean helm uninstall" "helm uninstall failed"
  [[ -z "$(kc get deploy "$K8S_RELEASE" -o name 2>/dev/null)" ]] \
    || probe_fail "no Deployment after the uninstall" "deploy/$K8S_RELEASE still exists"
  [[ -n "$(kc get secret ferroehr-db -o name 2>/dev/null)" ]] \
    || probe_fail "the operator's DSN Secret kept" "secret/ferroehr-db is gone" \
      "the book's step 1 says the Secrets you created survive the uninstall"
  probe_done

  probe "P-K8S-RESET-DATABASE" "working" "database" "#3677" \
    "the database is dropped and recreated as the book's step 2 writes it"
  out="$(docker compose -p "$COMPOSE_PROJECT" exec -T ferroehr-postgres \
    psql -v ON_ERROR_STOP=1 -U "$superuser" -d postgres \
    -c "DROP DATABASE \"$db\" WITH (FORCE);" \
    -c "CREATE DATABASE \"$db\" OWNER \"$owner\";" 2>&1)" \
    || probe_fail "DROP DATABASE … WITH (FORCE) and CREATE DATABASE succeed" "$out"
  probe_done

  probe "P-K8S-RESET-REINSTALL" "working" "chart" "#3677" \
    "a reinstall with the minimal values migrates the empty database and serves none of the old data"
  if k8s_install && k8s_rollout 240s && k8s_pf_start; then
    assert_eq "404" "$(curl -s -u "$K8S_BASIC" -o /dev/null -w '%{http_code}' "$K8S_API/ehr/$ehr")" \
      "GET /ehr/$ehr must find nothing after the reset"
    assert_eq "200" "$(curl -s -o /dev/null -w '%{http_code}' "$K8S_CDR/health/readiness")" \
      "the reinstalled release is ready on the recreated database"
  else
    probe_fail "a ready release after the reinstall" "rollout or port-forward failed" \
      "$(kc logs -l app.kubernetes.io/name=ferroehr --tail=6 --all-containers 2>&1 | tail -6 | tr '\n' ' ')"
  fi
  k8s_pf_stop
  probe_done
}
