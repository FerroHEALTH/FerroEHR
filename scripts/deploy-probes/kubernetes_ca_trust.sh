#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The in-cluster CA family of the Kubernetes probe (#6821): the server's own
# TLS listener, a CA that publishes no revocation service (as a cert-manager
# CA issuer's), and the viewer's CDR connection under each
# `service.tls.caRevocation` setting.
#
#   1. `required` names the CA in `[cdr.tls] trust_roots_file`, whose chains
#      connect only once a CRL or OCSP check cleared them: a Basic sign-in
#      through the viewer, which checks the credentials against the CDR, is
#      not admitted.
#   2. `none` names it in `trust_roots_without_revocation_file`: the same
#      sign-in is admitted and redirects (302 Found).
#
# The release is reinstalled without TLS afterwards, so the families after it
# measure the shipped default. Sourced by scripts/deploy-probe-k8s.sh after
# kubernetes.sh; never run directly. No openEHR specification governs
# deployment: our own design.

K8S_CA_SERVER_SECRET="ferroehr-server-tls"
K8S_CA_SECRET="ferroehr-server-ca"

# Mints the in-cluster CA and a server certificate for the release's Service
# names, valid for two days, into $PROBE_TMP/ca-trust, and stores them as the
# Secrets the values below name.
k8s_ca_mint() {
  local dir="$PROBE_TMP/ca-trust" svc="$K8S_RELEASE.$K8S_NS.svc"
  mkdir -p "$dir"
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 2 \
    -subj "/CN=ferroehr-probe-in-cluster-ca" \
    -addext "basicConstraints=critical,CA:TRUE" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" \
    -keyout "$dir/ca.key" -out "$dir/ca.crt" >/dev/null 2>&1 || return 1
  openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
    -subj "/CN=$K8S_RELEASE" -keyout "$dir/tls.key" -out "$dir/tls.csr" >/dev/null 2>&1 || return 1
  printf 'subjectAltName=DNS:%s,DNS:%s,DNS:%s.cluster.local\nextendedKeyUsage=serverAuth\nkeyUsage=critical,digitalSignature\n' \
    "$K8S_RELEASE" "$svc" "$svc" > "$dir/ext.cnf"
  openssl x509 -req -in "$dir/tls.csr" -CA "$dir/ca.crt" -CAkey "$dir/ca.key" \
    -CAcreateserial -days 2 -extfile "$dir/ext.cnf" -out "$dir/tls.crt" >/dev/null 2>&1 || return 1
  kc delete secret "$K8S_CA_SERVER_SECRET" "$K8S_CA_SECRET" --ignore-not-found >/dev/null 2>&1
  kc create secret tls "$K8S_CA_SERVER_SECRET" --cert="$dir/tls.crt" --key="$dir/tls.key" \
    >/dev/null 2>&1 || return 1
  kc create secret generic "$K8S_CA_SECRET" --from-file=ca.crt="$dir/ca.crt" >/dev/null 2>&1
}

# The values that turn the server's listener to TLS over the minted pair and
# name the CA-only Secret to the viewer, as deploy/helm/ci/server-tls-values.yaml
# does.
k8s_ca_values() {
  cat > "$PROBE_TMP/ca-trust/values.yaml" <<YAML
config:
  server:
    tls:
      certificate_file: /etc/ferroehr-tls/tls.crt
      key_file: /etc/ferroehr-tls/tls.key
extraVolumes:
  - name: server-tls
    secret:
      secretName: $K8S_CA_SERVER_SECRET
      defaultMode: 0440
extraVolumeMounts:
  - name: server-tls
    mountPath: /etc/ferroehr-tls
    readOnly: true
service:
  tls:
    caExistingSecret: $K8S_CA_SECRET
YAML
}

