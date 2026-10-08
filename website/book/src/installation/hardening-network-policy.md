# Namespaces, network & policy

Where the release's boundaries are drawn: namespace scoping, how organisations
are separated, the decisions not to adopt a mesh or a second policy engine,
resource bounds in four nested layers, the ingress policy that narrows ports
before it narrows sources, and deny-by-default egress: the one control on these
pages that is not free.

<!-- toc -->

## Namespaces, and one instance per organisation

**Ours, and satisfied by construction.** Every object the chart renders is
namespace-scoped: Deployment, Service, ConfigMap, Secret, ServiceAccount,
NetworkPolicy, PodDisruptionBudget, HorizontalPodAutoscaler, Ingress,
ServiceMonitor, the migration Job, and the viewer's own Deployment,
Service, ServiceAccount, NetworkPolicy and Ingress when that workload is enabled.
There is **no ClusterRole, no ClusterRoleBinding, no CustomResourceDefinition, no
cluster-scoped object of any kind**, and no template hard-codes a namespace: every
reference resolves within the release's own namespace. So two releases in two
namespaces cannot collide, and neither can reach the other's Secrets.

**Separating organisations is a release per organisation**, each in its own
namespace with its own database. FerroEHR carries no in-database tenancy, and
that is where openEHR puts the boundary: an openEHR system is one organisation's
repository, and multi-tenancy is something the infrastructure beneath it does
with several such systems (BASE `architecture_overview`
`master06-design_of_the_ehr.adoc` §The EHR System).

What that buys, against the in-process alternative it replaces:

| | One release per organisation |
|---|---|
| Isolation boundary | Kubernetes plus PostgreSQL: separate Secrets, NetworkPolicies, quotas, RBAC, and a separate database |
| Blast radius of an application-level bug | one organisation, because a query has nothing else to reach |
| Blast radius of a compromised database credential | one organisation's database |
| Cost | one Deployment, one connection pool and one image pull per organisation |

The cost is real and it is the price of the boundary. A shared process scoped by
a row predicate is cheaper and is not equivalent, however similar the wire
behaviour looks.

## Service mesh: a recorded decision

**Not adopted, deliberately.** The cheat sheet presents a mesh as a trade-off, not
a requirement, and for this workload the trade lands clearly.

What a mesh would provide, and what already provides it:

| Mesh benefit | Already covered by |
|---|---|
| mTLS between services | the server terminates TLS natively (`config.server.tls`), including client-certificate authentication for the IHE ATNA node-authentication posture; the database connection uses `sslmode=verify-full` |
| East-west traffic restriction | the shipped NetworkPolicy, and for the viewer an egress policy that admits the CDR Service, DNS and outbound HTTPS and nothing else |
| Request-level observability | OTLP traces and Prometheus metrics from the application, which sees openEHR operations rather than an L7 proxy's view of opaque HTTP |
| An audit trail of access | the ATNA/BALP audit trail, which records *who read which patient's record*, a property no proxy can reconstruct |

Against that: a mesh is a second control plane, a sidecar in every pod (which the
Restricted profile and the empty capability set then have to accommodate), and an
opinionated platform to upgrade in lockstep. For **one workload plus an external
database**, the lateral-movement problem a mesh exists to solve barely exists.

**A genuine gap, named rather than glossed:** a mesh would give *workload
identity*: SPIFFE-style cryptographic identity per pod, so the database could
authenticate the client workload rather than a shared password held in a Secret.
Nothing in this deployment provides that; the DSN is a bearer credential, and any
process that can read the Secret can use it. The mitigations available without a
mesh are an external secret manager plus short-lived credentials, for example
cloud IAM database authentication, where the DSN carries a rotating token rather
than a password, bound through `serviceAccount.annotations`. If you run a mesh
anyway, expect the overlaps above rather than double-implementing them, and note
that its sidecar will need its own Pod Security accommodation.

## Centralized policy, and which engine

**Operator's, and only for one of the three use cases a general policy engine is
usually proposed for.**

- **Application authorization: already solved, do not add a second engine.** This
  server ships a policy-driven authorization layer: RBAC, plus ABAC with an
  embedded Cedar engine or an external policy decision point. Adding a second
  engine for application decisions would mean two policy engines disagreeing about
  one question, and the one that loses is whichever is consulted second.
