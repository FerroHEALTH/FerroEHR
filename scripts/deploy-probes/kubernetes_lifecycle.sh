#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The lifecycle family of the Kubernetes probe (#6744): what an operator does
# to a running release, and whether the records survive it.
#
#   1. Two replicas boot together on an empty database: the migrations are
#      applied once (sqlx holds a PostgreSQL advisory lock while it migrates),
#      no pod restarts, and `ferroehr db verify` finds exactly this build's set.
#   2. The openEHR round trip: a template, an EHR, a COMPOSITION, and an AQL
#      query that returns it.
#   3. Every server pod deleted at once: the records read back afterwards.
#   4. `kubectl rollout restart` while a client inside the cluster commits:
#      every commit is answered 201 and every one is stored.
#   5. `helm upgrade` with a changed value, and PROBE_K8S_UPGRADE_IMAGE as the
#      new image when it is set: the release rolls, the records stay, and the
#      schema still verifies.
#
# The database is the host one the release was installed against, so a pod
# restart cannot empty it. Sourced by scripts/deploy-probe-k8s.sh after
# kubernetes.sh; never run directly. No openEHR specification governs
# deployment: our own design.

K8S_LIFE_TEMPLATE="corpus/templates/minimal_event.opt"
K8S_LIFE_COMPOSITION="corpus/fixtures/composition/minimal_event.v1.json"
K8S_LIFE_WRITES="${PROBE_K8S_LIFE_WRITES:-60}"
K8S_LIFE_WRITER="probe-writer"
K8S_LIFE_FIXTURE="probe-writer-fixture"
K8S_LIFE_CURL_IMAGE="docker.io/curlimages/curl:8.17.0@sha256:935d9100e9ba842cdb060de42472c7ca90cfe9a7c96e4dacb55e79e560b3ff40"
K8S_LIFE_UPGRADE_IMAGE="${PROBE_K8S_UPGRADE_IMAGE:-}"
K8S_LIFE_EHR=""
K8S_LIFE_UID=""

k8s_life_server_pods() {
  kc get pod -l "app.kubernetes.io/name=ferroehr,app.kubernetes.io/instance=${K8S_RELEASE}" \
    -o jsonpath='{range .items[*]}{.metadata.name}{" "}{.status.containerStatuses[0].restartCount}{"\n"}{end}' 2>/dev/null
}

# `ferroehr db verify` in a server pod, on the pod's own configuration: exit 0
# only when the database carries exactly this build's migrations.
k8s_life_verify() {
  kc exec "deploy/$K8S_RELEASE" -c ferroehr -- /usr/local/bin/ferroehr db verify 2>&1
}

# The number of COMPOSITIONs in the probe's EHR, read over AQL.
k8s_life_count() {
  curl -s -u "$K8S_BASIC" -H 'Content-Type: application/json' -X POST \
    -d "{\"q\":\"SELECT COUNT(c/uid/value) AS n FROM EHR e CONTAINS COMPOSITION c WHERE e/ehr_id/value = '$K8S_LIFE_EHR'\"}" \
    "$K8S_API/query/aql" | jq -r '.rows[0][0] // empty' 2>/dev/null
}

# A rebuilt forward after the pods behind the Service were replaced.
k8s_life_reforward() {
  k8s_pf_stop
  k8s_pf_start >/dev/null 2>&1
}

