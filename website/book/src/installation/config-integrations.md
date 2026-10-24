# Integrations

Query execution and the three optional integrations: `[query]`, `[events]`,
`[terminology]`, `[multimedia]`. Everything except the query knobs is off by
default, and a disabled integration contacts nothing and mounts no
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
| `enabled` | bool | `false` | Spawn the outbox publisher. It also gates the per-commit outbox INSERT, so with it off the commit path writes no outbox rows at all. |
| `url` | secret URL | `amqp://guest:guest@localhost:5672/%2f` | AMQP broker URL; credentials are redacted from every rendering. |
| `url_file` | path | unset | Read the broker URL from a file instead, for a mounted secret. At most one of the pair, where the built-in development default does not count as "set". |
| `exchange` | string | `ferroehr.events` | Topic exchange for the PHI-free envelope stream. |
| `tls` | bool | `false` | Upgrade an `amqp://` URL to `amqps://` (an already-`amqps://` URL is TLS regardless). Under `deployment_profile = "production"`, an enabled outbox that still publishes over `amqp://` opens the [`plaintext_broker`](configuration.md#deployment_profile) gap. |
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

## `[terminology]`

The terminology extension API and the one FHIR terminology server the CDR asks.

`[terminology]`: `api_enabled` (bool, `false`) mounts the terminology extension
API.

`[terminology.server]`:

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Build the client. With it off, nothing is requested and validation stays on the in-process openEHR terminology bundle. |
| `url` | secret URL | required when enabled | The server's FHIR R4B base URL, for FerroTERM `http://ferroterm:8080/r4b`. Empty while `enabled` is a boot error. Credentials embedded in it are redacted from every rendering. |
| `fail_on_error` | bool | `false` | What an unresolvable binding does to a commit: `false` accepts it (fail-open), `true` rejects it. |
| `request_timeout_ms` | int | `10000` | How long one request may take, connection included. |

Every key is a scalar, so the whole section is reachable through the
environment: `FERROEHR__TERMINOLOGY__SERVER__ENABLED`,
`FERROEHR__TERMINOLOGY__SERVER__URL`,
`FERROEHR__TERMINOLOGY__SERVER__FAIL_ON_ERROR`,
`FERROEHR__TERMINOLOGY__SERVER__REQUEST_TIMEOUT_MS`.

```toml
[terminology.server]
enabled = true
url = "http://ferroterm:8080/r4b"
fail_on_error = false
```

The server answers every terminology the bundled openEHR terminology does not
carry: the `/terminology/*` extension API, AQL `TERMINOLOGY(…)` resolution, the
ADL 2 binding check and the composition-commit binding checks below. A response
that is not a valid FHIR R4B `Parameters` or `ValueSet` resource (for example an
`$expand` result missing the required `ValueSet.status` or
`expansion.timestamp`) is treated as an upstream fault rather than partially
read, so it takes the same path as an unreachable server and `fail_on_error`
decides what the commit does.

> [!NOTE]
> The terminology server needs the `fhir` build feature, which is on in the
> published binary and container images. A binary built with
> `--no-default-features` refuses at startup if `terminology.server` is enabled;
> the in-process openEHR terminology bundle remains available.

### Authenticating to the terminology server

A server behind an authorization server takes the OAuth2 client-credentials
grant under `[terminology.server.oauth]`, with the same keys as
`[audit.fhir_feed.oauth]` and `[authz.abac.remote.oauth]`. The access token is
cached and re-requested shortly before it expires, and a request the server
answers with `401` is sent once more under a new token.

| Key | Type | Default | Description |
|---|---|---|---|
| `token_url` | secret URL | one of `token_url` and `issuer` | The OAuth2 token endpoint. Credentials embedded in it are redacted from every rendering. |
| `issuer` | string | one of `token_url` and `issuer` | The issuer whose metadata names the token endpoint (RFC 8414). |
| `client_id` | string | required | The registered client identifier. Empty is a boot error. |
| `client_secret` / `client_secret_file` | secret / path | one is required | The client secret, inline or read from a mounted file. |
| `client_auth` | enum{client_secret_basic,client_secret_post} | `client_secret_basic` | How the client authenticates at the token endpoint. |
| `scope` | string | unset | The `scope` request parameter. |
| `audience` | string | unset | The `audience` request parameter some authorization servers read. |
| `resource` | URL | unset | The RFC 8707 `resource` request parameter. |

```toml
[terminology.server.oauth]
token_url = "https://idp.example.org/realms/ts/protocol/openid-connect/token"
client_id = "ferroehr-cdr"
client_secret_file = "/run/secrets/ts-client"
scope = "system/*.read"
```

A grant beside credentials in the server URL's userinfo is a boot error: the
server would receive two authorizations.

### Archetype value-set bindings at commit

With `[terminology.server]` enabled, committing a COMPOSITION also resolves
the archetype **constraint bindings** its template declares: where a template
binds an `ac` code to an external terminology query, the coded value in the
composition must be a member of the value set that query returns.

- The code is in the value set → the commit proceeds.
- The code is **not** in the value set → `422` naming the path, the code, and
  the bound query. That is a real constraint violation, not a service failure,
  so `fail_on_error` does not change it.
- The value set could **not be resolved** (server down, error response, unknown
  value set) → `fail_on_error` decides: `false` (the default) accepts the
  commit and logs a warning; `true` rejects it with `422`.

With `[terminology.server]` disabled (the default) no binding is resolved
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