- **Service-mesh authorization: moot**, no mesh ([above](#service-mesh-a-recorded-decision)).
- **Admission control: applies**, and it is the same lever image provenance and
  Pod Security enforcement need.

**Decision: Kyverno**, chosen for what it must enforce rather than on general
merit. It is the only one of the three candidates that covers **both** levers with
one controller: `verifyImages` does keyless signature and attestation
verification, and its policies can enforce pod-security constraints beyond what
namespace labels express. `sigstore-policy-controller` does provenance only, so it
would have to be paired with something else. Kubernetes' built-in
**ValidatingAdmissionPolicy** (CEL, no external controller) is attractive for pod
shape and is the right tool for cheap structural rules, but it **cannot verify
signatures at all**, because CEL evaluation makes no network calls and cannot
reach a transparency log, so it cannot be the answer to the control that matters
most here.

If you already run OPA/Gatekeeper for other reasons, keep it and add
`sigstore-policy-controller` alongside for provenance; do not run two general
admission engines. The copyable provenance policies are in
[Images](hardening-supply-chain.md#image-provenance-at-admission), and Pod
Security enforcement needs no engine at all: it is
[the namespace label](hardening-workload.md#pod-security-admission-complying-versus-being-refused).

## Resource bounds: four layers

**Split exactly.** Container requests and limits are the chart's; namespace
`ResourceQuota` and `LimitRange` are cluster-admin objects, and a workload chart
that created one would be claiming the whole namespace for itself, wrong the
moment anything else shares it.

The chart's bounds, and where they come from:

| | Value | Derivation |
|---|---|---|
| `resources.requests.cpu` | `250m` | the scheduling floor: enough to boot, run migrations and serve steady traffic. An idle-but-serving pod uses far less, so this is deliberately generous rather than tuned to observed idle: a request is what the scheduler reserves, and a too-tight one gets the pod placed on a node with nothing left for a traffic spike. |
| `resources.requests.memory` | `256Mi` | observed steady-state resident use is a fraction of this, and the headroom is for the connection pool, the template and WebTemplate caches, and the per-request AQL working set. |
| `resources.limits.cpu` | `2` | AQL execution is the CPU-heavy path and is bounded per query by `config.query.timeout_ms`; two cores lets a query and normal traffic proceed without throttling. CPU limits throttle rather than kill, so this trades latency, not availability. |
| `resources.limits.memory` | `1Gi` | a **hard** ceiling: exceeding it is an OOM kill, so it sits well above observed use. The application-level body and result limits below are what keep a single request from approaching it. |

Those figures are for a modest replica. Tune them from your own metrics rather than
treating them as a recommendation, and remember that raising `replicaCount`
multiplies the request, which is what a namespace quota will notice first. The
viewer has its own, smaller bounds under `viewer.resources`.

**The layering is the point**, and an operator should see it as one story: four
nested bounds, each catching what the next cannot:

1. **Per request:** `config.server.limits.body_bytes` (413 on an over-large
   body), `config.query.timeout_ms` and `config.query.max_result_rows` (a query
   cannot return an unbounded row set), `config.db.statement_timeout_ms` as the
   backstop the HTTP timeout cannot be, because dropping a handler future does not
   cancel the statement PostgreSQL is running.
2. **Per connection / per caller:** `config.server.connection.header_read_timeout_secs`
   and its HTTP/2 siblings bound a socket before a request exists;
   `config.server.rate_limit` refuses `429`; `config.server.max_in_flight` sheds
   `503` when the server is full.
3. **Per container:** the requests and limits above.
4. **Per namespace:** the operator's, and the only layer that protects *other*
   workloads from this one:

```yaml
apiVersion: v1
kind: ResourceQuota
metadata:
  name: ferroehr
  namespace: ferroehr
spec:
  hard:
    requests.cpu: "2"
    requests.memory: 4Gi
    limits.cpu: "12"
    limits.memory: 8Gi
    pods: "12"
    count/services: "4"
---
apiVersion: v1
kind: LimitRange
metadata:
  name: ferroehr-defaults
  namespace: ferroehr
spec:
  limits:
    - type: Container
      # A pod that arrives with no limits gets these, so nothing in the namespace
      # can be unbounded by omission.
      default: {cpu: "1", memory: 512Mi}
      defaultRequest: {cpu: 100m, memory: 128Mi}
      max: {cpu: "4", memory: 2Gi}
```

Size the quota above `replicaCount` × requests with headroom for a rolling
upgrade: `maxSurge: 1` means one extra pod exists mid-rollout, and a quota with
no room for it makes upgrades stall rather than fail, which looks like a hung
deployment. The migration Job needs its share too when
`migrations.job.enabled` is on.

## Ingress: ports are narrowed, sources are yours

**The chart's mechanism, your peers:** and the one control on this page whose
default looks stronger than it is.

`networkPolicy.enabled` ships **on**, and the policy it renders admits inbound
traffic to the API port (plus the management port when that runs on its own
listener) and to nothing else. That half is unconditional. The other half is not:
the rule's **sources** are narrowed only when you set `networkPolicy.ingressFrom`.
While that list is empty the rule carries no `from` clause at all, and in the
NetworkPolicy API a rule with no `from` admits **every** source, other
namespaces and off-cluster clients included, not "any pod in this namespace"
([NetworkPolicies](https://kubernetes.io/docs/concepts/services-networking/network-policies/)).

So the shipped posture is: **ports restricted, sources open**. Nothing about
`kubectl get networkpolicy`, the object's own name, or a summary that says
"NetworkPolicy: enabled" distinguishes that from a policy that restricts both:
which is why the chart now states it as a value you can see and change rather
than leaving it implicit in an empty list.

Narrow it to whatever fronts the CDR:

```yaml
networkPolicy:
  enabled: true
  ingressFrom:
    - namespaceSelector:
        matchLabels:
          kubernetes.io/metadata.name: ingress-nginx   # your controller's namespace
    # - ipBlock: {cidr: 10.0.0.0/16}                   # or a CIDR, for off-cluster callers
  ingressAllowAll: false                               # refuse to ever render the open rule
```

The viewer does not need an entry: when `viewer.enabled` is on, the chart
appends the viewer's own pod selector to whatever you list, because a narrowed
policy that forgets it locks the viewer out of the CDR and presents as the CDR
being down.

**The viewer's own policy works exactly the same way**, under
`viewer.networkPolicy.ingressFrom` and `viewer.networkPolicy.ingressAllowAll`,
with the same refusal on `false` + empty. Its egress half is genuinely closed
(the CDR Service and DNS, nothing else) but its ingress half ships open like the
CDR's, and what sits behind it is a login page. Narrow both.

> [!IMPORTANT]
> **`networkPolicy.ingressAllowAll: false` makes "no open ingress" a checked
> fact.** With it set, an empty `ingressFrom` is refused at render time, the same
> treatment [egress](#egress-deny-by-default-and-what-it-breaks) gives a policy
> with no database destination, and for the same reason: an absent selector is a
> decision, and this one is invisible in the place an operator would look for it.
> Set it in the values file of any deployment where an open ingress rule would be
> a finding, and a later edit that drops your `ingressFrom` fails the install
> instead of quietly reopening the port.
>
> It ships as `true`, which is the honest name for what the chart has always
> rendered: a stock `helm install` produces the open rule, and now says so, in
> the values file, in the object's `kubernetes.io/description`, and in the notes
> printed at install. A non-empty `ingressFrom` always narrows, whatever this key
> is set to; it decides the empty case only.

> [!WARNING]
> **A NetworkPolicy is only as real as the CNI that implements it.** On a cluster
> whose network plugin does not enforce NetworkPolicy, the object is accepted,
> stored and displayed with no effect and no warning. Verify by attempting a
> connection the policy should refuse, from a pod in another namespace, since
> that is precisely the source an empty `ingressFrom` admits.

## Egress: deny by default, and what it breaks

**The mechanism is the chart's; the destinations are yours:** and this is the
control where a NetworkPolicy stops being free.

Enabling `networkPolicy.egress.enabled` refuses **all** outbound traffic except
what is listed. DNS is always included, and the database is a first-class value
because it is the only destination that is never optional:

```yaml
networkPolicy:
  egress:
    enabled: true
    database:
      to:
        - ipBlock: {cidr: 10.0.12.7/32}     # a managed database
        # or, in-cluster:
        # - podSelector: {matchLabels: {app.kubernetes.io/name: postgres}}
      port: 5432
    rules: []                                # one entry per integration, below
```

> [!IMPORTANT]
> Enabling egress with **no destinations at all** (neither
> `networkPolicy.egress.database.to` nor `networkPolicy.egress.rules`) is refused
> at render time rather than at rollout, because that mistake presents as a
> database fault: readiness reports the database down, the log shows a connect
> timeout, and nothing mentions the network policy. Supplying `rules` but omitting
> the database destination renders, so keep the database entry where it belongs
> rather than folding it into a raw rule where it is easy to lose.

**The destination set, derived from the configuration tree rather than from what a
default install happens to use.** Every row except the usage report is off
unless its key is set, so add only the rows you have switched on:

| Destination | Turned on by | Port | Typically |
|---|---|---|---|
| Cluster DNS | always | UDP+TCP 53 | in-cluster (always allowed) |
| PostgreSQL | the DSN, **never optional** | the DSN's (5432) | off-cluster (managed) |
| OTLP collector | `config.telemetry.otlp_endpoint` | 4317 gRPC / 4318 HTTP | in-cluster |
| OIDC issuer (discovery + JWKS) | `config.auth.oidc.issuer`, **unless** `config.auth.oidc.jwks_json` or `config.auth.oidc.jwks_json_file` is supplied | 443 | off-cluster |
| External policy decision point | `config.authz.abac.engine: remote` plus `config.authz.abac.remote.server` | the URL's (3001) | in-cluster |
| FHIR terminology server(s) | `config.terminology.external.enabled` plus a provider `url` | 443 | off-cluster |
| FerroTERM in the cluster | `terminology.enabled` | 8080 | in-cluster, and the **chart renders this rule itself** — it chose the destination when it wired the CDR at the Service, so you do not add it to `rules`. Omitted, terminology would fail silently: under the shipped fail-open posture an unresolvable binding is accepted, so nothing refuses and nothing logs. |
| Terminology token endpoint | `config.terminology.external.oauth2_clients.<name>.token_url` | 443 | off-cluster |
| AMQP broker (events) | `config.events.enabled` plus `secrets.eventsUrl` | 5672, or 5671 with `config.events.tls` | in-cluster |
| Object store | `config.multimedia.enabled` plus `config.multimedia.endpoint` (unset means AWS regional resolution) | 443, or the endpoint's | off-cluster |
| Syslog audit repository | `config.audit.syslog.enabled` | 514 UDP, or 6514 TCP with `config.audit.syslog.transport: tls` | off-cluster |
| FHIR audit repository | `config.audit.fhir_feed.enabled` plus `secrets.auditFhirFeedUrl` | 443 | off-cluster |
| FerroPULSE usage report | `usageReport.enabled`, **on by default** | 443 | off-cluster, `report.ferropulse.eu`. Without the rule the report fails quietly and nothing else is affected; see [Usage report](../usage-report.md#network) |

The viewer, when enabled, carries its own egress policy rather than
appearing in this table: it admits the CDR Service, DNS, and outbound HTTPS for an
identity provider. Narrow that last rule to your issuer's address if you can.

> [!WARNING]
> **A NetworkPolicy cannot match a DNS name.** The API selects peers by pod,
> namespace or `ipBlock` CIDR only, so every off-cluster row above needs a CIDR
> you supply and keep current. A managed database that moves IP, or a terminology
> server behind a rotating CDN address, will break under a CIDR that was correct
> when written. Where a provider publishes no stable range, the honest options are
> an egress gateway with a fixed address, or leaving egress off and accepting that
> outbound traffic is unrestricted, not a `0.0.0.0/0` rule that pretends to be a
> policy.

**Two failure modes worth knowing before you tighten this.**

**An over-tight policy silently stops observability.** A blocked OTLP exporter
does **not** fail the request that generated the span: it drops the span, with no
log line and no error. So a policy that forgets the collector produces a server
that is healthy by every check and has quietly stopped being observable. If you
enable egress and traces disappear, look at the policy before the collector.

**Tightening egress under a running pod appears to work when it has not.** A
NetworkPolicy is enforced on new connections; existing conntrack flows survive. So
a pod whose connection pool is already established keeps serving after you remove
its database rule, and fails at the next restart, which may be a node drain in
the middle of the night. Observed on a live cluster:

```text
policy: DNS + database  → readiness 200 {"status":"UP", db UP, migrations UP}, POST /ehr 201
remove the database rule (DNS only), pod untouched
                        → readiness STILL 200, db STILL "UP"   ← the pool survives
delete the pods so a fresh one must connect
                        → readiness 503, no replica available
                          kubelet: Readiness probe failed: HTTP probe failed with statuscode: 503
restore the rule        → ready again, with no restart
```

So: **verify an egress policy by restarting a pod, not by watching the one that is
already running.** (The recovery in the last line needing no restart is the
readiness check re-testing its dependencies on every probe, the same property
described for [migrations](kubernetes.md#health-probes).)
