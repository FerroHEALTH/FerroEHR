# Operations

Running a clinical data repository in production means more than starting the
binary: the database must be backed up and least-privileged, traffic must be
encrypted, upgrades must be safe while the service stays up, and you need to
see what the system is doing. This chapter covers TLS, the container image,
observability, the health probes and the management surface. The database
has its own page, [Database roles, migrations and
backup](database-operations.md), and so do [Storage sizing](storage-sizing.md),
[Upgrades](upgrades.md) and the [Admin & messaging
APIs](operations-admin-apis.md).

<!-- toc -->

## TLS and database security

These are database-side settings that belong to whoever provisions PostgreSQL;
the deployment references them but cannot enforce them:

- **TLS in transit.** Require `hostssl` on the server and put
  `?sslmode=verify-full` in the DSN so the client verifies the server
  certificate.
- **pgaudit.** Run pgaudit as the database-layer complement to the openEHR
  audit and the ATNA trail (for example `pgaudit.log = 'ddl, role,
  connection'` globally plus object-level audit on the PHI tables) and ship
  the audit log to an immutable store with long (roughly six-year) retention.
- **Encryption at rest.** Encrypt at the volume or disk layer. Do not encrypt
  the stored clinical JSON with pgcrypto: it would break AQL's ability to
  query inside the data.

## The container image and pod hardening

The published image is distroless and non-root (shell-less, with no package
manager) and is multi-architecture (amd64 and arm64) on GHCR. It is not a
static binary: the server links glibc and `libgcc_s` dynamically, which is why
the base image is the `cc` distroless variant rather than the smaller one, and
why the runtime needs no OpenSSL, no JVM and nothing else from a package
manager.

