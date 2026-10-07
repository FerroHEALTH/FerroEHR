# Server, database & telemetry

The listener and REST surface (`[server]` and its sub-tables), the PostgreSQL
connection (`[db]`), the two observability sections (`[log]`,
`[telemetry]`), and the usage report (`[usage_report]`). Precedence, the
environment-name grammar, and file discovery are on the
[Configuration reference](configuration.md) index.

<!-- toc -->

## `[server]`

The HTTP listener and REST surface.

```toml
[server]
bind = "0.0.0.0:8080"
base_path = "/ferroehr/rest/openehr/v1"
max_in_flight = 256
swagger_ui = "private"
cors_permissive = false
system_id = "ferroehr.local"
```

| Key | Type | Default | Description |
|---|---|---|---|
| `bind` | string | `0.0.0.0:8080` | Socket address the API listener binds. |
| `base_path` | string | `/ferroehr/rest/openehr/v1` | ITS-REST base path all API routes hang off. Shortening it is supported; see [below](#base_path-shortening-the-rest-base-path). The served OpenAPI document describes whatever paths this setting produces, never the defaults. |
| `max_in_flight` | int | `256` | Concurrent-request admission cap (not a rate). Requests beyond it are shed immediately with `503` + `Retry-After`, never queued, so offered load beyond capacity cannot exhaust memory. `0` installs no shedding layer at all. |
| `swagger_ui` | enum{off,admin_only,private,public} | `private` | Who may read the Swagger UI and the OpenAPI documents under the REST root: `off` does not mount them, `admin_only` needs the admin role, `private` needs any authenticated principal (unauthenticated is `401` with the server's `WWW-Authenticate` challenge, so a browser prompts for the Basic credential), `public` needs nothing. The documents list the whole enabled operation surface, admin and message groups included, so `public` discloses it to anyone who can reach the port; the [hosted sandbox](https://sandbox.ferroehr.eu/ferroehr/rest/swagger-ui) sets it deliberately. With authentication off the guard admits everyone, as the management endpoints do. |
| `cors_permissive` | bool | `false` | Permissive (development) CORS. Left on, any origin may read API responses, so the server warns loudly at boot. Production configures explicit origins at the edge. |
| `system_id` | string | `ferroehr.local` | **This deployment's own openEHR system identifier**; see [below](#system_id-the-data-authoring-identity). Set a stable, deployment-unique name in production (`FERROEHR__SERVER__SYSTEM_ID`). |

The shed sits on the clinical API subtree only, as its outermost layer: a shed
request never reaches authentication, auditing, or the request body, and the
always-on status, health, discovery and management routes are never shed.

### `base_path`: shortening the REST base path

ITS-REST leaves the API base to the deployment and fixes only the version
segment at its end, so you may shorten `base_path`. The shape is checked at
boot, and a value that breaks a rule stops the server with an error naming the
key and every rule it broke:

- The first segment is `ferroehr`. `/ferroehrx/v1` and `/x/ferroehr/v1` are
  refused. This segment is not configurable away.
- The last segment is `v1`, the openEHR API version this server implements.
  The shortest accepted value is therefore `/ferroehr/v1`.
- No trailing slash, no empty segment (`//`), and every segment uses only the
  unreserved URL characters `A-Z a-z 0-9 - . _ ~`.

The status and documentation routes hang off the **REST root**, which the server
derives from `base_path` by dropping the segments that name the openEHR API: the
trailing `v1`, plus an `openehr` segment directly before it when you spell one.

| `base_path` | REST root | Status document |
|---|---|---|
| `/ferroehr/rest/openehr/v1` (default) | `/ferroehr/rest` | `/ferroehr/rest/status` |
| `/ferroehr/openehr/v1` | `/ferroehr` | `/ferroehr/status` |
| `/ferroehr/v1` | `/ferroehr` | `/ferroehr/status` |
| `/ferroehr/cdr/v1` | `/ferroehr/cdr` | `/ferroehr/cdr/status` |

The Swagger UI (`{rest root}/swagger-ui`), the OpenAPI documents
(`{rest root}/api-docs/…`) and the SMART discovery document
(`{rest root}/.well-known/smart-configuration`) move with it. The health family
stays at the process root: `/health`, `/health/liveness`, `/health/readiness`.

Set it from the environment with:

```bash
FERROEHR__SERVER__BASE_PATH=/ferroehr/v1
```

> [!NOTE]
> The `ferroehr healthcheck` subcommand, which the published container images
> run as their Docker health check, derives its default URL from the effective
> configuration (`http://127.0.0.1:<port><REST root>/status`), so it follows a
> shortened base path on its own. An explicit `FERROEHR_HEALTHCHECK_URL` still
> wins, for example to probe `/health/readiness`, which no base path affects.

### Behind a path-prefixed reverse proxy

A proxy that mounts the CDR under a prefix stacks that prefix on top of
`base_path`. A proxy serving the server at `/cdr` with the default base path
gives clients URLs like:

```text
https://cdr.example.org/cdr/ferroehr/rest/openehr/v1/definition/template/adl1.4
```

Two ways to shorten that. Strip the prefix at the proxy, so the server sees the
path it serves (nginx `proxy_pass http://cdr:8080/;` with the trailing slash,
Caddy `handle_path`, Traefik `StripPrefix`). Or shorten the base path:

```bash
FERROEHR__SERVER__BASE_PATH=/ferroehr/v1
```

which gives `https://cdr.example.org/cdr/ferroehr/v1/definition/template/adl1.4`.

Whichever you choose, tell every client. The server builds its `Location`
headers, its served OpenAPI paths and its SMART discovery document from its own
`base_path`, and it cannot see a prefix the proxy adds. The
[FerroEHR Viewer](../viewer/index.md) has its own mirror key, `cdr.base_path`,
which must match.

### `[server.limits]`: request-body sizes

```toml
[server.limits]
body_bytes = 16777216        # 16 MiB
bulk_body_bytes = 67108864   # 64 MiB
```

| Key | Type | Default | Description |
|---|---|---|---|
| `body_bytes` | int | `16777216` | The largest request body the ordinary clinical surface accepts, in bytes. |
| `bulk_body_bytes` | int | `67108864` | The largest body the bulk routes accept: operational-template upload, `/message/import`, `/message/tdd`. |

A request over its tier's limit is refused `413 Payload Too Large` with the
standard openEHR error body. The status is not in the ITS-REST status table; it
is admitted there as an additional, non-conflicting code, and is what
RFC 9110 §15.5.14 defines for this refusal.

The defaults are sized against the measured payloads in the vendored clinical
corpus rather than chosen as round numbers: the clinical tier clears the
largest operational template in that corpus several times over, and the bulk
tier is four times the clinical one, for payloads with no published bound (a
whole-EHR extract, a TDD batch). **Raise `body_bytes` if your compositions
embed large `DV_MULTIMEDIA` data**: a base64 radiology image can exceed either
tier on its own, and that is a deliberate operator decision rather than a
default.

### `[server.rate_limit]`: per-caller request rates

```toml
[server.rate_limit]
enabled = true
principal_per_second = 1024
principal_burst = 2048
address_per_second = 2048
address_burst = 4096
```

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | Whether rate limiting is active. Off allocates no limiter state and costs no per-request check. |
| `principal_per_second` | int | `1024` | Sustained requests per second per authenticated subject, on the clinical API. |
| `principal_burst` | int | `2048` | How far one principal may burst before refusal. |
| `address_per_second` | int | `2048` | Sustained requests per second per client address, across the whole tree. |
| `address_burst` | int | `4096` | How far one address may burst before refusal. |

**This is not `max_in_flight`, and you should be able to tell them apart from
the status alone.** `max_in_flight` protects *capacity*: too many requests in
flight at once, from anyone, and the excess is shed `503` + `Retry-After`.
Rate limiting protects *fairness*: one caller asking too often over time,
refused `429` + `Retry-After`. A `503` means the server is full; a `429` means
you are asking too fast.

Two tiers, because they defend different things. The **address** tier sits
outside authentication, so a flood of unauthenticated requests is refused
before it can make the server verify a signature per request; a limiter must
not itself be the expensive path. The **principal** tier sits inside
authentication, keyed on the authenticated subject, which is the only fair key
for a clinical API: a hospital behind one NAT is a single address, so an
address-keyed clinical limit would throttle an entire site because one client
was busy.

Both defaults sit above this implementation's own measured whole-server
ceiling, so neither tier can refuse a caller until it is asking for more than
the server could have served; below that line, capacity is `max_in_flight`'s
job. **A deployment sized for more than the reference measurement environment
should raise both in proportion.**

Refusals carry the limiter's own `Retry-After` and `x-ratelimit-*` headers
alongside the openEHR error body. `429` is the status RFC 6585 §4 defines for
this refusal, admitted by ITS-REST as an additional, non-conflicting code.

The always-on health family is covered by the address tier only, deliberately:
an orchestrator probe must never be refused because a principal-keyed bucket
was exhausted, and probe rates are nowhere near the address ceiling.

> [!TIP]
> **Benchmarking this server?** Turn the limiter off first, or you will measure
> it instead of the server. The project's own measurement lanes compose an
> overlay that does exactly that, and both instruments refuse to write a record
> if the server answered any `429`.

### `[server.connection]`: bounds before a request exists

```toml
[server.connection]
header_read_timeout_secs = 10
max_concurrent_streams = 256
http2_keep_alive_interval_secs = 30
http2_keep_alive_timeout_secs = 10
```

Every other limit on this page engages once a request has been parsed and
dispatched: body size, the request timeout, the rate limiter, the in-flight
shed. A client that opens a socket and then trickles request headers reaches
none of them, while costing itself almost nothing. This table is where that is
bounded, and HTTP/1 and HTTP/2 need different bounds because the exposure
differs: HTTP/1 streams a request head, so it can be trickled; HTTP/2
multiplexes streams, so the exposure is concurrency.

| Key | Type | Default | Description |
|---|---|---|---|
| `header_read_timeout_secs` | int | `10` | How long a connection may take to deliver a complete HTTP/1 request head, in seconds. `0` disables the bound. Applies to both listeners. |
| `max_concurrent_streams` | int | `256` | The most HTTP/2 streams one connection may have open at once. Bounds the request-setup work a peer can trigger by opening and immediately cancelling streams (the "HTTP/2 Rapid Reset" amplification, CVE-2023-44487). `0` leaves the HTTP library's own default. |
| `http2_keep_alive_interval_secs` | int | `30` | Interval between HTTP/2 keep-alive PINGs, in seconds. `0` disables them, and a peer that vanishes without a FIN is then held until the operating system notices. |
| `http2_keep_alive_timeout_secs` | int | `10` | How long to wait for a keep-alive PING response before closing the connection, in seconds. |

### `system_id`: the data-authoring identity

`system_id` is the identifier this CDR stamps into the data it authors. It
appears on the wire in three places:

- **`EHR.system_id`**, recorded when an EHR is created. The openEHR RM
  (`EHR Information Model`, *EHR Identifier Allocation*) says the
  `EHR.system_id` "should be set to the value that would normally be used for
  locally created EHRs": a value the deployment chooses, not a product
  constant.
- **`AUDIT_DETAILS.system_id`** on every commit for which the client did not
  supply one through the `openehr-audit-details` header. The openEHR REST API
  requires that "when `system_id` is not provided by the client, the server
  MUST set it to its own configured system identifier".
- **`OBJECT_VERSION_ID.creating_system_id`:** the middle segment of every
  version identifier the server mints
  (`<object_id>::<creating_system_id>::<version>`).

Practical notes:

- **The value must be a legal openEHR UID:** a UUID, an ISO OID, or an
  internet id (a reverse-domain / DNS-style name) per the openEHR BASE
  identification grammar. The server judges it with the same validating
  constructor its reader uses and **refuses to boot** on an illegal value,
  because it becomes the `creating_system_id` segment of every version
  identifier the server mints and an illegal value would produce ids the
  server's own reader rejects. A DNS-style name like `cdr.hospital.example` is
  valid; an empty value, or one containing the `::` field separator, is not.
- **Choose it before going live and keep it stable.** The value is stored with
  each EHR and each version; changing it later affects only *newly* authored
  data: existing EHR ids, audit rows and version identifiers are never
  rewritten, and previously issued `OBJECT_VERSION_ID`s stay valid.
- **Make it unique per system**, so data exchanged between openEHR systems
  keeps unambiguous provenance.
- **`system_id` is not `[server.identity]`.** `system_id` says *which system
  authored the data*; [`[server.identity]`](#serveridentity) is the *display*
  identity of the `OPTIONS` System-Options manifest. Rebranding changes the
  manifest and nothing in stored data; changing `system_id` changes what new
  data says about its origin and leaves the manifest alone.

### `[server.tls]`: native TLS and mutual-TLS client authentication

Native TLS termination on the main listener, off by default; deployments
commonly terminate TLS at an ingress. `client_auth = "required"` is the IHE
ATNA ITI-19 mutually-authenticated-node posture (see the
[Audit trail chapter](../audit.md#node-authentication-iti-19-mutual-tls)). The
separate-port management listener always stays plain HTTP, so under
`deployment_profile = "production"` setting `[management] port` opens the
[`plaintext_listener`](configuration.md#deployment_profile) gap.

```toml
[server.tls]
enabled = false
client_auth = "off"
min_version = "1.3"
```

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Terminate TLS natively on the main listener. |
| `cert_file` | path | unset | Server certificate chain (PEM). Required when enabled; a missing, unreadable or certificate-less file stops startup. |
| `key_file` | path | unset | Server private key (PEM). Required when enabled. |
| `client_auth` | enum{off,optional,required} | `off` | Client-certificate policy. `optional` verifies a certificate when one is presented and still accepts connections without; `required` rejects any client without a verified certificate at the handshake. |
| `client_ca_file` | path | unset | The explicit CA bundle client certificates must chain to, never the web PKI. Required unless `client_auth = "off"`. |
| `min_version` | enum{"1.3","1.2"} | `"1.3"` | The lowest TLS version this listener negotiates. |

> [!NOTE]
> `min_version` defaults to **1.3 only**, following the OWASP Transport Layer
> Security Cheat Sheet: web applications must default to TLS 1.3 and may
> support TLS 1.2 for compatibility. Setting `"1.2"` enables 1.2 *alongside*
> 1.3, never instead of it; pick it only for a client that genuinely cannot
> do 1.3, such as an older integration engine or a pinned Java runtime.
> TLS 1.1 and 1.0 are not selectable at all: RFC 8996 deprecates them, and
> neither this key nor the TLS library offers them.

### `[server.identity]`

The System-Options manifest identity (`OPTIONS` on the API base path, e.g.
`OPTIONS /ferroehr/rest/openehr/v1`, the System API's one location). This is
the deployment's *display* identity only; the identifier stamped into authored
data is [`system_id`](#system_id-the-data-authoring-identity).

| Key | Type | Default | Description |
|---|---|---|---|
| `solution` | string | `FerroEHR` | Product name. |
| `solution_version` | string | the build's version | Product version. |
| `vendor` | string | `Cadasto B.V.` | Providing organisation: by default the manufacturer of the release, the name `ferroehr --version` and `GET /management/info` give. |
| `restapi_specs_version` | string | the ITS-REST release this build implements (`1.1.0`) | The openEHR REST API edition advertised. |
| `conformance_profile` | string | the profile the build's recorded conformance verdict earned | Advertised conformance profile. |

The defaults are derived from the build rather than typed into the handler, so
the manifest never out-claims what was actually measured. Override them only to
rebrand.

## `[db]`

PostgreSQL connection.

```toml
[db]
url = "postgres://ferroehr:ferroehr@localhost:5432/ferroehr"
migrate = "apply"
max_connections = 20
min_connections = 2
acquire_timeout_secs = 30
statement_timeout_ms = 60000
```

| Key | Type | Default | Description |
|---|---|---|---|
| `url` | secret URL | `postgres://ferroehr:ferroehr@localhost:5432/ferroehr` | Connection DSN. The default suits a local from-source run against a localhost PostgreSQL, and the server logs a prominent warning at boot while it is in use; **production MUST set it**. Credentials are redacted from every rendering. `DATABASE_URL` is a recognized lower-priority alias. |
| `url_file` | path | unset | Read the DSN from a file instead of the key above, for a mounted secret. Preferred over the environment form in Kubernetes: an environment value is readable through `/proc/<pid>/environ` and inherited by every child process. At most one of the pair, where the built-in development default does not count as "set". |
| `migrate_url` | secret URL | unset | DSN that PREPARES the schema, used for that one boot step and then closed. Preparation spans every schema of a database at once, which no domain-scoped runtime credential can do, so a deployment separating the runtime roles names the credential that can here. It prepares every domain whose DSN reaches the SAME DATABASE — which is the whole of the separated-credential posture, where each domain has a login role of its own on one database; a domain whose DSN reaches a different database is prepared on that DSN. Which it is, is read from the server (`pg_control_system()` plus `current_database()`), never from the DSN text. Unset, it falls back to `url`. See [Operations → Which credential prepares the schema](../operations.md#which-credential-prepares-the-schema). |
| `migrate_url_file` | path | unset | Read `migrate_url` from a file instead, for a mounted secret. At most one of the pair. |
| `migrate` | enum{apply,verify} | `apply` | Whether the server applies its embedded migrations at boot. `apply` is what makes an empty configuration boot against an empty database. `verify` issues **no DDL at all**: it checks that the database already carries exactly this build's migrations and refuses to start otherwise, so the serving DSN can authenticate as a role with no DDL rights. That check still READS every `_sqlx_migrations` table of each database, which no least-privilege role can do, so pair it with `migrate_url` above and with `ferroehr db migrate` run out of band; see [Operations](../operations.md#which-credential-prepares-the-schema). |
| `max_connections` | int | `20` | Pool ceiling. Write-heavy deployments benefit from raising it. |
| `min_connections` | int | `2` | Idle connections kept open, avoiding cold-reopen churn under variable load. |
| `acquire_timeout_secs` | int | `30` | Seconds to wait for a free connection before failing. |
| `statement_timeout_ms` | int | `60000` | `statement_timeout` applied to every pooled connection; `0` leaves the server default. |

`statement_timeout_ms` is the backstop the HTTP request timeout cannot be:
answering a client by dropping the handler future does **not** cancel the
statement PostgreSQL is running, so without it a handful of expensive queries
can hold every pooled connection while every one of their callers has already
given up. Keep it **above** [`query.timeout_ms`](config-integrations.md#query)
so the AQL engine's own typed refusal fires first and this only catches what
the engine does not govern.

## `[storage]`

One DSN per storage domain. Every domain always lives in its own schema
(`clinical`, `party`, `linkage`, `audit`) and every pool carries only its own on
its `search_path`, so no statement can reach another domain's relations without
naming a schema it is not granted. Setting a `url` here adds the CREDENTIAL
separation, and — if the DSN names another host — lets the domain live in a
database or cluster of its own.

```toml
[storage.party]
url_file = "/run/secrets/db-party-dsn"

[storage.linkage]
url_file = "/run/secrets/db-linkage-dsn"
```

| Key | Type | Default | Description |
|---|---|---|---|
| `storage.clinical.url` | secret URL | unset | DSN the clinical domain connects on (role `ferroehr_clinical`). Unset, it uses `db.url`. |
| `storage.party.url` | secret URL | unset | DSN the party (demographic) domain connects on (role `ferroehr_party`). Unset, it uses `db.url`. |
| `storage.linkage.url` | secret URL | unset | DSN the linkage domain connects on (role `ferroehr_linkage`) — the map from a party to the EHR whose subject it is, barred from both domains it joins and they from it. Unset, it uses `db.url`. |
| `storage.audit.url` | secret URL | unset | DSN the audit repository is written through. Unset, it uses `db.url`. |
| `storage.<domain>.url_file` | path | unset | Read that domain's DSN from a file instead, for a mounted secret. At most one of the pair. |

Why a schema is not always enough: a base backup, WAL archiving and physical
replication carry every schema of a database together
([PostgreSQL 18, Backup and Restore](https://www.postgresql.org/docs/18/backup-dump.html)),
so a separation that must survive those is a separation of databases. The Swiss
EPDV Art. 10 Abs. 1 lit. b asks for storage "von anderen Datenbeständen getrennt"
and the DSV Art. 4 Abs. 5 for the log to be kept "getrennt vom System, in welchem
die Personendaten bearbeitet werden"; openEHR reads the same way for the identity
cross-reference, which "could be located on different machines" (BASE
`architecture_overview/master07-security.adoc` §Anonymity). No openEHR spec
governs pools or database roles — this is our own design.

What the server checks at boot:

- a runtime role that can read another domain's relations is refused, always;
- two domains configured on **different** DSNs that turn out to authenticate as
  the **same** database role are refused — a separation that exists only in the
  configuration is worse than none, because it reads as one that holds;
- a missing domain role is a warning under `deployment_profile = "sandbox"` and a
  refusal under `production`, where there are no grants to separate anything
  with.

One constraint on relocation: the `linkage` migration set revokes a function the
`party` set creates, so those two domains are prepared in the same database. A
layout that splits them is refused before anything connects, with the remedy.

## `[log]`

```toml
[log]
format = "auto"
filter = "info,ferroehr=info"
```

| Key | Type | Default | Description |
|---|---|---|---|
| `format` | enum{auto,json,pretty} | `auto` | Stdout rendering. `auto` picks `json` when stdout is not a TTY and the coloured human format when it is; an explicit `pretty` forces colour even through a pipe. |
| `filter` | string | `info,ferroehr=info` | Boot log-filter directives; also the value `/management/loggers` resets to. `RUST_LOG` is a recognized lower-priority alias. |

> [!TIP]
> The ASCII boot banner follows the rendering that is actually installed, not the
> configured word: it prints for `pretty`, and for `auto` only when stdout is a
> terminal. Under `json`, and under `auto` off a terminal (a container, a pipe
> into a log collector) stdout is parseable JSON from the first byte.

## `[telemetry]`

OpenTelemetry export. With `otlp_endpoint` unset the trace export layer is not
installed at all, at zero overhead.

```toml
[telemetry]
service_name = "ferroehr"
environment = "dev"
traces_sample_ratio = 1.0
metrics_push = false
```

| Key | Type | Default | Description |
|---|---|---|---|
| `otlp_endpoint` | secret URL | unset | OTLP/gRPC collector endpoint. Unset ⇒ no trace export. Credentials embedded in it are redacted from every rendering. |
| `service_name` | string | `ferroehr` | The `service.name` resource attribute. |
| `environment` | string | `dev` | The `deployment.environment` resource attribute. |
| `traces_sample_ratio` | float | `1.0` | Head-sampling ratio (`0.1` is a common production start). |
| `metrics_push` | bool | `false` | Also **push** metrics over OTLP, alongside the Prometheus pull surface. Both surfaces are fed by one meter provider, so every instrument reaches both; the push needs `otlp_endpoint` set as well. |
| `flame_file` | path | unset | Span-timing flamegraph capture: write folded stack samples of every span to this file, and render offline with `inferno-flamegraph`. Unset ⇒ the layer is not installed. For diagnostic sessions, not a standing posture: the file grows with span traffic. |

> [!NOTE]
> Instrument names carry no Prometheus suffix; the exporter derives one. A
> counter named `auth_failures` is scraped as `auth_failures_total`, and units
> add `_seconds`/`_bytes`. Over OTLP the unsuffixed name is what a collector
> receives.

The scrape endpoint itself is not opened here; it is
[`management.endpoints.prometheus`](config-auth.md#management), which is `off`
until you name a level.

## `[usage_report]`

The daily installation report to FerroPULSE, the collector Cadasto B.V. runs.
It is **on by default**, and every boot logs one line saying whether it is on.
What a report carries, when it is sent, the network it needs and the privacy
notice are on the [Usage report](../usage-report.md) page.

```toml
[usage_report]
enabled = true
endpoint = "https://report.ferropulse.eu/v1/report"
slow_aql_ms = 1000
deployment = "unknown"
```

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | Send the report. `false` sends nothing and writes nothing to the database for it. Environment: `FERROEHR__USAGE_REPORT__ENABLED`. |
| `endpoint` | secret URL | `https://report.ferropulse.eu/v1/report` | Where reports go. Must be an absolute `https` URL without credentials (refused at boot, and redacted from every rendering); plain `http` is accepted for a loopback host only. Environment: `FERROEHR__USAGE_REPORT__ENDPOINT`. |
| `slow_aql_ms` | int | `1000` | Execution time in milliseconds above which an AQL query counts as slow in the daily report. At least `1`. Environment: `FERROEHR__USAGE_REPORT__SLOW_AQL_MS`. |
| `deployment` | enum{helm,compose,binary,unknown} | `unknown` | How the instance was deployed, as the report states it. The Helm chart sets it to `helm`. Environment: `FERROEHR__USAGE_REPORT__DEPLOYMENT`. |

Under the Helm chart, set the switch with `usageReport.enabled`. The chart
writes `enabled` and `deployment = "helm"` into the rendered file itself and
refuses to render if either is also set under `config.usage_report`;
`endpoint` and `slow_aql_ms` pass through `config.usage_report` unchanged.