probes_k8s_lifecycle() {
  bold "the release's lifecycle: migrations, round trip, restarts and upgrade"

  probe "P-K8S-LIFE-MIGRATE-ONCE" "$PROBE_WORKING" "server" "-" \
    "replicas booting together on an empty database apply the migrations once, none restarts, and db verify passes"
  local pods restarts verify
  pods="$(k8s_life_server_pods)"
  assert_eq "2" "$(printf '%s\n' "$pods" | grep -c .)" \
    "the chart's default two replicas are what race for the lock"
  restarts="$(printf '%s\n' "$pods" | awk '$2 != 0 { print $1 }')"
  if [[ -n "$restarts" ]]; then
    local pod
    for pod in $restarts; do
      probe_fail "no restart of $pod" \
        "$(kc logs "$pod" --previous 2>&1 | grep -E '"level":"(ERROR|WARN)"|^ferroehr:' | tail -2)" \
        "a replica that lost a migration race restarts; with the lock held, the second one waits and starts"
    done
  fi
  if verify="$(k8s_life_verify)"; then
    :
  else
    probe_fail "ferroehr db verify exiting 0" "${verify:0:300}" \
      "the bookkeeping must hold exactly this build's migrations, each once"
  fi
  probe_done

  probe "P-K8S-LIFE-ROUNDTRIP" "$PROBE_WORKING" "server" "-" \
    "a template, an EHR, a COMPOSITION, and an AQL query that returns it"
  local code headers aql
  code="$(curl -s -u "$K8S_BASIC" -o /dev/null -w "$K8S_HTTP_CODE" -X POST \
    -H 'Content-Type: application/xml' --data-binary "@$K8S_LIFE_TEMPLATE" \
    "$K8S_API/definition/template/adl1.4")"
  case "$code" in
    201 | 204 | 409) ;;
    *) probe_fail "the template stored (201, 204 or 409)" "$code" ;;
  esac
  headers="$(curl -s -u "$K8S_BASIC" -X POST -D - -o /dev/null "$K8S_API/ehr")"
  K8S_LIFE_EHR="$(printf '%s' "$headers" | grep -i '^location' | tr -d '\r' | awk -F/ '{print $NF}')"
  if [[ -z "$K8S_LIFE_EHR" ]]; then
    probe_fail "a Location header naming the new EHR" "$(printf '%s' "$headers" | sed -n 1p)"
    probe_done
    uncovered "P-K8S-LIFE-RESTART, P-K8S-LIFE-ROLLING and P-K8S-LIFE-UPGRADE" \
      "no EHR was created, so there were no records to keep"
    return 0
  fi
  headers="$(curl -s -u "$K8S_BASIC" -X POST -D - -o /dev/null \
    -H 'Content-Type: application/json' --data-binary "@$K8S_LIFE_COMPOSITION" \
    "$K8S_API/ehr/$K8S_LIFE_EHR/composition")"
  assert_contains "$(printf '%s' "$headers" | sed -n 1p)" "201" \
    "the COMPOSITION must commit on the deployed server"
  K8S_LIFE_UID="$(printf '%s' "$headers" | grep -i '^etag' | tr -d '\r"' | awk '{print $2}' | sed 's#^W/##')"
  aql="$(curl -s -u "$K8S_BASIC" -H 'Content-Type: application/json' -X POST \
    -d "{\"q\":\"SELECT c/uid/value AS uid FROM EHR e CONTAINS COMPOSITION c WHERE e/ehr_id/value = '$K8S_LIFE_EHR'\"}" \
    "$K8S_API/query/aql")"
  assert_contains "$aql" "${K8S_LIFE_UID:-<no ETag>}" \
    "the AQL result must carry the version just committed"
  probe_done

  probe "P-K8S-LIFE-RESTART" "$PROBE_WORKING" "server" "-" \
    "every server pod deleted at once: the records read back from the pods that replace them"
  local before after
  before="$(k8s_life_count)"
  kc delete pod -l "app.kubernetes.io/name=ferroehr,app.kubernetes.io/instance=${K8S_RELEASE}" \
    --wait=true >/dev/null 2>&1
  if ! k8s_rollout 240s; then
    probe_fail "the replacement pods rolled out" "no rollout within 240s"
  fi
  k8s_life_reforward
  after="$(k8s_life_count)"
  assert_eq "$before" "$after" "a pod restart must not lose a COMPOSITION"
  assert_eq "200" "$(http_code -u "$K8S_BASIC" "$K8S_API/ehr/$K8S_LIFE_EHR/composition/$K8S_LIFE_UID")" \
    "the version committed before the restart must read back"
  probe_done

  probe "P-K8S-LIFE-ROLLING" "$PROBE_WORKING" "chart" "-" \
    "a rolling restart while a client commits: every commit is answered 201 and every one is stored"
  before="$(k8s_life_count)"
  kc delete pod "$K8S_LIFE_WRITER" --ignore-not-found >/dev/null 2>&1
  kc delete configmap "$K8S_LIFE_FIXTURE" --ignore-not-found >/dev/null 2>&1
  kc create configmap "$K8S_LIFE_FIXTURE" --from-file="c.json=$K8S_LIFE_COMPOSITION" >/dev/null 2>&1
  kc apply -f - >/dev/null 2>&1 <<YAML
apiVersion: v1
kind: Pod
metadata:
  name: $K8S_LIFE_WRITER
