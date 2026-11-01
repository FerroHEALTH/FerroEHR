# Server, database & telemetry

The listener (`[server]`), the REST surface (`[rest]`), the PostgreSQL
connection (`[db]`), the observability section (`[telemetry]`), and the
usage report (`[usage_report]`). Precedence, the
environment-name grammar, and file discovery are on the
[Configuration reference](configuration.md) index.

<!-- toc -->

## `[server]`

The HTTP listener: the section every FerroHEALTH product shares
(`ferrohealth-server`), so the keys and their behaviour are the same in each.
It carries the stack every request passes through: the server's request id,
the request timeout, the body limit, the concurrency bound, compression, the
request log, the health routes and the bounded drain on `SIGTERM`.

```toml
[server]
listen = "0.0.0.0:8080"
base_path = ""
request_timeout_ms = 30000
header_read_timeout_ms = 30000
idle_timeout_ms = 120000
body_limit_bytes = 67108864
drain_delay_ms = 0
shutdown_timeout_ms = 10000
# max_concurrent_requests = 256
retry_after_s = 1
trusted_proxies = []
forwarded_header = "forwarded"
compression = true
compression_min_bytes = 1024
```

| Key | Type | Default | Description |
|---|---|---|---|
| `listen` | socket address | `0.0.0.0:8080` | The IP address and port the listener binds: every interface at the port the container image exposes. A host name is refused at startup. `PORT` alone gives `0.0.0.0:<PORT>`, and `FERROEHR__SERVER__LISTEN` wins over both. Set `127.0.0.1:8080` to keep a bare binary on the loopback. |
| `base_path` | string | empty | The path every route is served under, the health routes included. Empty serves them at the process root. The openEHR API path is [`rest.base_path`](#rest). |
| `request_timeout_ms` | int | `30000` | A request still running after this long is answered `408` with the openEHR error body. |
| `header_read_timeout_ms` | int | `30000` | How long a client may take to send its request head, counted from accept for a connection's first request and from the first byte of each later request on a kept-alive connection; a client that has not sent it by then, one that sent nothing after connecting included, is disconnected. Bounds the slow-header and the silent connection, which no other limit reaches. `0` is refused. |
| `idle_timeout_ms` | int | `120000` | How long a kept-alive HTTP/1.1 connection may wait for its next request; past it the server closes the connection with a clean end of stream. Keep it longer than the idle timeout of your clients (Go's `http.DefaultTransport` keeps an idle connection 90 s) and of any load balancer, ingress controller or proxy in front, so they close an idle connection first and never send a request into one the server is closing. `0` is refused. |
| `body_limit_bytes` | int | `67108864` (64 MiB) | The largest request body any route reads, refused `413` above it. The bulk routes (operational-template upload, `/message/import`) take up to this; [`rest.body_limit_bytes`](#rest) narrows the rest of the clinical surface below it. The largest operational template in the vendored CKM corpus is several megabytes, well below this ceiling. |
| `drain_delay_ms` | int | `0` | On `SIGTERM` or `SIGINT`, readiness answers `503` this long while the listener still accepts, so an orchestrator stops routing first. |
| `shutdown_timeout_ms` | int | `10000` | Then the listener closes and the requests in flight get this long to finish; a connection still open after it is dropped. |
| `max_concurrent_requests` | int | unset | The requests served at once. Past it a request is answered `503` with `Retry-After` and the openEHR error body, never queued, so offered load beyond capacity cannot exhaust memory. The health routes are never refused. Unset, there is no bound. |
| `retry_after_s` | int | `1` | The `Retry-After` of a `503` or `429` the listener writes. |
| `trusted_proxies` | list | `[]` | The reverse proxies, as addresses or CIDR blocks, whose forwarded client address is believed. Empty, every request is named by its peer address. |
| `forwarded_header` | enum{forwarded,x-forwarded-for} | `forwarded` | The header a trusted proxy names the client in (`Forwarded` is RFC 7239). |
| `compression` | bool | `true` | Compress a response with gzip or Brotli when the client accepts one. |
| `compression_min_bytes` | int | `1024` | The smallest response body that is compressed. |

A refusal the listener writes itself (`408`, `413`, `429`, `503`) and a handler
that panics (`500`) answer the standard openEHR `{ error, message }` body. `413`
and `429` are not in the ITS-REST status table; ITS-REST admits them as
additional, non-conflicting codes, and they are what RFC 9110 §15.5.14 and
RFC 6585 §4 define for these refusals. A panic's message reaches neither the
body nor the log.

Every request gets a request id the server mints, which the request log and the
access record carry and the response returns in `X-Request-Id`. A client's own
`X-Request-Id` is returned to it unchanged and never recorded.

### `[server.caller_rate]`: per-caller request rates

```toml
[server.caller_rate]
requests_per_second = 1024
burst = 2048
```

| Key | Type | Default | Description |
|---|---|---|---|
| `requests_per_second` | int | (required in the table) | Sustained requests per second one authenticated subject may send on the clinical API. |
| `burst` | int | (required in the table) | How many requests one subject may send at once before the rate applies. |

Unset, no caller is limited. The chart and the Compose quickstart set the values
above.

**This is not `max_concurrent_requests`, and you can tell them apart from the
status alone.** `max_concurrent_requests` protects *capacity*: too many
requests in flight at once, from anyone, and the excess is refused `503` +
`Retry-After`. The caller rate protects *fairness*: one caller asking too often
over time, refused `429` + `Retry-After`. A `503` means the server is full; a
`429` means you are asking too fast.

The limit is keyed on the authenticated subject, the only fair key for a
clinical API: a hospital behind one NAT is a single address. It sits inside
authentication, so the health routes, the status document and the discovery
document are never refused by it.

> [!TIP]
> **Benchmarking this server?** Lift the caller rate first, or you will measure
> it instead of the server. The project's own measurement lanes compose an
> overlay that does exactly that, and both instruments refuse to write a record
> if the server answered any `429`.

### `[server.tls]`: native TLS and mutual-TLS client authentication

Native TLS termination on the main listener, unset by default; deployments
commonly terminate TLS at an ingress. `client_auth = "required"` is the IHE
ATNA ITI-19 mutually-authenticated-node posture (see the
[Audit trail chapter](../audit.md#node-authentication-iti-19-mutual-tls)). The
separate-port management listener always stays plain HTTP, so under
`deployment_profile = "production"` setting `[management] port` opens the
[`plaintext_listener`](configuration.md#deployment_profile) gap.

```toml
[server.tls]
certificate_file = "/etc/ferroehr/server.pem"
key_file = "/etc/ferroehr/server.key"
client_auth = "none"
min_version = "1.3"
# reload_interval_s = 60
```

| Key | Type | Default | Description |
|---|---|---|---|
| `certificate_file` | path | (required in the table) | Server certificate chain (PEM), the leaf first. A missing, unreadable or certificate-less file stops startup. |
| `key_file` | path | (required in the table) | Server private key (PEM), read as a secret file. A key that does not belong to the leaf stops startup. |
| `client_auth` | enum{none,optional,required} | `none` | Client-certificate policy. `optional` verifies a certificate when one is presented and still accepts connections without; `required` rejects any client without a verified certificate at the handshake. |
| `client_ca_file` | path | unset | The explicit CA bundle client certificates must chain to, never the web PKI. Required when `client_auth` is `optional` or `required`, and refused at load when it is `none`, since a listener under `none` asks no client certificate. |
| `min_version` | enum{"1.3","1.2"} | `"1.3"` | The lowest TLS version this listener negotiates. `"1.0"` and `"1.1"` are refused when the configuration is read (RFC 8996). |
| `reload_interval_s` | int | unset | How often the certificate, key and client CA are read again while the server runs. The files are always read again on `SIGHUP`. A changed pair is served, and a changed client CA verifies, from the next handshake on without a restart; a read that fails is logged and the running configuration stays. |
| `healthcheck_identity_file` | path | unset | The client certificate chain and key, in one PEM, that `ferroehr healthcheck` presents to a listener requiring one. |

Before 5.5.0, `client_ca_file` alone meant that every client presents a
certificate. From 5.5.0, `client_auth` decides it and defaults to `none`, so a
configuration carried over from an earlier release set `client_ca_file` and no
`client_auth`, and 5.5.0 served it without asking a client certificate. Such a
configuration is now refused at load. Set `client_auth = "required"` beside
`client_ca_file` to keep mutual TLS (GHSA-52mf-qg39-4q4j).

`ferroehr healthcheck` speaks TLS to a TLS listener over the loopback and
accepts exactly the leaf certificate in `certificate_file`.

> [!NOTE]
> `min_version` defaults to **1.3 only**, as RFC 9325 §3.1.1 has it supported.
> Setting `"1.2"` enables 1.2 *alongside* 1.3, with the four forward-secret
> suites RFC 9325 §4.2 recommends; pick it only for a client that cannot do
> 1.3, such as an older integration engine or a pinned Java runtime.

> [!NOTE]
> An EC certificate must name its curve. RFC 5480 §2.1.1 forbids explicit
> curve parameters, TLS clients refuse such a certificate, and the server
> refuses the pair at startup. LibreSSL (the `openssl` that ships with macOS)
> writes explicit parameters from `genpkey` and `req -newkey ec` unless you add
> `-pkeyopt ec_param_enc:named_curve`; `openssl ecparam -name prime256v1
> -genkey` writes a named curve.

## `[rest]`

The openEHR REST surface the listener serves.

```toml
[rest]
base_path = "/ferroehr/rest/openehr/v1"
swagger_ui = "private"
cors_permissive = false
system_id = "ferroehr.local"
# body_limit_bytes = 16777216
```

| Key | Type | Default | Description |
|---|---|---|---|
| `base_path` | string | `/ferroehr/rest/openehr/v1` | ITS-REST base path all API routes hang off. Shortening it is supported; see [below](#base_path-shortening-the-rest-base-path). The served OpenAPI document describes whatever paths this setting produces, never the defaults. |
| `swagger_ui` | enum{off,admin_only,private,public} | `private` | Who may read the Swagger UI and the OpenAPI documents under the REST root: `off` does not mount them, `admin_only` needs the admin role, `private` needs any authenticated principal (unauthenticated is `401` with the server's `WWW-Authenticate` challenge, so a browser prompts for the Basic credential), `public` needs nothing. The documents list the whole enabled operation surface, admin and message groups included, so `public` discloses it to anyone who can reach the port; the [hosted sandbox](https://sandbox.ferroehr.eu/ferroehr/rest/swagger-ui) sets it deliberately. With authentication off the guard admits everyone, as the management endpoints do. |
| `cors_permissive` | bool | `false` | Permissive (development) CORS. Left on, any origin may read API responses, so the server warns loudly at boot. Production configures explicit origins at the edge. |
| `system_id` | string | `ferroehr.local` | **This deployment's own openEHR system identifier**; see [below](#system_id-the-data-authoring-identity). Set a stable, deployment-unique name in production (`FERROEHR__REST__SYSTEM_ID`). |
| `body_limit_bytes` | int | unset | The largest body the ordinary clinical surface accepts, below [`server.body_limit_bytes`](#server), which the bulk routes keep. Unset, every route takes the listener's limit. A value at or above the listener's is refused at startup. The largest example composition in the vendored corpus is 526 KB; raise it if your compositions embed large `DV_MULTIMEDIA` data. |

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
is served under `server.base_path`, which is empty by default, so it stays at the
process root: `/health`, `/health/liveness`, `/health/readiness`.

Set it from the environment with:

```bash
FERROEHR__REST__BASE_PATH=/ferroehr/v1
```

> [!NOTE]
> The `ferroehr healthcheck` subcommand, which the published container images
> run as their Docker health check, asks `GET /health/readiness` on the
> `server.listen` port over the loopback, so `rest.base_path` does not move it.

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
FERROEHR__REST__BASE_PATH=/ferroehr/v1
```

which gives `https://cdr.example.org/cdr/ferroehr/v1/definition/template/adl1.4`.

Whichever you choose, tell every client. The server builds its `Location`
headers, its served OpenAPI paths and its SMART discovery document from its own
`rest.base_path`, and it cannot see a prefix the proxy adds. The
[FerroEHR Viewer](../viewer/index.md) has its own mirror key, `cdr.base_path`,
which must match.

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
  keeps unambiguous provenance. A node of a federation must hold a
  `system_id` no other member uses (Federation Tier N42a); the default
  `ferroehr.local` is the same on every node that leaves it unset. See
  [FerroEHR as a federation node](../beyond-core/federation-node.md#admission-n42a).
- **`system_id` is not `[rest.identity]`.** `system_id` says *which system
  authored the data*; [`[rest.identity]`](#restidentity) is the *display*
  identity of the `OPTIONS` System-Options manifest. Rebranding changes the
  manifest and nothing in stored data; changing `system_id` changes what new
  data says about its origin and leaves the manifest alone.

### `[rest.identity]`

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

The defaults are derived from the build rather than typed into the handler.
Override them only to rebrand. The manifest carries no `conformance_profile`:
the ITS-REST `Options` schema makes that member optional, and FerroEHR leaves
it out because no conformance record it publishes assigns a profile.

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
| `migrate_url` | secret URL | unset | DSN that PREPARES the schema, used for that one boot step and then closed. Preparation spans every schema of a database at once, which no domain-scoped runtime credential can do, so a deployment separating the runtime roles names the credential that can here. It prepares every domain whose DSN reaches the SAME DATABASE — which is the whole of the separated-credential posture, where each domain has a login role of its own on one database; a domain whose DSN reaches a different database is prepared on that database's `storage.migrate.<domain>.url` (on the domain's own DSN when that is unset). Which it is, is read from the server (`pg_control_system()` plus `current_database()`), never from the DSN text. Unset, it falls back to `url`. See [Operations → Which credential prepares the schema](../operations.md#which-credential-prepares-the-schema). |
| `migrate_url_file` | path | unset | Read `migrate_url` from a file instead, for a mounted secret. At most one of the pair. |
| `migrate` | enum{apply,verify} | `apply` | Whether the server applies its embedded migrations at boot. `apply` is what makes an empty configuration boot against an empty database. `verify` issues **no DDL at all**: it checks that the database already carries exactly this build's migrations and refuses to start otherwise, so the serving DSN can authenticate as a role with no DDL rights. That check still READS every `_sqlx_migrations` table of each database, which no least-privilege role can do, so pair it with `migrate_url` above and with `ferroehr db migrate` run out of band; see [Operations](../operations.md#which-credential-prepares-the-schema). |
| `max_connections` | int | `20` | Pool ceiling. Write-heavy deployments benefit from raising it. |
| `min_connections` | int | `2` | Idle connections kept open, avoiding cold-reopen churn under variable load. |
| `acquire_timeout_secs` | int | `30` | Seconds to wait for a free connection before failing. |
| `statement_timeout_ms` | int | `60000` | `statement_timeout` applied to every pooled connection; `0` leaves the server default. |

`statement_timeout_ms` is the backstop for every statement. An AQL query
(ad hoc or stored) and the composition-count statistics stop sooner when their
caller goes away: a client that disconnects, or the AQL engine's own
[`query.timeout_ms`](config-integrations.md#query), closes the statement's
connection, its pool slot frees at once, and every pooled session sets
PostgreSQL's `client_connection_check_interval` to one second, so the backend
stops the statement within about a second. Other statements run to their end
or to this timeout, so keep it **above** `query.timeout_ms`: the AQL engine's
typed refusal then fires first and this only catches what the engine does not
govern.

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

Each database is prepared on its own migration credential. `[db] migrate_url`
names one database; a domain whose DSN reaches another database is prepared
there, and verified there under `migrate = "verify"`, on the credential named in
`[storage.migrate.<domain>]`:

```toml
[storage.linkage]
url_file = "/run/secrets/db-linkage-dsn"           # postgres://ferroehr_linkage:…@linkage-db/…
[storage.migrate.linkage]
url_file = "/run/secrets/db-linkage-migrate-dsn"   # postgres://ferroehr_migrator:…@linkage-db/…
```

| Key | Type | Default | Description |
|---|---|---|---|
| `storage.migrate.<domain>.url` | secret URL | unset | DSN that prepares the database the domain lives in, when that is not the database `db.migrate_url` reaches. Used for that one step and closed again. Unset, such a database is prepared on the domain's own `url`, which a DML-only runtime role cannot do: it is refused on the first bookkeeping table it cannot read. |
| `storage.migrate.<domain>.url_file` | path | unset | Read it from a file instead, for a mounted secret. At most one of the pair. |

The server refuses to start when a domain's `url` and its `storage.migrate`
URL reach different databases. A database that holds the `linkage` domain
without the `party` domain also receives the party migration set, its schema
and grants and never a row, because the linkage set revokes a function the
party set creates.

## `[telemetry]`

The console, the request log and the OpenTelemetry export: the section every
FerroHEALTH product shares (`ferrohealth-telemetry`). With no
`[telemetry.otlp]` table nothing is exported.

```toml
[telemetry]
format = "auto"
filter = "info,hyper=warn,tower=warn,h2=warn"
logged_query_parameters = []
# environment = "production"

# [telemetry.otlp]
# endpoint = "http://collector:4317"
# traces = true
# metrics = true
# sample_ratio = 1.0
```

| Key | Type | Default | Description |
|---|---|---|---|
| `format` | enum{auto,json,pretty} | `auto` | Stdout rendering. `auto` picks `json` when stdout is not a TTY and the human format when it is. `NO_COLOR` turns colour off. |
| `filter` | string | `info,hyper=warn,tower=warn,h2=warn` | Console filter directives, refused at boot when they do not parse; also the value `/management/loggers` resets to. `RUST_LOG` is a lower-priority fallback. The filter decides what the console writes and never thins a trace. |
| `logged_query_parameters` | list of strings | `[]` | The query parameters whose values the request log may carry, each cut to 64 characters. Empty, the request log carries no query value. |
| `environment` | string | unset | The `deployment.environment.name` resource attribute of every span and metric. |
| `otlp.endpoint` | secret URL | (required in the table) | OTLP/gRPC collector endpoint; `https://` speaks TLS. Credentials in it are masked in every rendering. |
| `otlp.traces` | bool | `true` | Export FerroEHR's spans. |
| `otlp.metrics` | bool | `true` | Push the metrics over OTLP beside the Prometheus pull surface. One meter provider feeds both. |
| `otlp.sample_ratio` | float | `1.0` | The share of traces sampled (`0.1` is a common production start). A client's `traceparent` is never read: every request starts a trace of the server's own. |
| `otlp.trust_roots_file` | path | unset | PEM trust roots the collector's certificate may chain to, beside the platform's. |
| `otlp.client_identity_file` | path | unset | PEM client certificate chain and key, for mutual TLS with the collector. |

The `service.name` of every span and metric is `ferroehr`, and
`service.version` is the release and its commit.

The request log writes one line per request: the method, the matched route
template, the status, the latency and the server's request id, never a body,
the raw path or a header value. It records the `http.server.request.duration`
histogram, scraped as `http_server_request_duration_seconds` with the
`http_request_method`, `http_route`, `http_response_status_code` and
`url_scheme` labels.

> [!TIP]
> The ASCII boot banner follows the rendering that is actually installed, not the
> configured word: it prints for `pretty`, and for `auto` only when stdout is a
> terminal. Under `json`, and under `auto` off a terminal (a container, a pipe
> into a log collector) stdout is parseable JSON from the first byte.

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
deployment = "unknown"
```

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | Send the report. `false` sends nothing and writes nothing to the database for it. In a CI run (`GITHUB_ACTIONS` or `CI` set to `true` or `1`) the report is off whatever this says. Environment: `FERROEHR__USAGE_REPORT__ENABLED`. |
| `endpoint` | URL | `https://report.ferropulse.eu/v1/report` | Where reports go. Must be an absolute `https` URL without credentials; plain `http` is accepted for a loopback host only. Anything else is refused when the configuration is read. Environment: `FERROEHR__USAGE_REPORT__ENDPOINT`. |
| `deployment` | enum{helm,compose,binary,unknown} | `unknown` | How the instance was deployed, as the report states it. The Helm chart sets it to `helm`. Environment: `FERROEHR__USAGE_REPORT__DEPLOYMENT`. |
| `state_dir` | path | unset | Refused. The section is the one every FerroHEALTH product shares, where this key names the directory of the instance id; FerroEHR keeps the instance id and the counts in the clinical database, so every replica reports as one installation. |

`slow_aql_ms` is no longer a key: the report carries no AQL series. A
configuration that still sets it is refused at boot; remove the line.

Under the Helm chart, set the switch with `usageReport.enabled`. The chart
writes `enabled` and `deployment = "helm"` into the rendered file itself and
refuses to render if either is also set under `config.usage_report`;
`endpoint` passes through `config.usage_report` unchanged.