# A Basic sign-in posted to the viewer from a pod inside the cluster: the
# response's status line, its Location header when it has one, and whether
# the body is the viewer's form. Read from the finished pod's logs, as
# k8s_ui_get does.
k8s_ca_sign_in() {
  local name="probe-ca-$$-$RANDOM" out phase _i
  kubectl -n "$K8S_NS" run "$name" --image="$K8S_LIFE_CURL_IMAGE" --restart=Never \
    --labels="$K8S_UI_CLIENT_LABEL" --command -- curl -s -i \
    -H 'Sec-Fetch-Site: same-origin' \
    --data-urlencode "username=${K8S_BASIC%%:*}" \
    --data-urlencode "password=${K8S_BASIC#*:}" \
    --data-urlencode "next=/" \
    "http://$K8S_UI:3000/sign-in" >/dev/null 2>&1
  for _i in $(seq 1 40); do
    phase="$(kc get pod "$name" -o jsonpath="$K8S_PHASE_PATH" 2>/dev/null)"
    case "$phase" in Succeeded | Failed) break ;; *) ;; esac
    sleep 2
  done
  out="$(kc logs "$name" 2>/dev/null | tr -d '\r')"
  kc delete pod "$name" --ignore-not-found --wait=false >/dev/null 2>&1
  printf 'status=%s\n' "$(printf '%s' "$out" | sed -n '1s/^HTTP\/[0-9.]* \([0-9]*\).*/\1/p')"
  printf 'location=%s\n' "$(printf '%s' "$out" | sed -n 's/^[Ll]ocation: *//p' | sed -n 1p)"
  case "$out" in *'name="username"'*) echo "form=yes" ;; *) echo "form=no" ;; esac
}

# Installs the TLS release with the viewer and the given caRevocation, and
# waits for both workloads.
k8s_ca_install() {
  k8s_ui_install -f "$PROBE_TMP/ca-trust/values.yaml" \
    --set "service.tls.caRevocation=$1" && k8s_rollout 240s && k8s_ui_rollout
}

probes_k8s_ui_ca_trust() {
  bold "the viewer's trust in the in-cluster CA (service.tls.caRevocation)"
  if ! k8s_ca_mint; then
    uncovered "P-K8S-UI-CA-REQUIRED and P-K8S-UI-CA-NONE" \
      "openssl could not mint the in-cluster CA or the Secrets could not be stored"
    return 0
  fi
  k8s_ca_values
  local seen

  probe "P-K8S-UI-CA-REQUIRED" "$PROBE_BROKEN" "server" "#6821" \
    "a CA with no revocation service under caRevocation=required: the viewer's CDR connection fails closed"
  if ! k8s_ca_install required; then
    probe_fail "the TLS release with the viewer rolled out" "$K8S_HELM_REFUSED" \
      "the server boots over its own listener before the viewer's trust is read"
  else
    seen="$(k8s_ca_sign_in)"
    assert_contains "$seen" "form=yes" \
      "the viewer must answer with its form and the reason, not an error page from elsewhere"
    assert_not_contains "$seen" "status=302" \
      "a chain no CRL or OCSP check cleared must never reach the CDR (ferrohealth-server 0.14)"
  fi
  probe_done

  probe "P-K8S-UI-CA-NONE" "$PROBE_WORKING" "chart" "#6821" \
    "the same CA under caRevocation=none: a Basic sign-in through the viewer reaches the CDR over TLS"
  if ! k8s_ca_install none; then
    probe_fail "the TLS release with the viewer rolled out" "$K8S_HELM_REFUSED"
  else
    seen="$(k8s_ca_sign_in)"
    assert_contains "$seen" "status=302" \
      "the CDR checked the credentials over the in-cluster TLS hop and the viewer redirected"
    assert_not_contains "$seen" "location=/sign-in" \
      "a redirect back to sign-in is a refusal, not a session"
  fi
  probe_done

  if ! { k8s_ui_install && k8s_rollout 240s && k8s_ui_rollout; }; then
    red "the release did not return to the plain listener; the probes after this one measure a TLS release"
  fi
  kc delete secret "$K8S_CA_SERVER_SECRET" "$K8S_CA_SECRET" --ignore-not-found >/dev/null 2>&1
}