spec:
  restartPolicy: Never
  automountServiceAccountToken: false
  securityContext:
    runAsNonRoot: true
    runAsUser: 65532
    seccompProfile: { type: RuntimeDefault }
  containers:
    - name: writer
      image: $K8S_LIFE_CURL_IMAGE
      securityContext:
        allowPrivilegeEscalation: false
        capabilities: { drop: [ALL] }
      command: ["sh", "-c"]
      args:
        - |
          ok=0; failed=0; i=0
          while [ "\$i" -lt $K8S_LIFE_WRITES ]; do
            i=\$((i + 1))
            code=\$(curl -s -o /dev/null -w '%{http_code}' --max-time 15 -u '$K8S_BASIC' \
              -H 'Content-Type: application/json' --data-binary @/fixture/c.json \
              http://$K8S_RELEASE:8080/ferroehr/rest/openehr/v1/ehr/$K8S_LIFE_EHR/composition)
            if [ "\$code" = 201 ]; then ok=\$((ok + 1)); else failed=\$((failed + 1)); echo "commit \$i answered \$code"; fi
            sleep 0.5
          done
          echo "RESULT ok=\$ok failed=\$failed"
      volumeMounts: [{ name: fixture, mountPath: /fixture, readOnly: true }]
  volumes: [{ name: fixture, configMap: { name: $K8S_LIFE_FIXTURE } }]
YAML
  if ! kc wait --for=condition=Ready "pod/$K8S_LIFE_WRITER" --timeout=120s >/dev/null 2>&1; then
    probe_fail "the in-cluster client running" "$(kc get pod "$K8S_LIFE_WRITER" -o jsonpath="$K8S_PHASE_PATH" 2>&1)"
    probe_done
  else
    sleep 3
    kc rollout restart "deploy/$K8S_RELEASE" >/dev/null 2>&1
    k8s_rollout 300s || probe_fail "the rolling restart completed" "no rollout within 300s"
    kc wait --for=jsonpath='{.status.phase}'=Succeeded "pod/$K8S_LIFE_WRITER" --timeout=300s >/dev/null 2>&1
    local result
    result="$(kc logs "$K8S_LIFE_WRITER" 2>&1 | tail -5)"
    assert_contains "$result" "RESULT ok=$K8S_LIFE_WRITES failed=0" \
      "a rolling restart must answer every commit: the PodDisruptionBudget, the preStop sleep and readiness keep a ready pod behind the Service"
    k8s_life_reforward
    after="$(k8s_life_count)"
    assert_eq "$((before + K8S_LIFE_WRITES))" "$after" \
      "every commit answered 201 must be stored"
    probe_done
  fi
  kc delete pod "$K8S_LIFE_WRITER" --ignore-not-found --wait=false >/dev/null 2>&1
  kc delete configmap "$K8S_LIFE_FIXTURE" --ignore-not-found >/dev/null 2>&1

  probe "P-K8S-LIFE-UPGRADE" "$PROBE_WORKING" "chart" "-" \
    "helm upgrade with a changed value rolls the release, keeps the records, and the schema still verifies"
  local revision_before revision_after
  revision_before="$(helm history "$K8S_RELEASE" -n "$K8S_NS" --max 1 -o json 2>/dev/null | jq -r '.[0].revision // empty')"
  before="$(k8s_life_count)"
  local -a upgrade=(--set-string "podAnnotations.probe\.ferroehr\.eu/upgrade=$(date +%s)")
  if [[ -n "$K8S_LIFE_UPGRADE_IMAGE" ]]; then
    upgrade+=(--set image.repository="${K8S_LIFE_UPGRADE_IMAGE%:*}" --set image.tag="${K8S_LIFE_UPGRADE_IMAGE##*:}")
  fi
  if ! k8s_install "${upgrade[@]}"; then
    probe_fail "a successful helm upgrade" "$K8S_HELM_REFUSED"
  elif ! k8s_rollout 300s; then
    probe_fail "the upgraded release rolled out" "no rollout within 300s"
  fi
  revision_after="$(helm history "$K8S_RELEASE" -n "$K8S_NS" --max 1 -o json 2>/dev/null | jq -r '.[0].revision // empty')"
  [[ -n "$revision_before" && -n "$revision_after" && "$revision_after" -gt "$revision_before" ]] ||
    probe_fail "a new release revision" "before ${revision_before:-none}, after ${revision_after:-none}"
  k8s_life_reforward
  after="$(k8s_life_count)"
  assert_eq "$before" "$after" "an upgrade must keep every COMPOSITION"
  if verify="$(k8s_life_verify)"; then
    :
  else
    probe_fail "ferroehr db verify exiting 0 after the upgrade" "${verify:0:300}"
  fi
  probe_done
  if [[ -z "$K8S_LIFE_UPGRADE_IMAGE" ]]; then
    uncovered "P-K8S-LIFE-UPGRADE to a second image" \
      "PROBE_K8S_UPGRADE_IMAGE was not set, so the upgrade changed a value and kept the image"
  fi
  # Back to the shipped values for the families that follow.
  k8s_install
  k8s_rollout 300s
  k8s_life_reforward
}
