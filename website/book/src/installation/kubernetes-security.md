# Kubernetes: security posture, probes and authentication

What the chart enforces about the workload, how it probes health, and the authentication
it requires before it installs. Installing the chart is on [Kubernetes & Helm](kubernetes.md).

<!-- toc -->

## Security posture

The chart pins the following, and its render gate holds it: the Restricted fields
are asserted per container for every workload in the render, the two isolation
settings are asserted to agree across a release's workloads, and the golden
renders pin the exact bytes so a changed default fails a diff.

| Field | Value |
|---|---|
| `runAsNonRoot` | `true` (uid/gid 65532, the distroless `nonroot` user) |
| `readOnlyRootFilesystem` | `true` (a writable `emptyDir` is mounted at `/tmp`) |
| `allowPrivilegeEscalation` | `false` |
| `capabilities.drop` | `[ALL]` |
| `seccompProfile.type` | `RuntimeDefault` (pod and container) |
| `hostUsers` | `false`; the pod gets its own user namespace |
| `supplementalGroupsPolicy` | `Strict`; only the groups the manifest names |
| ServiceAccount token | not mounted (the workload never calls the K8s API) |
| `enableServiceLinks` | `false` (see below; not a preference) |
| NetworkPolicy | ports narrowed to the API (and management) port; **sources admitted unless you narrow them** with `networkPolicy.ingressFrom`; set `networkPolicy.ingressAllowAll: false` to have the chart refuse the open state (see [§Ingress](hardening-network-policy.md#ingress-ports-are-narrowed-sources-are-yours)) |

The whole set satisfies the Pod Security **Restricted** profile, and the
[deployment probe harness](./kubernetes-hardening.md) reads it back off a running
pod rather than off the rendered manifest: the container runtime's own spec for
the security context, the API server for admission, the EndpointSlice for
readiness.

Satisfying the profile and **enforcing** it are different things, and only one of
them is yours to do. Enforcement comes from a label on the namespace, which a
chart cannot set for a namespace it does not own, so label it, or the posture
above is a convention nothing checks and nothing fails when a future change
regresses it:

```shell
kubectl label --overwrite namespace ferroehr \
  pod-security.kubernetes.io/enforce=restricted \
  pod-security.kubernetes.io/enforce-version=latest
```

With the label in place the API server refuses a non-compliant pod outright
([Pod Security
Admission](https://kubernetes.io/docs/concepts/security/pod-security-admission/)),
which is a stronger guarantee than any check the chart can make about itself.
The install notes print this as a prerequisite for the same reason.

`enableServiceLinks: false` is load-bearing, not hygiene. The kubelet injects a
[set of Service link environment
variables](https://kubernetes.io/docs/concepts/services-networking/service/#environment-variables)
for every Service in the namespace, and for a Service named `ferroehr*` those
land inside the server's reserved `FERROEHR_` namespace, whose strict boot-time
sweep rejects unknown variables and refuses to start. Leaving service links on
makes every install crash-loop.

Egress restriction is opt-in (`networkPolicy.egress.enabled`) because egress
targets (the database, broker, terminology server) are deployment-specific;
when you enable it the chart always admits DNS and you add rules for the rest.
In the default posture the server's only outbound traffic is DNS and PostgreSQL,
so those two rules suffice; each integration you switch on adds a target, and a
blocked one can fail silently. The full destination table is in
[Namespaces, network & policy](./hardening-network-policy.md#egress-deny-by-default-and-what-it-breaks).

Two limits worth stating plainly. First, with `networkPolicy.ingressFrom` empty
the rendered ingress rule carries no `from` selector, and a rule without `from`
admits **every** source, other namespaces included. Only the port list is
narrowed in that state, so set `ingressFrom` to your ingress controller for a PHI
workload, and set `networkPolicy.ingressAllowAll: false` if an open ingress rule
must never render at all; the full treatment is
[§Ingress](hardening-network-policy.md#ingress-ports-are-narrowed-sources-are-yours).
The viewer's own policy carries the same pair under `viewer.networkPolicy`.
Second, a NetworkPolicy is only as real as the CNI that implements it:
on a cluster whose network plugin does not enforce NetworkPolicy the object is
documentation rather than a control, and nothing in Kubernetes reports that.
Confirm it by attempting a connection the policy should refuse.

The database-side controls (the server certificate behind `sslmode=verify-full`,
pgaudit, at-rest encryption, WAL archiving / PITR) belong to whoever provisions
PostgreSQL; the chart mounts the CA (`database.caConfigMap`) but cannot enforce
the rest. See
[Operations](../operations.md).

### TLS at the pod

The production overlay ends TLS at the Ingress and accepts `plaintext_listener`
by name. To close that gap instead, the server terminates TLS itself
(`config.server.tls`), and every client the chart wires to it follows: the
viewer and FerroBRIDGE call `https://<release>:<service.port>`, the kubelet
probes over HTTPS, the Service declares `appProtocol: https`, and a
ServiceMonitor that scrapes the API port uses `https` too.

The clients check the server's certificate against a CA you name in
`service.tls.caExistingSecret`. The certificate must carry the Service name
(the release's full name, `ferroehr` for a release called `ferroehr`) as a DNS
SAN, which no public CA issues, so the chart refuses to render a TLS listener
beside an enabled viewer, FerroBRIDGE or API-port scrape while that value is
empty. Name a Secret that holds the CA certificate alone: every pod and the
ServiceMonitor that read it then read no private key, and a ServiceMonitor in
another namespace reads its own copy there. A cert-manager `Certificate`
writes the key pair and the CA into one Secret (`tls.crt`, `tls.key`,
`ca.crt`); copy `ca.crt` out of it (trust-manager's `Bundle` does this) rather
than naming that Secret. The Ingress in front of the TLS listener must reach
it over HTTPS too, and how a controller is told so is controller-specific, so
the chart refuses to render the Ingress with server TLS on until
`ingress.backendTlsAnnotations` names it:

```yaml
config:
  deployment.accepts: []          # nothing left to accept for the listener
  server:
    tls:
      certificate_file: /etc/ferroehr-tls/tls.crt
      key_file: /etc/ferroehr-tls/tls.key
      reload_interval: 1min
extraVolumes:
  - name: server-tls
    secret:
      secretName: ferroehr-server-tls
      defaultMode: 0440
      items:
        - { key: tls.crt, path: tls.crt }
        - { key: tls.key, path: tls.key }
extraVolumeMounts:
  - { name: server-tls, mountPath: /etc/ferroehr-tls, readOnly: true }
service:
  tls:
    caExistingSecret: ferroehr-server-ca    # ca.crt alone
ingress:
  backendTlsAnnotations:                    # ingress-nginx
    nginx.ingress.kubernetes.io/backend-protocol: HTTPS
```

When cert-manager renews the certificate, the kubelet updates the mounted
files and the server serves the new pair within
`config.server.tls.reload_interval` (a minute above; unset, only `SIGHUP`
reads the files again), with no restart; a renewed pair that fails to load is logged and the old one stays in
service. A rotated client CA (`client_ca_file`, for `client_auth`) is picked up
on the same interval, and the connections already open keep the trust anchor
they were verified against.

The viewer reads the mounted CA through its own `[cdr.tls]` table, beside the
image's roots, in the key `service.tls.caRevocation` names (below). FerroBRIDGE reads it through `SSL_CERT_DIR`, which
the chart sets to the image's system roots plus the mounted CA, so its other
peers that verify through the platform verifier stay trusted. For an OIDC
issuer whose certificate comes from a private CA, name a Secret holding that
CA in `viewer.auth.oidc.caExistingSecret` (key `viewer.auth.oidc.caExistingSecretKey`,
`ca.crt` by default): the chart mounts it and points the viewer's
`[oidc.tls] trust_roots_file` at it. For the server's own `[auth]`
issuer behind a private CA, name the Secret in `auth.caExistingSecret` (key
`auth.caExistingSecretKey`, `ca.crt` by default): the chart mounts it and points
the server's `[auth.tls] trust_roots_file` at it.

Each of these three CAs takes a sibling `caRevocation` (`service.tls.caRevocation`,
`viewer.auth.oidc.caRevocation`, `auth.caRevocation`). The default, `required`,
names the CA in `trust_roots_file`: a connection opens only once a check of
the chain's CRL or OCSP sources cleared it, so the CA must publish one the
pods can reach. `none` names the CA in `trust_roots_without_revocation_file`,
trusted with no revocation check and named at the start. A cert-manager CA
issuer publishes no CRL, so an in-cluster CA from it needs `none`
([How an outbound TLS table verifies a peer](config-integrations.md#how-an-outbound-tls-table-verifies-a-peer)).
The ServiceMonitor reads the Secret from its own namespace, and
the `prometheus.io/scheme: https` pod annotation follows the listener when
`metrics.enabled` is set. With `config.server.tls.client_auth: required` the
kubelet probes cannot present a client certificate, so the chart refuses to
render it; keep it at `none` or `optional`.
`config.management.listen` stays plain HTTP on its own listener.

## Health probes

All three probes use the always-on, unauthenticated, PHI-free health routes on the
main HTTP port. They need no configuration at all: no management surface, no
access level, nothing to forget:

| Probe | Route | Contract |
|---|---|---|
| liveness | `/health/liveness` | 200 while the process is up; touches no dependency |
| readiness | `/health/readiness` | 200 (up or degraded) or 503 (down, booting or draining): checks the database ping, migrations applied and the audit sender, each bounded at one second |
| startup | `/health/liveness` | the same constant, with a long failure threshold, so a slow first boot is not killed mid-migration |

That split is deliberate: a database outage must fail *readiness* (the pod stops
receiving traffic) and never liveness (which would restart the container in a
loop).

> [!NOTE]
> The chart offers no `exec` probe: the `httpGet` probes above are what it
> runs. The binary's `healthcheck` subcommand asks `/health/readiness`.

> [!WARNING]
> **Migrations run only at boot, so a replaced or wiped database leaves the pods
> NotReady until something migrates it.** Readiness reports the `migrations`
> indicator `down` (its detail on `/management/health`: `core schema tables
> missing (migrations not applied)`) while `db` reads `up`: the pod reaches PostgreSQL, finds no
> schema, and does not migrate again, because migration is a startup step.
> Liveness keeps passing (correctly: the process is healthy), so the kubelet never
> restarts the container and the Deployment sits with no ready replicas and no
> error in the logs after the first one:
>
> ```shell
> kubectl -n ferroehr get pods                      # Running, not READY
> curl -s http://ferroehr:8080/health/readiness     # 503, "migrations" down, "db" up
> kubectl -n ferroehr rollout restart deploy/ferroehr
> ```
>
> The check re-tests the schema on every probe, so a pod recovers on its own within
> one `probes.readiness.periodSeconds` of the schema existing; an untouched pod
> returns to ready, with no restart, once anything else has migrated. What needs
> the restart is the case where the only thing that would migrate is the pod
> itself.
>
> That is what makes the interaction with flow (b) load-bearing: when migrations
> run out of band, the migration step must complete *before* the Deployment rolls,
> or the first pods sit unready waiting for a schema. `migrations.job.enabled`
> does exactly that, as a hook Helm waits on. Durable storage is the other
> half: an `emptyDir`-backed or otherwise disposable PostgreSQL puts a clinical
> repository one node eviction away from this state.
>
> One recovery path does **not** self-heal, and it is worth knowing before you try
> it: restoring or dropping *part* of the schema set. A restore that brings back
> some of a domain's relations without its migration bookkeeping makes the next
> migration run fail permanently with `relation "version" already exists`: the
> pod crash-loops, and restarting it retries the same failure. Recreate the whole
> database rather than one schema.

The management surface is independent of the probes and stays ops-only
(`/management/info`, `/prometheus`, `/metrics`, `/env`, `/loggers`,
`/flamegraph`). Unlike the bare binary, the chart ships
`config.management.enabled: true` with **every endpoint off**, so nothing is
exposed until you opt one in. Set
`config.management.endpoints.prometheus: public` plus `metrics.enabled=true`
to add the `prometheus.io/*` scrape annotations; set `config.management.listen`
to serve the surface on its own internal listener, so `/management` is never
reachable on the clinical API port. The health probes stay on the main port
regardless of all of this, and the install notes warn you if you add the scrape
annotations while the endpoint is still `off`.

Those annotations are honoured by a Prometheus that discovers targets in its own
scrape configuration. An **operator-managed** Prometheus
(kube-prometheus-stack) ignores them and selects targets through
[ServiceMonitor](https://prometheus-operator.dev/docs/developer/getting-started/)
objects instead, so set `metrics.serviceMonitor.enabled=true` there;
`metrics.serviceMonitor.labels` is where the label your Prometheus'
`serviceMonitorSelector` matches on goes. It needs the `monitoring.coreos.com`
CRDs installed first, or the install fails on an unknown kind.

## Authentication is required, and the chart says so before it installs

`config.auth.enabled` defaults to **true**, and the server requires at least one
mechanism with it: a `401` challenge has to name a scheme the server actually
implements ([RFC 9110 §11.6.1](https://www.rfc-editor.org/rfc/rfc9110#section-11.6.1)),
so it exits rather than run as an openEHR API that can only refuse every request.

The chart therefore **refuses to render** a values file that enables
authentication without configuring one, so `helm install` stops with an
actionable error instead of reporting success and crash-looping:

```text
Error: execution error at (ferroehr/templates/deployment.yaml:...):
config.auth.enabled is true but no authentication mechanism is configured...
```

Pick one:

```yaml
config:
  auth:
    issuer:
      keycloak:
        issuer: https://keycloak.example.org/realms/ferroehr
        audience: [ferroehr]
        profile:
          token_types: [JWT]
        party:
          identifier_issuer: urn:example:staff-login
          identifier_type: uuid
  authz:
    rbac:
      role_issuers: [keycloak]
    party_binding:
      provider: { issuer: "urn:oid:2.16.528.1.1007.3.3", id: "12345678" }
```

or Basic auth, whose password hashes go in `secrets.basicUserPasswordHashes`
while the user stays under `config.auth.basic.users`; its scopes and roles
are grants on its `PERSON`, and `config.authz.rbac.first_admin` names the
Basic user that holds `admin_role`. The authorization server's registry is
the one list of clients, so a token whose `aud` names FerroEHR admits its
client ([Authentication & access](config-auth.md)).
A Basic user is a person: each maps to the binding key of its `PERSON` under
`config.authz.party_binding.basic_user`, under the identifier issuer of the
`party` table under its issuer in `config.auth.issuer` (see
`deploy/helm/ci/basic-auth-values.yaml`).

The server stores FerroEHR's
[default demographic definitions](default-demographic-definitions.md) as it
boots, and with no mapping key set the party mapping names them, so a chart
install needs no list of definitions. A party mapping that names templates
of your own needs them in the definition store before the server boots,
which refuses a mapping whose templates it lacks. `definitions.files` lists
them, each a `name` ending in `.opt` (OPT 1.4) or `.adls` (ADL 2) and its
`content`: the chart puts them in a ConfigMap of their own and runs
`ferroehr definitions load` on them as an init container, with the server's
image, configuration and secrets, in the order listed. A definition already
stored with the same text is left alone, so the step runs at every pod
start, and one stored under its id with other text fails the pod. The list
is empty by default. `deploy/helm/ci/basic-auth-values.yaml` keeps the
default mapping and provisions its user through `/scim/v2/Users` (see
[`[auth.scim]`](config-scim.md#authscim)).

An issuer with `sender_constraint: required` runs on any number of
replicas: each records the DPoP proofs it accepts in the database they all
share, so a proof replayed to another replica is refused there (RFC 9449
§11.1).

> [!WARNING]
> `config.auth.enabled: false` makes the chart render, and serves **every**
> request unauthenticated. On a repository holding patient data that is a
> development-only choice; the chart makes you state it explicitly rather than
> reaching it by omission.