Under Kubernetes the pod runs the Kubernetes `restricted` profile: non-root
uid/gid 65532, read-only root filesystem with one writable `/tmp`, no
privilege escalation, an empty capability bounding set, `RuntimeDefault`
seccomp, and no service-account token (the workload never calls the Kubernetes
API). The chart's own gates assert that per container on every render, and the
posture read back from a running container, plus the two things the chart
cannot do for you (applying the namespace enforcement label, and narrowing the
NetworkPolicy's ingress sources: the shipped policy narrows the *ports* and,
until you set `networkPolicy.ingressFrom`, admits every source on them; set
`networkPolicy.ingressAllowAll: false`, and `viewer.networkPolicy.ingressAllowAll`
for the viewer, to have that state refused at render instead), are covered in
[Installation → The workload: security context & admission](installation/hardening-workload.md)
and [Namespaces, network & policy →
Ingress](installation/hardening-network-policy.md#ingress-ports-are-narrowed-sources-are-yours).

The Compose path carries a comparable floor, and one guard keeps it:
`scripts/checks/compose-hardening.sh` runs over every committed compose
artifact and fails on a service definition without `cap_drop: [ALL]` and
`no-new-privileges:true`, on any `privileged: true`, on a `seccomp=unconfined`
or `apparmor=unconfined` override, on a mounted Docker daemon socket, and on a
published port that names no host address. The compose files go further than
the guard checks (capabilities are added back one at a time only where an
entrypoint provably needs them, file-descriptor limits are bounded, ports
default to the loopback interface, and most services run a read-only root
filesystem) but those extras are conventions, not enforced properties, so
check them when you adapt a file.

Two services in the quickstart file deliberately do not run read-only, and each
says why in place. The S3 gateway writes its volume store; the server cannot,
because Compose refuses an inline `config` in a read-only service and that
inline config is what makes the file standalone. A deployment that mounts its
configuration from a file instead can add `read_only: true` and a tmpfs at
`/tmp`.

### What the host owes, and we cannot enforce

Four controls belong to whoever runs the daemon. They are stated here because a
container hardening story that ignores them is misleading: the strongest pod
security context in the world sits on top of these.

**Keep the host kernel and Docker Engine current.** A container is a kernel
namespace, not a virtual machine: a kernel privilege-escalation bug is a container
escape, and every runtime hardening above assumes the kernel enforcing it is
patched. Track your distribution's kernel updates and the Docker Engine release
notes with the same urgency you would give a public-facing service.

**Prefer rootless mode.** Running the daemon as a non-root user means a container
escape lands as an unprivileged user rather than as root on the host
(<https://docs.docker.com/engine/security/rootless/>). The images here need no
privileged operation, no host networking, and no daemon socket, so nothing in this
deployment prevents rootless: the constraints are usually the host's (cgroup v2,
`newuidmap`/`newgidmap`, and no privileged ports below 1024, which is why the
server binds 8080 rather than 80).

**Set the daemon log level to `info` (the default) and keep it.** Docker's `debug`
level records request payloads and can put secret material into daemon logs, which
are typically world-readable to anyone with host log access and are shipped
wholesale to log aggregators.

**Control who can pull and push your images.** GHCR access is the deployment's
authorization boundary for what runs in production: whoever can push a tag your
manifests reference can run their code with your database credentials. Restrict
push rights, prefer digest pins over mutable tags for anything you deploy (the
compose files pin the third-party images by digest for exactly this reason), and
verify the published attestations before rollout; the verification command is in
[Installation → Kubernetes & Helm](installation/kubernetes.md).

None of these four is something this project can assert on your behalf, which is
why they are written as your checklist rather than as our claim.

## Observability

`tracing` is the single instrumentation API. From it, three signal families
fan out, and **identified data never enters any of them**: telemetry uses
only closed-set labels and opaque request/trace ids, so correlation to a
patient is possible only through the [audit trail](audit.md).

- **Logs** go to stdout (JSON when not attached to a terminal, pretty on a
  TTY). Shipping and rotation are the platform's job.
  `FERROEHR__TELEMETRY__FORMAT` (`auto`/`json`/`pretty`) and
  `FERROEHR__TELEMETRY__FILTER` (or `RUST_LOG`, default
  `info,hyper=warn,tower=warn,h2=warn`) control them, and the level can be
  changed at runtime through the `loggers` endpoint below. Every request writes
  one request-log line: method, route template, status, latency and the
  server's request id. On boot the server prints a one-time ASCII banner
  (version, maintainer, project URL, and spec pins) to stdout ahead of the
  logs; it is suppressed under `FERROEHR__TELEMETRY__FORMAT=json` so machine
  log consumers see only structured lines.
- **Traces** export to any OpenTelemetry collector (Tempo, Jaeger, and so on)
  over OTLP, but only when you configure `[telemetry.otlp]`. Each request's
  span is a server span named `{method} {route}`, such as
  `GET /ferroehr/rest/openehr/v1/ehr/{ehr_id}`, never by a path containing
  ids, and its status is an error on a 5xx answer. A client's
  `traceparent` is never read: every request starts a trace of the server's
  own.
- **Metrics** come from **one** OpenTelemetry meter provider with up to two
  readers: a Prometheus reader behind `/management/prometheus`, and (with
  `[telemetry.otlp]` and `metrics = true`) a periodic OTLP reader. Every
  instrument reaches both by construction, so a family can never exist on the
  scrape surface and be missing from the push. The catalogue covers HTTP
  request duration (`http.server.request.duration`) and the requests in flight
  (`http.server.active_requests`); authentication failures
  and login-binding refusals; database pool state,
  acquire latency and transaction counts; AQL query counts, latency and plan-
  cache events; compositions committed (by openEHR audit change type);
  validation failures and version-signature faults; WebTemplate cache events;
  events published; the whole ATNA audit pipeline; Tokio runtime gauges;
  process start time and resident memory; and the `ferroehr_build_info`
  identity. The access path has its own set: the policy copies and their
  listeners, the access evaluator's decisions by `refused-*` code, the
  access log's appends, signatures, seals and retention, and the
  verification of the audit chains. [Access path metrics and
  runbooks](operations-access-path.md) lists them with a runbook for each
  failure.

  Instrument names carry **no** `_total` suffix and no unit suffix: units are
  declared on the instrument and the Prometheus exporter derives
  `_total`/`_seconds`/`_bytes` itself. Read the exposition to learn the exact
  rendered names rather than assuming either spelling.

The telemetry environment variables:

| Environment variable | Default | Meaning |
|---|---|---|
| `FERROEHR__TELEMETRY__OTLP__ENDPOINT` | unset (nothing exported) | OTLP collector endpoint |
| `FERROEHR__TELEMETRY__OTLP__TRACES` | `true` | export the spans |
| `FERROEHR__TELEMETRY__OTLP__METRICS` | `true` | push the metrics beside the Prometheus reader |
| `FERROEHR__TELEMETRY__OTLP__SAMPLE_RATIO` | `1.0` | share of traces sampled (start at `0.1` in production) |
| `FERROEHR__TELEMETRY__ENVIRONMENT` | unset | reported deployment environment |
| `FERROEHR__MANAGEMENT__FLAME_FILE` | unset (layer not installed) | write folded span-timing samples to this file for offline rendering, diagnostic sessions only |

Neither reader is reachable until you say so: the Prometheus surface needs
`management.enabled` plus an access level on the `prometheus` endpoint (see
[the management surface](#the-management-surface)), and the OTLP reader needs
`telemetry.otlp.endpoint`. A server with neither still records every
instrument; nothing exports it.

> [!TIP]
> A single-container dev stack (`grafana/otel-lgtm`, bundling an OTLP
> collector, Prometheus, Tempo, Grafana, and Loki) ships as a Compose overlay,
> together with a provisioned Grafana dashboard (request rate/errors/duration,
> database pool, AQL latency, validation failures, audit health) and a starter
> alert pack; point the server at it with the OTLP variables above.

**On Kubernetes**, the same keys arrive through the chart's `config`
passthrough, and the metrics half has a second switch that is easy to miss:

```yaml
# values.yaml
config:
  telemetry:
    environment: production
    otlp:
      endpoint: http://otel-collector.observability:4317
      sample_ratio: 0.1
      metrics: true
  management:
    enabled: true
    endpoints:
      prometheus: admin_only   # the scrape endpoint is off until you name it
metrics:
  enabled: true                # adds the prometheus.io/* pod annotations
  serviceMonitor:
    enabled: false             # or true, with the Prometheus Operator CRDs installed
```

`metrics.enabled` only adds the scrape **annotations**; the endpoint itself is
opened by `config.management.endpoints.prometheus`. Both are needed for an
annotation-discovering Prometheus, and neither is needed if you push over OTLP
instead. **To turn the export off**, drop `telemetry.otlp`; nothing is
exported without it.

**The default dashboard.** The "FerroEHR — service overview" Grafana
dashboard (request rate/errors/latency, AQL rate and phase latency,
plan-cache hit ratio, database pool, Tokio runtime, audit throughput — every
query written against the served metric names) reaches Grafana three ways:

- **Compose**: the observability overlay provisions it automatically;
  nothing to do.
- **Kubernetes**: set `metrics.grafanaDashboard.enabled: true` and the chart
  ships it as a ConfigMap labelled `grafana_dashboard: "1"`, which the
  Grafana Helm chart's dashboard sidecar (kube-prometheus-stack includes it)
  discovers and imports. The sidecar watches its own release namespace by
  default — install FerroEHR there, or widen the sidecar's
  `searchNamespace`. `metrics.grafanaDashboard.folder` sets the
  `grafana_folder` annotation for sidecars with folder routing configured.
- **Anywhere else**: import
  `deploy/helm/ferroehr/files/dashboards/ferroehr-overview.json` by hand
  (Grafana → Dashboards → Import).

> [!WARNING]
> With the chart's default-deny egress policy on, add the collector to
> `networkPolicy.egress.rules` (port 4317). An OTLP exporter that cannot reach
> its collector fails **silently**: no traces, no error.

### The usage report

Apart from your own telemetry, the server sends one report of its own: the
[usage report](usage-report.md) to FerroPULSE, the collector Cadasto B.V.
runs. It is on by default outside a CI run and goes out at each start and
once a day, with the version, the licence grant type and coarse latency
figures, and no patient data. `ferroehr usage-report --print` shows the exact
JSON, and `usageReport.enabled=false` (Helm) or
`FERROEHR__USAGE_REPORT__ENABLED=false` switches it off. With the chart's egress policy on, it needs a rule for
`report.ferropulse.eu` on port 443; without one it logs a warning.

## The admin and messaging APIs

Two operator-facing HTTP surfaces have a page of their own, because each route
carries its own switch, authorization class and status-code contract:

- **The admin API** (`{base}/admin/…`, off by default): physical deletion, the
  activity report, archiving to the cold tier, and whole-repository dump and
  load.
- **The messaging API** (`/ferroehr/rest/extensions/v1/message/…`, always mounted): EHR Extract
  export and import.

The full reference is [Admin & messaging APIs](operations-admin-apis.md).

> [!WARNING]
> Enabling the admin API puts irreversible, whole-repository operations on the
> wire: `DELETE {base}/admin/ehr/all` with no parameter empties the
> repository. Keep it off unless a workflow needs it, turn RBAC on, and gate
> the admin role tightly.

## Health probes

The health endpoints are **always served, on the main API port, without
authentication**. There is nothing to enable and nothing to remember: they are
mounted outside the API's authentication and overload-shedding layers, so an
orchestrator can probe a server whose management surface, admin API, and every
optional integration are switched off, and a saturated server still answers
its own probes.

### Choosing a health endpoint

| Endpoint | Contract | Use it for |
|---|---|---|
| `GET /health` | the readiness status (`200` or `503`) with the state alone, `{"state": …}` | load balancers, anything that must never be auth-gated |
| `GET /health/liveness` | constant `200` with `{"state":"up"}`, touches nothing | Kubernetes `livenessProbe` and `startupProbe` |
| `GET /health/readiness` | `200` while ready, `503` while the process boots or drains or an indicator is `down`; the FerroHEALTH readiness document (`state`, `phase` and each indicator's `name` and `state`), each check bounded to one second. Only `db` and `migrations` read `down`; every other indicator reads at worst `degraded` | Kubernetes `readinessProbe`, ops dashboards |
| `GET /ferroehr/rest/status` | public status document: `status`, `server_version`, `openehr_rest_api_version`, `timestamp` and `deployment.profile`. The full document, which adds `licence` (the grant in force: `state`, `use`, `licensee`, `not_after`, `configured_token`), the whole `deployment` posture (the declared profile, the separations still open and the ones accepted by name) and `support` (`release_date`, `support_ends`, `status`), is `GET /management/status` | version/identity checks |
| `GET /management/*` | ops introspection; see below | operators, off by default, enable deliberately |

Every management request is itself recorded in the audit trail as a
system-domain access under its own operation id (`management_info`,
`management_env`, `management_loggers_set`, …), so a runtime filter change or a
configuration read is never an unrecorded administrative act.

There is exactly one health surface: the `/health` family above, the one every
FerroHEALTH product serves, and the route the container's `ferroehr
healthcheck` subcommand asks is `/health/readiness`. `/ferroehr/rest/status` is
a different contract, and no health endpoint exists under the REST root.

Not every indicator blocks readiness, and the distinction is deliberate:

| Indicator | Checks | Blocks readiness |
|---|---|---|
| `db` | a pooled connection answers | yes |
| `migrations` | this build's schema is present, re-tested on every probe | yes |
| `audit_sender` | the audit posture: `DEGRADED` with the consequence stated when auditing is off (no access log, no EHDS logging component), and `DEGRADED` with its age when the oldest record of either log has waited more than three minutes past the last sealed window | no: reports `DEGRADED`, never `503` |
| `audit_delivery` | each forwarder to an external Audit Record Repository (present when `[audit.syslog]` or `[audit.fhir_feed]` is on): the records waiting, those parked (refused by the repository, not renderable, or removed by retention before delivery), those delivered, and whether the last attempt reached it; `DEGRADED` when the repository is unreachable or a record is parked, until an operator retries or dismisses it (`GET /admin/audit-delivery`) | no: records wait in their spool or the store, so delivery problems never take the instance out of rotation |
| `audit_chain` | the latest scheduled verification of the sealed windows of the system records and the access log (incremental from the last intact window, with a periodic full pass) (present when the local store is on and `[audit.store] verify_interval` is not `0`): `UP` while intact or before the first run, `DEGRADED` naming the finding count and the first damaged position, the reason the verification could not run, or the chain whose oldest unverified record waited longer than `[audit.store] max_verify_lag`; `audit_chain_intact` and [its runbook](operations-access-path.md#a-chain-verification-that-fails) | no: a damaged chain reports `DEGRADED` in the body, never `503`, because the trail keeps recording while the damage is investigated |
| `access_log_checkpoints` | whether the access log is sealed and its checkpoints published outside the database (present when the local store is on): `UP` with an OpenPGP key and `[audit.access_log] publish_directory` while publication works, `DEGRADED` without the key, without the directory (removing the newest entries then goes undetected), or while a publication fails, naming which; [its runbook](operations-access-path.md#a-checkpoint-publication-that-fails) | no: the log keeps recording |
| `policy_listeners` | whether each policy copy's `LISTEN` connection is up: `ehr_access`, `grants`, `party_binding`, `policy_registers` and `access_policy` (always present): `UP` while all five listen, `DEGRADED` naming each lost listener, while that copy reads the database on every request ([Running several instances](installation/several-instances.md)); `policy_listener_up` and [its runbook](operations-access-path.md#a-lost-policy-listener) | no: every decision stays correct while a listener is down, so the instance stays in rotation |
| `pmir_refused_messages` | the refused ITI-93 messages still to retry or settle (present when `[pix]` is set): `UP` below `[pix] refused_degraded_at`, `DEGRADED` naming their count from it on, each keeping the identifiers it split unresolved; the `pmir_messages_refused` gauge carries the count, and [`[pix]`](installation/config-integrations.md#pix) lists, retries and settles them | no: a split fails closed, so the instance stays in rotation |
| `instances` | whether every instance of the deployment that touched its row in the last 90 seconds signs with this instance's key and runs its configuration (always present): `UP` while they agree, `DEGRADED` naming each instance that runs another signing key or another configuration, or when the comparison could not run ([Running several instances](installation/several-instances.md)) | no: each instance keeps serving, so a difference is reported without taking any out of rotation |

An instance whose audit repository is unreachable therefore keeps taking
traffic and says so in the body; an instance that cannot reach its database, or whose
database lost the schema, leaves rotation. The `detail` strings are written to
carry no connection information (no DSN host, database name or role) because
this surface is unauthenticated by design.

> [!IMPORTANT]
> Liveness and readiness are deliberately different: liveness never touches a
> dependency, so a database outage takes the instance out of rotation
> (readiness `503`) instead of getting the container killed and restarted in a
> loop. Wire `livenessProbe` and `startupProbe` to `/health/liveness` and
> `readinessProbe` to `/health/readiness`. The Helm chart does exactly this out
> of the box, and only the timings are tunable (`probes.liveness`,
> `probes.readiness`, `probes.startup`); there is deliberately no option to
> point the probes at the container's `ferroehr healthcheck` subcommand
> instead, because that subcommand probes the status document rather than a
> health endpoint, which would leave readiness never touching the database.

While the database restarts or is out of reach, requests answer `503 Service
Unavailable` with `Retry-After`: those that cannot get a connection, and those
whose open connection the restart or the network cut (SQLSTATE class `08`,
`57P01` admin shutdown, `57P02` crash shutdown, `57P03` cannot connect now,
or a reset socket). A transaction the cut interrupted is rolled back by
PostgreSQL. Once the database answers again, no request is handed a
connection the outage closed while it sat idle: the pools test a connection
that has been idle for a second or longer before they hand it out, and
replace it when the database no longer answers on it, so the first request
after a restart is served. A request that waits more than a second for a connection makes
the server try a connection of its own: when that cannot reach the database,
the request answers `503` at once, and so does every request after it, until
a fresh attempt (at most every half second) connects again. A pool that is
only busy keeps its requests queued up to
[`[database] acquire_timeout`](installation/config-server.md#database) (30 seconds by
default).

A statement the database cancels (SQLSTATE `57014`, by
[`[database] statement_timeout`](installation/config-server.md#database) or by an
operator's `pg_cancel_backend`) also answers `503` with `Retry-After`: the
statement rolled back, so the request can be repeated. With the defaults the
request timeout (30 seconds) answers first, so the statement timeout (60
seconds) does not fire on a request. PostgreSQL measures its timeouts against
the database host's wall clock, so a host whose clock steps forward, such as a
virtual machine resynchronised after a stall, cancels every running statement
at once; keep the database host's clock synchronised by slewing, not stepping.

## The management surface

The management surface is **ops introspection only**: build info, Prometheus,
the metric views, the effective configuration, and runtime log control. It is
**off by default** on the bare binary, and each endpoint is independently
opt-in with an access level (`admin_only`, `private`, or `public`). It can be
bound to its own internal port so it never appears on the public API listener.
Keeping it off costs you nothing operationally: the health probes above do not
depend on it.

| Environment variable | Default | Meaning |
|---|---|---|
| `FERROEHR__MANAGEMENT__ENABLED` | `false` | enable the management surface |
| `FERROEHR__MANAGEMENT__BASE_PATH` | `/management` | base path for the surface |
| `FERROEHR__MANAGEMENT__LISTEN` | unset (main listener) | serve management on its own socket address, for example `0.0.0.0:9090` |
| `FERROEHR__MANAGEMENT__ENDPOINTS__<NAME>` | `off` | the access level for ONE endpoint: `off`, `private`, `admin_only` or `public`. There is no global default beside it: an endpoint you do not name is not mounted and answers `404`. |

The ops endpoints:

Every one of them ships `off`: nothing is mounted until you name the endpoint
and the level it should answer at. The right-hand column is the level to choose,
not a default you already have.

| Endpoint | Endpoint name to set | Purpose | Level to give it |
|---|---|---|---|
| `GET /management/info` | `info` | product name and version, build SHA, build date, `rustc`, the active `deployment.spec_profile`, the openEHR specification versions that profile selects, the PostgreSQL target, the `audit` posture (`enabled`), the `manufacturer` of the release (`name`, `postal_address`, `email`, `website`), and the `support` period (`release_date`, `support_ends`, `status`) | `admin_only` |
| `GET /management/prometheus` | `prometheus` | Prometheus text exposition | `admin_only`, or `public` only when the port is not reachable outside the cluster; a `public` endpoint is served OUTSIDE authentication |
| `GET /management/metrics` | `metrics` | JSON list of the registered metric names | `admin_only` |
| `GET /management/metrics/{name}` | `metrics` | the current value(s) of one metric; `404` for a name that is not registered | `admin_only` |
| `GET /management/env` | `env` | effective configuration, with secrets redacted | `admin_only` |
| `GET`/`POST`/`DELETE` `/management/loggers` | `loggers` | read and change the log level at runtime | `admin_only` |
| `GET /management/flamegraph` | `flamegraph` | on-demand CPU flamegraph of the running server | `admin_only` |

Paths above show the default `management.base_path`; change it and every path
moves with it. The `metrics` name covers both metric routes, and the
`prometheus`, `metrics` and `loggers` routes additionally need their backing
machinery present; without it they are simply not mounted, which is the same
`404` as leaving them `off`.

A `private` or `admin_only` endpoint answers `401` with a challenge when the
request carries no valid credential. When the credential holds and the login
binding refuses the caller, it answers `403` with the binding code, such as
`refused-unbound`, as every other route does, and an `admin_only` endpoint
answers `403` to a caller without `authz.rbac.admin_role`.

> [!WARNING]
> `/management/env` and `/management/loggers` expose and change server
> internals; keep them `admin_only`, and prefer binding the surface to an
> internal-only port.

### Profiling: the on-demand CPU flamegraph

When the server is measurably slow, the metrics tell you *how slow*;
`/management/flamegraph` tells you **where the time goes**. The endpoint samples the
whole process with an in-process sampling profiler (the
[`pprof`](https://docs.rs/pprof/latest/pprof/) crate) for a bounded window and
answers with a rendered flamegraph SVG; open it in a browser and read the wide
frames.

```bash
# sample 10 s at 99 Hz (the defaults) and open the result
curl -u admin:… -o flamegraph.svg \
  "http://cdr.internal:9100/management/flamegraph?seconds=10&frequency=99"
```

- `seconds` (default 10) and `frequency` (default 99 Hz) are capped by
  `management.profiling.max_duration` and `max_frequency` (30 and 999 out of the
  box); a request beyond a cap is refused with `400`, never silently clamped.
- **One sample window at a time**: a second request while one runs answers
  `409`; retry when the window completes.
- Sampling is low-overhead but not free; profile under the real load you are
  diagnosing, and keep the endpoint `admin_only` on an internal port like the
  rest of the surface.
- Best results come from the container images and release builds, which keep
  line tables (`debug = "line-tables-only"`) so frames resolve to `file:line`.

With the surface enabled, the viewer grows an **Operations** screen over
it (dependency health, build provenance, the metric registry, and runtime log
control) which appears only while the CDR serves `/management/info`. See
[Viewer → Server](viewer/server.md).

## The manufacturer and the deployment report

Cadasto B.V. is the manufacturer of each tagged release. The running system
names it, with its postal address and single point of contact, on every
surface an operator or a user reads:

| Surface | What it shows |
|---|---|
| startup banner | `Manufactured by Cadasto B.V. · https://www.cadasto.com/contact/`, then the postal address and `info@cadasto.com` |
| `ferroehr --version` | the version (a release build prints the release, any other build appends its commit as `+dev.<commit>`, as `server_version` on `GET /ferroehr/rest/status` does), then `Manufactured by Cadasto B.V., Comeniusstraat 2d, 1817 MS Alkmaar, The Netherlands, info@cadasto.com` and the website |
| `GET /management/info` | a `manufacturer` object with `name`, `postal_address`, `email` and `website` |
| `OPTIONS` on the API base path | `vendor` defaults to `Cadasto B.V.` ([`[api.options]`](installation/config-server.md#apioptions)) |
| viewer | the footer of every screen, and a Manufacturer card on `/system` |
| container images | the `eu.ferroehr.image.manufacturer` label |

`ferroehr report` writes one JSON file naming what a deployment runs: the
version, commit and build date, the enabled features, the specification pins
and `openehr-*` crate versions (the published ones in a release build, the
source tree's in any other, as `openehr_crates_origin` says), the schema
migration level, the deployment
posture, the licence summary, the manufacturer and the effective
configuration with every secret redacted. Attach it to a complaint or a
serious-incident report; see
[`ferroehr report`](installation/config-cli.md#ferroehr-report) for what it
holds and what it leaves out.

## Next

- [Admin & messaging APIs](operations-admin-apis.md): the operator-facing
  HTTP surfaces, route by route.
- [Usage report](usage-report.md): what the daily report to FerroPULSE
  carries, and how to switch it off.
- [Configuration reference](installation/configuration.md): every setting,
  with the server, database and telemetry keys on
  [their own page](installation/config-server.md).
- The API reference at `/ferroehr/rest/swagger-ui` on your own deployment, or
  <https://sandbox.ferroehr.eu/ferroehr/rest/swagger-ui> on the hosted
  sandbox: the document the server itself generates.
