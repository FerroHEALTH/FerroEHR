---
name: k8s-test
description: >
  Runs the ferroehr Helm chart on a real single-node Kubernetes cluster (Docker
  Desktop's) and verifies what only a live run can: that the pods start under the
  hardened security context, migrate the database, pass their probes, serve an
  openEHR request, load-balance across replicas, autoscale, export metrics and
  traces, upgrade and uninstall cleanly. Use when the user asks to deploy or test
  the chart, to check the Kubernetes/Helm path, before changing
  deploy/helm/**, or when a chart claim needs evidence from a running pod rather
  than from `helm template`.
allowed-tools: [Bash, Read, Edit, Write, Grep, Glob]
argument-hint: "[--namespace <ns>] [--tag <image-tag>] [--observability]"
---

# /k8s-test

`helm template` proves the YAML we generate and nothing else. This skill proves
the deployment: pods that start under `runAsNonRoot` + `readOnlyRootFilesystem`,
migrations that apply idempotently, probes that gate rather than pass early, and
a served openEHR call. Every command below was run against Docker Desktop
Kubernetes v1.36.1 (kubectl 1.36.2, helm 4.1.3) and its output is the real
output.

## Ground rules

- **Never relax a security setting to make something start.** If the read-only
  root filesystem, the dropped capabilities or the non-root uid prevent a boot,
  that is the finding — the fix is making the software work under the setting.
- **The chart ships no database on purpose** (a CDR's PostgreSQL must be
  externally managed, backed up, PITR-capable), so a test has to supply one.
  `postgres.yaml` beside this file is that supply and is explicitly throwaway.
- **Fix findings in the chart, not in the test.** Bending the values overlay
  until it works hides exactly what this exercise exists to surface.
- Chart behaviour changes regenerate the goldens: `deploy/helm/validate.sh --update`.
- **The viewer (`viewer.enabled`) is covered by the automated harness**
  (`scripts/deploy-probe-k8s.sh`, the `P-K8S-UI-*` family: off/boot/runtime
  posture/PSA admission/served login page/narrowed-ingress install). What
  remains manual-only for the viewer: its OIDC sign-in path, screens behind a
  session, and the viewer→CDR REST hop (a served login page is not evidence of
  it — the login form renders even when the CDR is unreachable).

## Four facts people get wrong

1. **The cluster does NOT share the Docker image store.** Docker Desktop's
   Kubernetes runs a kind-style node (`desktop-control-plane`) with its own
   containerd. A locally built image is invisible to it, and **the symptom depends
   on the pull policy** — measured 2026-08-06 by building a local-only image and
   deploying it:

   - `imagePullPolicy: IfNotPresent` (or `Always`) → **`ErrImagePull`**, because
     the kubelet does not find it locally and falls back to the registry:
     `failed to resolve reference "docker.io/library/<image>": pull access denied,
     repository does not exist`. This is the case you will actually hit, since it
     is the chart's default policy.
   - `imagePullPolicy: Never` → `ErrImageNeverPull`, the kubelet refusing to look
     at a registry at all.

   Either way the fix is the same: import it into the node's containerd (step 0b).
   Published images pull normally.
2. **Service link env variables break this server.** The kubelet injects
   `<SVC>_SERVICE_HOST`, `<SVC>_PORT_8080_TCP_ADDR` … for every Service in the
   namespace. For a Service named `ferroehr*` those land inside the server's
   reserved `FERROEHR_` namespace and its strict env sweep refuses to boot. The
   chart therefore pins `enableServiceLinks: false`, and
   `deploy/helm/validate.sh` asserts it.
3. **The chart's `config:` defaults can be ahead of `appVersion`'s image.** Chart
   values track the main tree; `appVersion` names the last published release.
   A key added since that release makes the pod exit with
   `unknown configuration key …`. See the troubleshooting entry — it is the most
   likely failure of a first run. Check the pairing without a cluster, using the
   image as the authority:

   ```bash
   helm template ferroehr deploy/helm/ferroehr -s templates/configmap.yaml \
     -f deploy/helm/ci/default-values.yaml \
     | sed -n '/ferroehr.toml/,$p' | sed '1d;s/^    //' > /tmp/rendered.toml
   docker run --rm -v /tmp/rendered.toml:/etc/ferroehr/ferroehr.toml:ro \
     -e FERROEHR__DB__URL=postgres://u:p@db:5432/ferroehr \
     --entrypoint /usr/local/bin/ferroehr ghcr.io/ferrohealth/ferroehr:3.17.3 config check
   # 3.17.3, 2026-08-06: `unknown configuration key statement_timeout_ms` (default
   # overlay) and `limits` (all-features) — i.e. the chart on main currently
   # cannot deploy its own appVersion without the version-skew nulls below.
   ```

4. **A secret in `config:` is refused, not rendered.** The chart classifies every
   key it renders: one with a `secrets:` route (the DSN, the AMQP URLs, a Basic
   user's hash, the OIDC HMAC secret, a signing passphrase, a terminology client
   secret) **fails the render** naming the key that carries it. One with no route
   at all would move the whole `ferroehr.toml` into a Secret and render no
   ConfigMap — but no key reaches that branch today, so a normal run has a
   ConfigMap. If `helm template` refuses with "refusing to render a secret into
   the ConfigMap", move the value to `secrets:` as the message says.

## 0. Cluster

```bash
kubectl config current-context      # docker-desktop
kubectl get nodes                   # desktop-control-plane   Ready   control-plane
helm version --short                # v4.1.3+gc94d381
```

If there is no cluster: Docker Desktop → Settings → Kubernetes → *Enable
Kubernetes* → Apply & restart. Nothing here needs a cloud account.

## 0b. Testing a chart change that needs an UNRELEASED server

The chart often needs server support that no published image has yet — the
`*_file` configuration keys are the standing example. `appVersion` names the last
release, so `helm install` with the default tag crash-loops on
`unknown configuration key …`.

**As of 3.17.3 this is not a key or two — the published image cannot run the
chart's default posture at all.** Measured 2026-08-09 by installing the chart
with `image.tag=3.17.3` and reading each crash in turn: `migrate`, then
`password_hash_file`, then `url_file`. That last one is the DSN mechanism
itself, so nulling keys does not rescue the run — every path the chart uses to
deliver a secret is newer than the image. Budget the build; do not start by
adding nulls to `test-values.yaml`.

**And once you are on a branch build, DELETE the skew block from
`test-values.yaml`.** A null omits the key, so against an image that understands
it the run exercises the key's absence rather than the value the chart ships —
the test still passes while quietly measuring "the chart's defaults minus six
keys". That is the same shape as a gate that renders an overlay nobody enables:
green, and about the wrong thing.

Build the image from the branch and load it:

```bash
docker build -t ghcr.io/ferrohealth/ferroehr:dev-local \
  --target runtime-from-source -f docker/Dockerfile .        # ~10-15 min cold
docker save ghcr.io/ferrohealth/ferroehr:dev-local \
  | docker exec -i desktop-control-plane ctr -n k8s.io images import -
helm upgrade ... --set image.tag=dev-local --set image.pullPolicy=IfNotPresent
```

**The `ctr import` step is not optional, and `IfNotPresent` alone does not save
you.** Docker Desktop's Kubernetes runs in the `desktop-control-plane` container
with **its own containerd image store, which does not share the Docker daemon's** —
verified by building a local-only image and watching the kubelet try to pull it
from docker.io:

```text
crictl images | grep local-only-probe     → (nothing)
pod status: ErrImagePull — "failed to resolve reference docker.io/library/local-only-probe:v1:
             pull access denied, repository does not exist"
```

After the import the same pod runs and prints its marker. Check with
`docker exec desktop-control-plane crictl images | grep ferroehr` before blaming
the chart.

A locally built image is a legitimate SUT for a hardening or behaviour test. It is
**not** legitimate for a provenance claim — it carries no attestation, so anything
about signing or verification still needs a published artifact.

## 1. Static gates first (seconds, and they catch most mistakes)

```bash
helm lint deploy/helm/ferroehr
bash deploy/helm/validate.sh        # pin + secret-leak + lint + render + security + goldens
```

`validate.sh` must be green BEFORE installing. It runs in CI as the
`helm-golden` job on any PR touching `deploy/helm/**`, so a drift here is a
merge blocker rather than a note.

It refuses outright if your helm is not the version in
`deploy/helm/.tool-versions`: `helm template` output is **not byte-stable across
helm releases** (4.2.3 emits 6 blank lines in the default render that 4.1.3 does
not), so a skewed helm would report chart drift that is really a whitespace diff.
Bumping the pin and re-running `--update` in the same change is the fix.

## 2. Namespace and database

```bash
kubectl create namespace ferroehr-test
kubectl -n ferroehr-test apply -f .claude/skills/k8s-test/postgres.yaml
kubectl -n ferroehr-test wait --for=condition=Ready pod \
  -l app.kubernetes.io/name=postgres --timeout=240s
```

That manifest also creates the `ferroehr-db` Secret holding
`FERROEHR__DB__URL`, which is what `database.existingSecret` points at.

## 3. Install

```bash
helm install ferroehr deploy/helm/ferroehr -n ferroehr-test \
  -f .claude/skills/k8s-test/test-values.yaml \
  --set image.tag=3.17.3

kubectl -n ferroehr-test rollout status deploy/ferroehr --timeout=180s
# deployment "ferroehr" successfully rolled out
```

To test a locally built image instead, import it into the node's containerd
first — `docker load` on the host is not enough:

```bash
docker save ghcr.io/ferrohealth/ferroehr:local \
  | docker exec -i desktop-control-plane ctr -n k8s.io images import -
docker exec desktop-control-plane crictl images | grep ferroehr
helm install … --set image.tag=local --set image.pullPolicy=Never
```

## 4. Prove it serves

```bash
kubectl -n ferroehr-test port-forward svc/ferroehr 8080:8080 &
curl -s http://127.0.0.1:8080/health/liveness            # OK
curl -s http://127.0.0.1:8080/health/readiness           # {"status":"UP","components":{…}}
curl -s http://127.0.0.1:8080/ferroehr/rest/status       # {"status":"UP","server_version":"3.17.3",…}

# The proof that matters: a served openEHR request.
curl -s -u ferroehr:ferroehr -X POST -D /tmp/h -o /dev/null \
  -w 'POST /ehr %{http_code}\n' \
  http://127.0.0.1:8080/ferroehr/rest/openehr/v1/ehr     # 201
EID=$(grep -i '^location' /tmp/h | tr -d '\r' | awk -F/ '{print $NF}')
curl -s -u ferroehr:ferroehr \
  http://127.0.0.1:8080/ferroehr/rest/openehr/v1/ehr/$EID   # the canonical EHR
curl -s -u ferroehr:ferroehr --get \
  --data-urlencode 'q=SELECT e/ehr_id/value FROM EHR e LIMIT 3' \
  http://127.0.0.1:8080/ferroehr/rest/openehr/v1/query/aql   # RESULTSET 1.1.0
```

A `201` with an empty body is correct: without `Prefer: return=representation`
the response carries the id in `Location`/`ETag`, not a body.

`port-forward` targets ONE pod, so it can never show load balancing — use step 8
for that.

## 5. The security posture, read off the running pod

What the chart requested, as the API server stored it:

```bash
P=$(kubectl -n ferroehr-test get pod -l app.kubernetes.io/name=ferroehr \
      -o jsonpath='{.items[0].metadata.name}')
kubectl -n ferroehr-test get pod $P -o jsonpath='{.spec.securityContext}'            # fsGroup 65532, runAsNonRoot, RuntimeDefault
kubectl -n ferroehr-test get pod $P -o jsonpath='{.spec.containers[0].securityContext}'
kubectl -n ferroehr-test get pod $P -o jsonpath='{.spec.containers[0].resources}'    # requests 250m/256Mi, limits 2/1Gi
kubectl -n ferroehr-test get pod $P \
  -o jsonpath='automount={.spec.automountServiceAccountToken} links={.spec.enableServiceLinks}'
```

What the runtime actually applied — the `docker inspect` equivalent, and the one
that settles arguments:

```bash
CID=$(docker exec desktop-control-plane crictl ps --name ferroehr -q | head -1)
docker exec desktop-control-plane crictl inspect $CID | jq '
  .info.runtimeSpec
  | { user: .process.user, noNewPrivileges: .process.noNewPrivileges,
      bounding: .process.capabilities.bounding, readonly: .root.readonly,
      seccomp: .linux.seccomp.defaultAction }'
```

Correct output is `{"additionalGids":[65532],"gid":65532,"uid":65532}`, `true`,
`[]`, `true`, `SCMP_ACT_ERRNO` — uid/gid 65532, no-new-privileges, an EMPTY bounding
capability set (not "the default 14 minus ours"), a read-only root, and a seccomp
filter that denies by default. The same inspect output lists the mounts: only
`/tmp` (the chart's `emptyDir`) and the kubelet's own `/dev`, `/etc/hosts`,
`/dev/termination-log` are writable; `/etc/ferroehr` and
`/etc/ferroehr-secrets` are `ro`.

Where each secret actually lives, and the proof none of them is in the ConfigMap
or the environment (`test-values.yaml` configures a Basic user and an external-Secret
DSN, so this run exercises the mounted path):

```bash
kubectl -n ferroehr-test get configmap,secret
kubectl -n ferroehr-test get configmap -o yaml | grep -c argon2id     # 0
# the DSN is a PATH in the env, never a value:
kubectl -n ferroehr-test get pod $P -o jsonpath='{range .spec.containers[0].env[*]}{.name}={.value}{"\n"}{end}'
#   FERROEHR__DB__URL_FILE=/etc/ferroehr-secrets/db.url
kubectl -n ferroehr-test get pod $P -o jsonpath='{.spec.volumes}'     # projected: the operator's Secret + the chart's
```

A file-borne secret, checked on the node rather than in the template:

```bash
PU=$(kubectl -n ferroehr-test get pod $P -o jsonpath='{.metadata.uid}')
docker exec desktop-control-plane \
  ls -lnL /var/lib/kubelet/pods/$PU/volumes/kubernetes.io~secret/secrets/
# -r--r----- 1 0 65532 48 … auth.oidc.hmac_secret
```

Owner root, group `fsGroup`, mode 0440 — the process reads it through the group
bit. A secret that is mounted but never read looks identical in
`kubectl describe`, so prove the read: point a `*_file` key at a path that does
not exist and watch the boot refuse.

```bash
helm upgrade … --set config.signing.key_passphrase_file=/etc/ferroehr-secrets/absent
kubectl -n ferroehr-test logs <new-pod>
# Error: 1 configuration error(s):
#   - reading secret file /etc/ferroehr-secrets/absent: No such file or directory (os error 2)
```

No secret VALUE may appear in `kubectl describe pod` — env entries read
`<set to the key … in secret …>` and file-borne ones show only their path.

## 6. Probes, and the readiness/liveness split

```bash
kubectl -n ferroehr-test describe pod $P | sed -n '/Liveness:/,/Environment:/p'
kubectl -n ferroehr-test describe pod $P | sed -n '/Events:/,$p'   # probe failures show here
```

`describe` shows probe failures even for a pod that eventually starts, so read
the Events block every time. Then prove readiness GATES rather than passing
early, by taking the database away:

```bash
kubectl -n ferroehr-test scale deploy/ferroehr-postgres --replicas=0
# within ~35s (failureThreshold 3 × period 10s):
#   Warning Unhealthy … Readiness probe failed: HTTP probe failed with statuscode: 503
kubectl -n ferroehr-test get pod $P \
  -o jsonpath='ready={.status.containerStatuses[0].ready} restarts={.status.containerStatuses[0].restartCount}'
# ready=false restarts=0        ← readiness fails, liveness does NOT restart it
kubectl -n ferroehr-test get endpointslice -l kubernetes.io/service-name=ferroehr \
  -o jsonpath='{.items[0].endpoints[0].conditions}'
# {"ready":false,"serving":false}   ← removed from the Service
kubectl -n ferroehr-test scale deploy/ferroehr-postgres --replicas=1
```

Then the boot-only migration behaviour, which is the one that reads as a mystery
in production. Take the schema away and watch what recovers on its own:

```bash
kubectl -n ferroehr-test exec deploy/ferroehr-postgres -- \
  psql -U ferroehr -d ferroehr -c 'DROP SCHEMA ehr CASCADE'
curl -s http://127.0.0.1:8080/health/readiness
# {"status":"DOWN","components":{"db":{"status":"UP"},
#  "migrations":{"status":"DOWN","detail":"core schema tables missing (migrations not applied)"}}}
kubectl -n ferroehr-test get pods -l app.kubernetes.io/name=ferroehr \
  -o custom-columns='NAME:.metadata.name,READY:.status.containerStatuses[0].ready,RESTARTS:.status.containerStatuses[0].restartCount'
# both READY false, RESTARTS unchanged — readiness gates, liveness does NOT restart

kubectl -n ferroehr-test delete pod <one-of-them>   # its replacement migrates at boot
# the pod that was NEVER restarted returns to READY true with RESTARTS 0 and its
# original start time: the readiness check re-queries the schema every probe, so a
# running instance recovers as soon as ANYTHING migrates. Only the case where the
# instance itself is the only migrator needs `rollout restart`.
```

**A partial wipe does not recover at all**, and it is worth knowing before you
try it as a shortcut: `DROP SCHEMA ehr CASCADE` leaves `cold`, `audit` and `ext`,
and migration 7 then dies on the surviving `cold.vo_version`
(`relation "vo_version" already exists`) with `_sqlx_migrations` stuck at 6 —
`CREATE SCHEMA IF NOT EXISTS cold` is guarded, the `CREATE TABLE cold.*` under it
is not. Every restart retries the same failure. Recreate the whole database
(`DROP DATABASE`/re-apply `postgres.yaml`) rather than one schema.

Migration idempotency, in two installs against the same database:

```bash
kubectl -n ferroehr-test exec deploy/ferroehr-postgres -- \
  psql -U ferroehr -d ferroehr -tAc 'select count(*), max(version) from ehr._sqlx_migrations'
# 7|7  after the first install (30 tables in the ehr schema), and still 7|7 after a
# second install over the existing schema
```

## 7. Is NetworkPolicy actually enforced here?

"The policy exists" is not the question. Prove it with a connection that must be
refused:

```bash
kubectl -n ferroehr-test run nc1 --image=busybox:1.37 --restart=Never --rm --attach --quiet \
  --command -- sh -c 'nc -w3 -z ferroehr-postgres 5432; echo exit=$?'   # exit=0

kubectl -n ferroehr-test apply -f - <<'EOF'
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata: {name: deny-all-to-postgres}
spec:
  podSelector: {matchLabels: {app.kubernetes.io/name: postgres}}
  policyTypes: [Ingress]
  ingress: []
EOF
kubectl -n ferroehr-test run nc2 --image=busybox:1.37 --restart=Never --rm --attach --quiet \
  --command -- sh -c 'nc -w4 -z ferroehr-postgres 5432; echo exit=$?'   # exit=1  ⇒ ENFORCED
kubectl -n ferroehr-test delete networkpolicy deny-all-to-postgres
```

Docker Desktop's CNI is kindnetd, and the build tested (`v20260528`) DOES enforce
NetworkPolicy. Do not assume that of every cluster — an unenforcing CNI turns the
chart's policy into a document. Do not use `curl telnet://` as the probe: it
times out even when the port is open.

Then check what the chart's own policy admits. An ingress rule with `ports` and no
`from` admits EVERY source, so with `networkPolicy.ingressFrom` empty a pod in
another namespace reaches the API port:

```bash
PIP=$(kubectl -n ferroehr-test get pod -l app.kubernetes.io/name=ferroehr \
        -o jsonpath='{.items[0].status.podIP}')
kubectl -n default run xns --image=busybox:1.37 --restart=Never --rm --attach --quiet \
  --env=PIP=$PIP --command -- sh -c 'nc -w4 -z $PIP 8080; echo exit=$?'   # exit=0
```

Egress, when `networkPolicy.egress.enabled=true`: the default posture needs
exactly DNS (UDP/TCP 53) and PostgreSQL (TCP 5432) and nothing else. Confirm from
conntrack on the node:

```bash
docker exec desktop-control-plane sh -c "conntrack -L 2>/dev/null | grep $PIP"
# ESTABLISHED … dport=5432   (the database Service)
# udp … dport=53             (cluster DNS)
```

Every additional integration adds a target — an OTLP collector, an AMQP broker, a
terminology server, S3 — and a blocked one fails SILENTLY (step 9).

## 8. Pod Security admission, replicas, load balancing, autoscaling

Does the namespace refuse a non-compliant pod, and is our own pod compliant?

```bash
kubectl label namespace ferroehr-test pod-security.kubernetes.io/enforce=restricted --overwrite
kubectl -n ferroehr-test rollout restart deploy/ferroehr    # admitted ⇒ the chart meets Restricted
kubectl -n ferroehr-test apply -f - <<'EOF'
apiVersion: v1
kind: Pod
metadata: {name: privileged-probe}
spec:
  containers:
    - {name: bad, image: "busybox:1.37", command: ["sleep","60"], securityContext: {privileged: true}}
EOF
# Error from server (Forbidden): … violates PodSecurity "restricted:latest": privileged …
kubectl label namespace ferroehr-test pod-security.kubernetes.io/enforce-
```

Load balancing across replicas, counted rather than assumed — `port-forward`
cannot show this, and kube-proxy distributes CONNECTIONS, so count them in
conntrack:

```bash
kubectl -n ferroehr-test run lbdrive --image=curlimages/curl:8.11.1 --restart=Never \
  --command -- sh -c 'for i in $(seq 1 30); do curl -s -o /dev/null http://ferroehr:8080/ferroehr/rest/status; done'
docker exec desktop-control-plane sh -c "conntrack -L 2>/dev/null | grep 'dport=8080'" \
  | sed -E 's/.*dport=8080 src=([0-9.]+).*/\1/' | sort | uniq -c
#  38 10.244.0.43
#  35 10.244.0.44     ← both replicas served
kubectl -n ferroehr-test delete pod lbdrive
```

Horizontal autoscaling needs a metrics API, which Docker Desktop does not ship:

```bash
kubectl top nodes            # error: Metrics API not available
kubectl apply -f https://github.com/kubernetes-sigs/metrics-server/releases/latest/download/components.yaml
kubectl -n kube-system patch deploy metrics-server --type=json \
  -p='[{"op":"add","path":"/spec/template/spec/containers/0/args/-","value":"--kubelet-insecure-tls"}]'
kubectl -n kube-system rollout status deploy/metrics-server --timeout=180s
```

Then drive it:

```bash
helm upgrade … --set autoscaling.enabled=true --set autoscaling.minReplicas=2 \
               --set autoscaling.maxReplicas=4 --set autoscaling.targetCPUUtilizationPercentage=50
kubectl -n ferroehr-test create deployment loadgen --image=curlimages/curl:8.11.1 --replicas=12 \
  -- sh -c 'while true; do curl -s -o /dev/null -u ferroehr:ferroehr -X POST http://ferroehr:8080/ferroehr/rest/openehr/v1/ehr; done'
watch kubectl -n ferroehr-test get hpa ferroehr
# cpu: 0%/50% → 63%/50% → 112%/50%, replicas 2 → 3 → 4
kubectl -n ferroehr-test delete deploy loadgen
```

Read traffic barely moves the needle (30 curl loops of `/rest/status` sat at
~25% of a 250m request); writes are what load it. Scale-DOWN waits out the
5-minute downscale stabilisation window by design, so do not read a slow
shrink as a fault.

## 9. Metrics, dashboards and traces

```bash
kubectl -n ferroehr-test apply -f .claude/skills/k8s-test/observability.yaml
kubectl -n ferroehr-test wait --for=condition=available \
  deploy/prometheus deploy/grafana deploy/otel-collector --timeout=240s

helm upgrade … --set metrics.enabled=true \
               --set config.management.endpoints.prometheus=public \
               --set config.telemetry.otlp_endpoint=http://otel-collector:4317 \
               --set config.telemetry.traces_sample_ratio=1.0
```

`metrics.enabled` only adds the `prometheus.io/*` annotations; the endpoint
itself is opened by `config.management.endpoints.prometheus`. Both are needed.

```bash
kubectl -n ferroehr-test port-forward svc/prometheus 9090:9090 &
curl -s 'http://127.0.0.1:9090/api/v1/targets?state=active' \
  | jq -r '.data.activeTargets[] | "\(.labels.pod) \(.scrapeUrl) \(.health)"'
# ferroehr-…-vcqgj http://10.244.0.43:8080/management/prometheus up
# ferroehr-…-smkbz http://10.244.0.44:8080/management/prometheus up

kubectl -n ferroehr-test port-forward svc/grafana 3000:3000 &
curl -s -u admin:admin \
  'http://127.0.0.1:3000/api/datasources/proxy/uid/prom/api/v1/query?query=ferroehr_build_info'
# one series per pod, labelled version / git_sha / rm_version

kubectl -n ferroehr-test logs deploy/otel-collector | grep -A1 'Span #'
# Name : POST /ferroehr/rest/openehr/v1/ehr   with service.name: ferroehr
```

Annotation discovery works with a Prometheus that does `kubernetes_sd`, which is
what `observability.yaml` runs. An operator-managed Prometheus
(kube-prometheus-stack) ignores those annotations entirely and needs the chart's
`metrics.serviceMonitor.enabled=true` object plus the `monitoring.coreos.com`
CRDs — without the CRDs installed the install fails on an unknown kind.

## 10. Upgrade and uninstall — the paths an operator actually uses

```bash
helm upgrade ferroehr deploy/helm/ferroehr -n ferroehr-test \
  -f .claude/skills/k8s-test/test-values.yaml --set image.tag=3.17.3 --set replicaCount=2
kubectl -n ferroehr-test rollout status deploy/ferroehr --timeout=180s
helm -n ferroehr-test history ferroehr        # every revision, and its status
```

A change under `config:` rewrites the ConfigMap, whose `sha256sum` is a pod
annotation, so the pods roll — that is the mechanism, and it is worth confirming
in `rollout status` rather than assuming.

```bash
helm uninstall ferroehr -n ferroehr-test
kubectl -n ferroehr-test get deploy,svc,secret,configmap,networkpolicy,pdb,hpa,sa,servicemonitor
```

Everything the chart created must be gone: Deployment, Service, ConfigMap, the
`<release>-env` Secret, NetworkPolicy, PDB, HPA, ServiceAccount. The chart
creates no PersistentVolumeClaim at all, so there is none to orphan. What remains
is only what YOU applied — the postgres fixture, the `ferroehr-db` Secret, the
observability stack.

## 11. Teardown (leave the cluster clean)

```bash
helm uninstall ferroehr -n ferroehr-test 2>/dev/null
kubectl delete namespace ferroehr-test
# cluster-scoped leftovers from the steps above:
kubectl delete clusterrole,clusterrolebinding ferroehr-test-prometheus 2>/dev/null
kubectl delete crd servicemonitors.monitoring.coreos.com 2>/dev/null   # only if you installed it
kubectl -n kube-system delete -f https://github.com/kubernetes-sigs/metrics-server/releases/latest/download/components.yaml 2>/dev/null
pkill -f 'kubectl.*port-forward'
kubectl get ns    # ferroehr-test gone
```

Deleting the namespace does NOT remove ClusterRole/ClusterRoleBinding/CRD/APIService
objects — the observability fixture and metrics-server both leave some behind.

## Troubleshooting — the failures actually hit on the first run

| Symptom | Cause | Fix |
|---|---|---|
| `Error: 16 configuration error(s): unknown configuration environment variable FERROEHR_SERVICE_HOST` (+ `FERROEHR_PORT_8080_TCP…`, `FERROEHR_POSTGRES_…`) then `CrashLoopBackOff` | The kubelet's Service link variables collide with the reserved `FERROEHR_` prefix for any Service named `ferroehr*` | `enableServiceLinks: false` on the pod spec. The chart pins it and `validate.sh` asserts it — if you see this, someone removed it |
| `unknown configuration key spec_profile (line 1)` / `statement_timeout_ms` / `limits` / `connection` | The chart's `config:` defaults are newer than the image `appVersion` names | Diff the key sets against the image itself: `kubectl -n default run cfg --image=ghcr.io/ferrohealth/ferroehr:<tag> --restart=Never --attach --rm --quiet --command -- /usr/local/bin/ferroehr config default`. Then either use an image that has the keys, or null them for the test (`--set config.spec_profile=null`, `--set config.db.statement_timeout_ms=null`, `--set config.server.limits=null`, …). Do NOT delete the keys from `values.yaml` — they are correct for the next release |
| Every request `401` with `WWW-Authenticate: Basic realm="ferroehr"` | Chart default is `auth.enabled: true` with NO mechanism configured — it boots and refuses everything | Supply `config.auth.basic.users` (as `test-values.yaml` does) or `config.auth.oidc` |
| `Upgrade failed: … .env: duplicate entries for key [name="FERROEHR__…"]` | `extraEnv` repeated a name the chart already emits | Set the value through its own key, not `extraEnv` |
| Pods stay NotReady with `migrations` DOWN / `core schema tables missing (migrations not applied)` while `db: UP` | The database was replaced/wiped under a running pod; migrations run only at boot | `kubectl rollout restart deploy/ferroehr` (a running pod also recovers by itself once anything else migrates — step 6) |
| A replacement pod crash-loops on `relation "vo_version" already exists`, `_sqlx_migrations` stuck at 6 | Only the `ehr` schema was dropped; `cold.vo_version` survived and migration 7 creates it unguarded | Recreate the whole database, not one schema (step 6) |
| `ErrImagePull` (or `ErrImageNeverPull` with `pullPolicy: Never`) for an image `docker images` clearly lists | The cluster's containerd store is separate from the Docker daemon's — see fact 1 for which symptom appears when | `docker save … \| docker exec -i desktop-control-plane ctr -n k8s.io images import -` (step 0b) |
| Pod exits with `unknown configuration key …` | the chart's values or its `*_FILE` env outrun the pinned image | build from the branch (step 0b), or pin a newer tag. Against 3.17.3 the skew reaches `url_file` — the DSN mechanism — so nulling keys never gets you to a running pod (step 0b) |
| `kubectl get configmap <release>` returns NotFound | A Basic user (or any secret with no `*_file` route) moved the whole config into `<release>-config`, a Secret | Read the Secret; this is the mode `test-values.yaml` runs in (fact 4) |
| `golden ...: SKIPPED — running helm X, goldens are pinned to Y` | Local helm differs from `deploy/helm/.tool-versions`; renders are not byte-stable across helm releases | Install the pinned helm, or bump the pin AND `validate.sh --update` together |
| Readiness `503` with `db: DOWN, terminating connection due to administrator command` | The database went away | Expected and correct: readiness fails, liveness does not, the pod leaves the Service and is not restarted |
| OTLP traces never arrive, no error in the logs | `networkPolicy.egress.enabled=true` without a rule for the collector; the exporter fails silently | Add the collector to `networkPolicy.egress.rules` (port 4317) |
| `nc` says a port is closed that clearly works, or `curl telnet://` always times out | `curl`'s telnet handler waits for input | Probe TCP with `nc -w3 -z host port` from a `busybox` pod |
| `kubectl top` / HPA `TARGETS <unknown>` | No metrics-server on Docker Desktop | Install it with `--kubelet-insecure-tls` (step 8) |
| `helm install` fails on `no matches for kind "ServiceMonitor"` | `metrics.serviceMonitor.enabled=true` without the Prometheus Operator CRDs | Install the CRDs, or leave the flag off |
| `kubectl exec` into the ferroehr pod fails | The image is distroless — no shell | Inspect from the node (`crictl inspect`, `ls` under `/var/lib/kubelet/pods/<uid>/volumes`) or use the HTTP surface |
