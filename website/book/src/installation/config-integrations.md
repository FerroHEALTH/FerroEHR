# Integrations

Query execution and the four optional integrations: `[query]`, `[events]`,
`[fhir]`, `[terminology]`, `[multimedia]`. Everything except the query knobs is
off by default, and a disabled integration contacts nothing and mounts no
routes. Precedence, the environment-name grammar, and file discovery are on the
[Configuration reference](configuration.md) index.

<!-- toc -->

## `[query]`

AQL execution bounds.

```toml
[query]
plan_cache_capacity = 256
timeout_ms = 30000
max_result_rows = 10000
```

| Key | Type | Default | Description |
|---|---|---|---|
| `plan_cache_capacity` | int | `256` | Maximum distinct cached query plans; `0` disables the cache and every lookup runs the full parse-and-lower path. Cache activity is reported by the `aql_plan_cache_events` counter. |
| `timeout_ms` | int | `30000` | Per-query database execution budget; `0` disables it. Overrun is refused `408`. |
| `max_result_rows` | int | `10000` | The largest page one query execution serves: the page of a query nothing else bounds, and the maximum an explicit `LIMIT` or `fetch` may ask for (a larger page is refused `400`); `0` means unbounded. |

**`timeout_ms` is on by default, and deliberately tighter than
[`db.statement_timeout_ms`](config-server.md#db).** The HTTP request timeout
cannot stand in for it: answering the client by dropping the handler does not
cancel the statement PostgreSQL is running, so overrunning queries would keep
holding pooled connections after their callers had been given up on. Keeping
this budget the tighter of the two means an overrun surfaces as the engine's own
typed `408` rather than a driver error.

**`max_result_rows` is the largest page one execution serves.** Without it,
`SELECT c FROM COMPOSITION c` with no `fetch` generates SQL with no `LIMIT` and
materialises every matching row: one request, unbounded allocation. A query that
nothing else bounds takes the ceiling as its page. An explicit AQL `LIMIT` or a
`fetch` parameter is honoured as written up to the ceiling; a page larger than the
ceiling is refused with `400` naming the ceiling. The refusal is deliberate: a
silently shortened page would let a client that pages with its own `fetch` as the
stride skip the rows between the shortened page and its next `offset`, and the
`RESULT_SET` has nothing to say so. ITS-REST leaves both the `fetch` default and
its maximum to the implementation. A bulk consumer pages with `offset` and a
`fetch` at or below the ceiling, or the operator raises the ceiling deliberately.

## `[events]`

Contribution-outbox eventing to an AMQP broker, plus its admin API. Off by
default; the envelopes are PHI-free by design.

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Spawn the outbox publisher. Together with `fhir.outbound.enabled` it also gates the per-commit outbox INSERT, so with both off the commit path writes no outbox rows at all. |
| `url` | secret URL | `amqp://guest:guest@localhost:5672/%2f` | AMQP broker URL; credentials are redacted from every rendering. |
| `url_file` | path | unset | Read the broker URL from a file instead, for a mounted secret. At most one of the pair, where the built-in development default does not count as "set". |
| `exchange` | string | `ferroehr.events` | Topic exchange for the PHI-free envelope stream. |
| `tls` | bool | `false` | Upgrade an `amqp://` URL to `amqps://` (an already-`amqps://` URL is TLS regardless). |
| `batch_size` | int | `128` | Rows drained per poll. |
| `poll_interval_ms` | int | `1000` | Idle poll interval. |
| `retention_days` | int | `7` | Published-row retention window. |
| `prune_interval_secs` | int | `3600` | Retention-prune cadence. |
| `publish_max_retries` | int | `3` | Per-row publish retries before backing off. |
| `admin_api` | bool | `false` | Mount the `/admin/event_subscription` CRUD routes. |

> [!NOTE]
> Eventing needs the `events` build feature, which is on in the published binary
> and container images. A binary built with `--no-default-features` refuses at
> startup if `events.enabled` is set, rather than running with the publisher
> silently absent.

## `[fhir]`

The FHIR connector: an inbound façade and an independent outbound emitter.

`[fhir]`: `api_enabled` (bool, `false`) mounts `/fhir/r4/*` plus the
`/admin/fhir_mapping` CRUD; the routes answer `404` while it is off.

`[fhir.outbound]`:

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Emit mapped FHIR resources to the broker. |
| `url` | secret URL | `amqp://guest:guest@localhost:5672/%2f` | AMQP broker URL; credentials redacted. |
| `url_file` | path | unset | Read the broker URL from a mounted file instead. At most one of the pair. |
| `exchange` | string | `ferroehr.fhir` | Topic exchange, deliberately distinct from the events exchange, for PHI isolation. |
| `tls` | bool | `false` | Upgrade `amqp://` to `amqps://`. |
| `batch_size` | int | `128` | Outbox rows scanned per poll. |
| `poll_interval_ms` | int | `1000` | Idle poll interval. |
| `publish_max_retries` | int | `3` | Per-message publish retries before backing off. |

> [!WARNING]
> The outbound stream carries **PHI**: its payload *is* the mapped FHIR
> resource. That is why it is a separate switch and a separate exchange from the
> PHI-free change-event stream: broker-level access control can then restrict
> the PHI-bearing stream on its own. Enable it only against a TLS,
> access-controlled broker.

## `[terminology]`

The terminology extension API and external FHIR terminology servers.

`[terminology]`: `api_enabled` (bool, `false`) mounts the terminology extension
API.

`[terminology.external]`:

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Materialise the configured providers. With it off, no remote provider is built and validation stays on the in-process openEHR terminology bundle. |
| `fail_on_error` | bool | `false` | What an unresolvable terminology lookup does to a commit: `false` accepts it (fail-open), `true` rejects it. |

Enabling `[terminology.external]` with no provider configured is a boot error.

`[terminology.external.providers.<name>]`, conventionally at least `default`:

| Key | Type | Default | Description |
|---|---|---|---|
| `type` | enum{fhir} | `fhir` | Server kind. Only FHIR R4B is supported. |
| `url` | secret URL | required | The server's FHIR base URL. Empty is a boot error. Credentials embedded in it are redacted from every rendering. |
| `operation` | enum{validate_code,expand} | `validate_code` | The membership operation. `validate_code` is a direct yes/no with the least payload; `expand` plus a membership test is the fallback for servers without `$validate-code`. |
| `connect_timeout_ms` | int | `2000` | TCP connect timeout. |
| `request_timeout_ms` | int | `10000` | Overall request timeout. |
| `oauth2_client` | string | unset ⇒ unauthenticated | Names an entry under `[terminology.external.oauth2_clients]`. A name with no such entry is a boot error. |
| `client_cert_path` / `client_key_path` | path | unset | The mutual-TLS client identity; see [below](#mutual-tls-to-a-terminology-server). |
| `ca_bundle_path` | path | unset | The trust anchors this server's certificate is verified against; see [below](#mutual-tls-to-a-terminology-server). |
| `cache_ttl_secs` | int | `300` | TTL of the per-provider response cache; a repeated validate/expand/subsumes/lookup within the window is served locally instead of one HTTPS round trip per validated code. `0` disables it. |
| `cache_capacity` | int | `10000` | Maximum cached responses per provider. |

Cached entries are the *decoded* responses, not raw JSON: a server answer that
is not a valid FHIR R4B `Parameters`/`ValueSet` resource (for example an
`$expand` result missing the required `ValueSet.status` or
`expansion.timestamp`) is treated as an upstream fault rather than partially
read, so it takes the same path as an unreachable server and `fail_on_error`
decides what the commit does.

> [!NOTE]
> External terminology servers need the `fhir` build feature, which is on in the
> published binary and container images. A binary built with
> `--no-default-features` refuses at startup if `terminology.external` is
> enabled with any provider configured; the in-process openEHR terminology
> bundle remains available.

### Several terminology servers at once

**Every** entry under `[terminology.external.providers]` is materialised at
startup, so one instance can serve SNOMED CT from one server and LOINC or ICD
from others. `[terminology.external.routes]` maps a terminology to the provider
that answers for it: the key is a terminology id (`SNOMED-CT`) or a system URI
(`http://snomed.info/sct`), matched case-insensitively as a whole string, and
the value names a provider. A terminology with no route goes to the provider
named `default`, or to the sole configured provider when there is exactly one. A
route naming a provider that does not exist is a startup error, because a
dangling route would otherwise degrade silently into "ask the default server".

```toml
[terminology.external]
enabled = true
fail_on_error = false

[terminology.external.providers.default]
type = "fhir"
url = "https://r4.ontoserver.csiro.au/fhir"

[terminology.external.providers.snomed]
type = "fhir"
url = "https://snowstorm.example.org/fhir"
oauth2_client = "ts-client"

[terminology.external.routes]
"SNOMED-CT" = "snomed"
"http://snomed.info/sct" = "snomed"
"http://loinc.org" = "default"
```

Routing applies everywhere terminology is consulted: the `/terminology/*`
extension API, AQL `TERMINOLOGY(…)` resolution, and the composition-commit
binding checks below.

### Authenticating to a terminology server

`[terminology.external.oauth2_clients.<name>]` configures an OAuth2
client-credentials client; a provider references it by name with
`oauth2_client`. The access token is cached and re-requested shortly before it
expires, so a validation burst costs one token request per token lifetime.

| Key | Type | Default | Description |
|---|---|---|---|
| `token_url` | secret URL | required | The OAuth2 token endpoint. Empty is a boot error. Credentials embedded in it are redacted from every rendering. |
| `client_id` | string | required | The registered client identifier. Empty is a boot error. |
| `client_secret` / `client_secret_file` | secret / path | one is required | The client secret, inline or read from a mounted file. |
| `scopes` | list of string | `[]` | Scopes requested with the client-credentials grant. |
| `refresh_leeway_secs` | int | `30` | How long before stated expiry the token is renewed. |
| `auth_method` | enum{client_secret_basic,client_secret_post} | `client_secret_basic` | How the client authenticates at the token endpoint. |

```toml
[terminology.external.oauth2_clients.ts-client]
token_url = "https://idp.example.org/realms/ts/protocol/openid-connect/token"
client_id = "ferroehr-cdr"
client_secret_file = "/run/secrets/ts-client"
scopes = ["system/*.read"]
```

### Mutual TLS to a terminology server

A terminology server that authenticates its clients with certificates instead of
(or in addition to) a bearer token is configured **per provider**, because a
client certificate is issued by that server's PKI: a deployment enrolled with a
national SNOMED CT service, a commercial value-set server and an in-house FHIR
server holds three different certificates. Repeat the same paths in each
provider table if one identity really does serve them all.

| Key | Meaning |
|---|---|
| `client_cert_path` | PEM file with the client certificate (optionally a chain) presented to this server. |
| `client_key_path` | PEM file with that certificate's private key. |
| `ca_bundle_path` | PEM bundle of the trust anchors this server's certificate is verified against. |

```toml
[terminology.external.providers.snomed]
type = "fhir"
url = "https://snowstorm.example.org/fhir"
client_cert_path = "/run/secrets/ts-snomed-client.crt.pem"
client_key_path = "/run/secrets/ts-snomed-client.key.pem"
ca_bundle_path = "/run/secrets/ts-snomed-ca.pem"
```

`client_cert_path` and `client_key_path` are set together; one without the
other is a startup error, never a connection that silently presents no
certificate. Unreadable files, a certificate file with no certificate in it and
a key file with no key in it are startup errors too, so a broken identity never
waits until the first validated code to surface.

`ca_bundle_path` **replaces** the default trust anchors for that provider, so a
terminology server issued by a private PKI is pinned to that PKI instead of also
accepting the whole public web PKI. Leave it unset to use the platform's default
trust store.

> [!WARNING]
> There is no option to disable certificate verification. Server-certificate and
> hostname verification are always on for every provider; `ca_bundle_path`
> changes *which* anchors are trusted, never *whether* the server is verified.

The client identity applies to the connection to the terminology server itself.
An OAuth2 token endpoint (`oauth2_client`) is a different host in a different
trust domain and keeps the default TLS stack.

Kubernetes deployments mount the PEM files with the chart's `config.files` map,
which materialises them under `/etc/ferroehr/`.

### Archetype value-set bindings at commit

With `[terminology.external]` enabled, committing a COMPOSITION also resolves
the archetype **constraint bindings** its template declares: where a template
binds an `ac` code to an external terminology query, the coded value in the
composition must be a member of the value set that query returns. The query goes
to the server the binding's terminology routes to.

- The code is in the value set → the commit proceeds.
- The code is **not** in the value set → `422` naming the path, the code, and
  the bound query. That is a real constraint violation, not a service failure,
  so `fail_on_error` does not change it.
- The value set could **not be resolved** (server down, error response, unknown
  value set, no provider routes to that terminology) → `fail_on_error` decides:
  `false` (the default) accepts the commit and logs a warning; `true` rejects it
  with `422`.

With `[terminology.external]` disabled (the default) no binding is resolved
and no request is made, so commit behaviour is exactly as if this section did
not exist.

> [!NOTE]
> The composition's `terminology_id` is sent verbatim as the FHIR `system`
> parameter, and no openEHR specification defines a mapping between
> `terminology_id` values (`SNOMED-CT`) and FHIR system URIs
> (`http://snomed.info/sct`). If your archetypes and your terminology server
> disagree, align them in the terminology-server configuration; the CDR does
> not rewrite the value.

## `[multimedia]`

`DV_MULTIMEDIA` externalization to an S3-compatible object store. Off by
default: blobs stay inline, byte-identical, and no object store is built or
contacted.

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Externalize large multimedia data. |
| `threshold_bytes` | int | `262144` (256 KiB) | Decoded size strictly above which data is offloaded; at or below it stays inline. |
| `endpoint` | secret URL | unset ⇒ default AWS endpoint resolution | S3-compatible endpoint. Must be an absolute `http`/`https` URL when set. Credentials embedded in it are redacted from every rendering. |
| `bucket` | string | `openehr-multimedia` | Target bucket for content-addressed blobs. |
| `region` | string | `us-east-1` | AWS region; S3 requires one even for non-AWS endpoints. |
| `access_key_id` | string | unset | S3 access key id. Unset, with no secret key either, runs the client anonymously. |
| `secret_access_key` / `secret_access_key_file` | secret / path | unset | S3 secret access key. At most one of the pair. |
| `allow_http` | bool | `false` | Allow plain-HTTP endpoints, development only; production S3 is HTTPS. |

An enabled integration whose `endpoint` is set but blank, not an absolute URL,
or carries a scheme other than `http`/`https` is a **boot error**. That case is
easy to reach by accident (an unset Compose variable expanding to nothing, or
an empty Helm value) and used to boot cleanly and then fail on the first
multimedia commit, so it is refused where an operator can still act on it.

> [!WARNING]
> Offloaded blobs are PHI. The bucket must be private and encrypted, and reached
> over HTTPS. Multimedia externalization also needs the `multimedia` build
> feature (on in the published binary and container images); a slim build
> refuses at startup rather than silently keeping blobs inline.
