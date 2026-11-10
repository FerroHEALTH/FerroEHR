# Configuration reference

FerroEHR is configured by **one file, `ferroehr.toml`**, whose sections cover
the entire server, with `FERROEHR_*` environment variables (and repeatable
`--set` flags) as per-key overrides on top. This page is the entry point: the
quickstart, how configuration loads and how environment names map onto the
file, the one key that selects the openEHR specification generation set, and a
map of which page documents which section.

<!-- toc -->

## Quickstart

Copy the annotated template, edit it, and run:

```bash
# A fully-commented ferroehr.toml with every key at its default.
curl -fsSLo ferroehr.toml \
  https://raw.githubusercontent.com/FerroHEALTH/FerroEHR/main/app/ferroehr/assets/ferroehr.default.toml

# Edit it — at minimum set database.url and an auth mechanism.
$EDITOR ferroehr.toml

# Validate without touching the database, then run.
ferroehr config check --config ferroehr.toml
ferroehr --config ferroehr.toml
```

A server started with **no file and no environment** still boots, with one
exception you will hit immediately: authentication is on by default and a
server with no mechanism configured refuses to start. See
[Zero-config boot and the production checklist](config-cli.md#zero-config-boot-and-the-production-checklist).

## How configuration loads

Configuration is assembled once at boot from four layers, lowest precedence to
highest:

1. **Built-in defaults:** the values in the tables on the section pages.
2. **The config file:** `ferroehr.toml` (see [file discovery](#file-discovery)).
3. **`FERROEHR_*` environment variables:** override individual keys.
4. **`--set key=value` CLI flags** (repeatable), which win over everything.

Some conventional names sit *below* their `FERROEHR_` forms within layer 3:

- `DATABASE_URL` → `database.url`, and `RUST_LOG` → `telemetry.filter`.
- `PORT` → a `0.0.0.0:<PORT>` value for `server.listen` (the port container
  platforms such as Vercel, Cloud Run and Heroku inject and route traffic
  to).
- The [libpq environment set](https://www.postgresql.org/docs/current/libpq-envars.html)
  (`PGHOST`, `PGUSER`, `PGPASSWORD`, `PGDATABASE`, `PGPORT`, `PGSSLMODE`),
  which managed-Postgres integrations such as Neon inject: when `PGHOST` is
  set and no URL form is, the server assembles the DSN from them. An
  explicit `DATABASE_URL` beats the assembled form, and `FERROEHR__DATABASE__URL`
  beats both.

Nothing else has a non-`FERROEHR_` name.

### The environment-variable mapping

Every key has one mechanical environment spelling: **`FERROEHR` + the TOML
path, upper-cased, with a double underscore (`__`) between every segment,
including after the `FERROEHR` prefix.** A single underscore only ever appears
*inside* a key word.

| TOML | Environment variable |
|---|---|
| `[database] max_connections = 20` | `FERROEHR__DATABASE__MAX_CONNECTIONS=20` |
| `[authz.rbac] admin_role = "…"` | `FERROEHR__AUTHZ__RBAC__ADMIN_ROLE=…` |
| `[management.endpoints] env = "off"` | `FERROEHR__MANAGEMENT__ENDPOINTS__ENV=off` |
| `[terminology.endpoint] url = "…"` | `FERROEHR__TERMINOLOGY__ENDPOINT__URL=…` |

Scalars are typed automatically (bool / int / float, else string).
**List-typed keys take comma-separated values**
(`FERROEHR__AUTHZ__RBAC__ROLE_ISSUERS=https://a,https://b`). Map-keyed tables
are reachable too: the map key is just another segment
(`FERROEHR__AUTHZ__ABAC__POLICY__COMPOSITION__NAME`). An array of tables is
set by element index: `FERROEHR__AUTH__ISSUER__0__AUDIENCE=ferroehr` sets
`audience` of the first `[[auth.issuer]]` table key by key over the file's,
and the next index past the end adds a table. An index with a leading zero,
one that skips an element, or one under a key that holds no array of tables
refuses the start.

> [!NOTE]
> Enum values are lowercase / `snake_case` tokens, exactly as the tables show.
> Secret-typed keys are redacted everywhere the configuration is rendered (the
> `/management/env` snapshot, `GET /ferroehr/rest/extensions/v1/admin/config`, `ferroehr config
> check`, `ferroehr report`, logs), and a secret-URL key shows its host and path
> with the embedded credentials masked. Each secret, and each database URL,
> has a `*_file` sibling that reads the value from a
> file, for Kubernetes and Docker secret mounts. Setting a key and its `*_file`
> sibling at once is a boot error.

### File discovery

The server reads the file `--config <path>` names, or else the file
`FERROEHR_CONFIG=<path>` names; later layers still override its values. A named
file is fatal if absent, unreadable or unparseable. With neither set, the server
reads no file and searches no directory: the environment and the defaults make
the whole configuration.

### Strict validation

Configuration is validated at boot (and by `ferroehr config check`), and the
server refuses to start on any error:

- **Unknown keys are rejected:** in the file, naming the key and the closest
  valid one, and in every `FERROEHR__…` override, so a misspelled security key
  can never be silently ignored. Only the uniform `FERROEHR__<SECTION>__<KEY>`
  form sets a key. Any other `FERROEHR_` variable, such as the single-underscore
  `FERROEHR_DB_URL`, sets nothing and is left alone, because the viewer's
  `FERROEHR_VIEWER_` variables share the prefix.
- **Type errors are boot errors**, naming the key and what was expected.
- **Semantic errors are aggregated:** one pass reports every problem at once,
  so a broken configuration is fixed in a single iteration.

### Durations, sizes and periods

Every duration is one string with its unit and no space: `"30s"`, `"10min"`,
`"12h"`, `"7d"`, or milliseconds with `ms`. The unit symbols are the UCUM codes (`ms`, `s`,
`min`, `h`, `d`), the unit system FHIR `Quantity` and openEHR `DV_QUANTITY`
use, so `"10m"` is refused (`m` is the metre) and a bare number is refused,
never read as seconds or milliseconds. Every size is one string with an
IEC 80000-13 binary unit: `"512B"`, `"256KiB"`, `"64MiB"`, or `GiB`; a decimal
unit such as `MB` is refused. A retention is an ISO 8601 calendar period
of whole years or days, `"P5Y"` or `"P1830D"`; unset keeps forever. The
environment carries the same strings: `FERROEHR__RECORD__QUERY__TIMEOUT=45s`.
`[server]` and `[auth]` are family sections that keep their own key shapes
until the family releases move them.

### Keys a release before v5.7.0 read

v5.7.0 moved the configuration tree to one table per concern. Every key an
earlier release read is refused at load with the key that replaced it, all of
them in one pass, so `ferroehr config check --config <old file>` prints the
whole move list for an old file:

```text
db was replaced by [database]: db.acquire_timeout_secs is database.acquire_timeout ...
rest was replaced by [api]: rest.swagger_ui is api.openapi_ui ...
audit.store.retention_days was replaced by audit.store.retention, a calendar period ...
```

No key is read under two names. These refusals ship in v5.7.0 only; the next
release removes them, and an old key is then an unknown key. The full map is
in the v5.7.0 changelog.

## `deployment.spec_profile`

```toml
# The openEHR specification generation set the server runs.
[deployment]
spec_profile = "development"   # or "stable"
```

openEHR publishes released specification versions and keeps developing the
next ones. FerroEHR generates **both**, and this key decides which set the
running server serves.

| Value | RM | BASE | Choose it when |
|---|---|---|---|
| `development` *(default)* | 1.2.0 | 1.3.0 | You want the generations this build is developed against. This is the default for every deployment that does not set the key. |
| `stable` | 1.1.0 | 1.2.0 | Your governance requires running on released openEHR specifications only. |

Environment form: `FERROEHR__DEPLOYMENT__SPEC_PROFILE=stable`.

### Why it is one key and not two

The components' generations are modelled against each other, not
independently: RM 1.1.0's own machine-readable model declares that it includes
BASE 1.2.0. Letting you pick RM 1.1.0 with BASE 1.3.0 would offer a
combination openEHR never published, so the profile is a single coupled choice
and incoherent sets are unrepresentable.

### Seeing which profile is active

The profile is reported in two places, so it is never a guess:

- the **boot banner**, on every start, alongside the RM version it serves;
- **`GET /management/info`**, which names the active profile with the RM and
  BASE generations it selects, next to the build provenance.

### What changes on the wire

The profile is an **acceptance boundary**, and it is exact in both directions.

Under `stable`, a query that addresses specification surface the released
generations do not define is **refused with a typed error naming the active
profile**: an AQL `FROM` class or a path attribute RM 1.1.0 does not declare
is rejected at planning time rather than answered as though it existed.

Released surface the development line later dropped **stays accepted** under
`stable`, which is the half most implementations get wrong. A demographic
party carrying the RM 1.1.0 `PARTY.reverse_relationships` attribute is read
and accepted under `stable`; `development` refuses it as an undeclared key,
because RM 1.2.0 removed it. The attribute is derived data the server
recomputes from `relationships`, so the copy you send is validated and then
dropped rather than stored.

Reading a stored object is bounded the same way. Every commit records whether
the released generations can express the body it accepted, and under `stable`
a stored version they cannot is **refused with `409 Conflict`** naming the
active profile, the version, and the remedy; never served under a generation
set that does not define it, and never rewritten to fit one. Under
`development` the same object reads normally. This is FerroEHR's own
extension: no openEHR specification governs runtime version selection, so the
status follows HTTP itself (RFC 9110 §15.5.10, a conflict with the current
state of the target resource, whose resolution the response describes).

**Queries take the same refusal wherever they serve a version body.** AQL is
gated in two places, and they answer different questions. At *planning* time the
query text is checked: a `FROM` class or path attribute the released generations
do not declare is rejected. At *result assembly* the projection is checked: a
whole-object projection (`SELECT c FROM EHR e CONTAINS COMPOSITION c`) returns
stored version bodies, so if any row of the page comes from a version the
released generations cannot express, the whole query answers `409 Conflict`
naming that version. The row is never quietly dropped from the result set
instead: a `RESULT_SET` is columns and rows of values with nowhere to explain a
missing row, so silently eliding one would be an answer you could not tell from
"no such data".

A **leaf projection** over the very same rows (`SELECT c/name/value FROM EHR e
CONTAINS COMPOSITION c`) still answers `200`. It serves data values rather than
version bodies, over paths the planning gate has already bounded to the released
generation's declared surface. That is the honest boundary: the profile is an
acceptance boundary on what is served as an openEHR object, not a content filter
over the values inside one.

Under `development` none of this applies and it costs nothing: the assembly
gate returns before it touches the database.

The exact additive delta between the two generation sets is pinned in the
build, so a future openEHR re-vendoring cannot silently widen or narrow what a
profile accepts.

### Changing the profile on an existing deployment

Treat the profile as a deployment commitment. Both directions are defined, but
they are not symmetric:

| Direction | Supported? | Why |
|---|---|---|
| `stable` → `development` | **Always safe** | openEHR minor releases are additive by the Foundation's own release strategy, so every object stored under the released generations is valid under the development ones. |
| `development` → `stable` | **Only for data that never used a development-only construct** | There is no down-conversion. An object that did use one becomes unreadable (`409`, not a silently degraded body) until you switch back. |

If you need to stay on released specifications, choose `stable` on day one
rather than migrating into it later. Silently rewriting stored clinical
content to fit an older generation would be data loss disguised as a setting,
so no tool does it.

Objects committed before this stamp existed, and objects written by the
verbatim-replay paths (EHR-Extract import, archive load), carry no recorded
answer; they are assessed at read instead, which costs one extra parse per
read of such an object and only under `stable`. Nothing is written back: a
read stays a read.

> [!NOTE]
> No openEHR specification governs runtime version selection; this key is
> FerroEHR's own design. What the specifications do govern is the
> compatibility direction it relies on: minor releases within a major line are
> additive supersets.

## `deployment.profile`

```toml
# The declared posture: what this deployment may hold.
[deployment]
profile = "sandbox"   # or "production"
accepts = []          # production only: gaps run without, by name
```

Every separation FerroEHR can make is a configuration key an operator can leave
unset, and a deployment that has made none of them looks, from its own logs and
its own API, like one that has made all of them. This key gives the server a
declared posture and makes the two impossible to confuse.

| Value | Asserts | On boot |
|---|---|---|
| `sandbox` *(the binary's default; the Helm chart has none and refuses to render until you choose)* | Nothing; the deployment must not hold real personal data | Says which separations are missing: a red notice on the banner, one `warn` line per gap with the structured field `posture = "deployment"`, and the same list on `GET /management/status` under `deployment`; the public `GET /ferroehr/rest/status` carries the profile only |
| `production` | Every separation below holds, or is accepted by name in `deployment.accepts` | Refuses to start otherwise, naming each open gap, what it found and what to change |

The separations, each a real property the server checks rather than a box
ticked by being present:

| Gap token | What `production` requires |
|---|---|
| `shared_credential` | `[database.party] url` (or `[database.clinical] url`) set, so the two domains connect on their own database roles |
| `shared_cluster` | The three pools reach three different PostgreSQL clusters, read from `pg_control_system().system_identifier` on each pool, never from the DSN text. The audit pool counts: Swiss law keeps the access log on a system separate from the one processing the data |
| `open_subject_namespace` | `[privacy] subject_namespaces` declared, so an `EHR_STATUS` subject is an opaque pseudonym |
| `audit_off` | `[audit]` enabled with a durable sink (the local store, syslog or the FHIR feed) |
| `audit_fails_open` | `[audit] fail_mode = "closed"`, so an operation whose access record cannot be taken is refused instead of succeeding unlogged |
| `no_category_map` | A category map applies while auditing is on: the one FerroEHR ships (the default) or `[audit.categories]` templates and archetypes of your own, so every access record names the categories of the data it reached (EHDS Annex II 3.2(c)). The gap opens only with `[audit.categories] shipped_map = false` and nothing declared |
| `open_ehr_access_default` | `[authz.rbac] ehr_access_default = "restricted"`, so an EHR carrying no `ACCESS_CONTROL_SETTINGS` is reachable only by an admin. The shipped default is `open`. The setting is enforced with `[authz.rbac] enabled` on or off |
| `migrate_on_runtime_credential` | `[database] migrate_url` set, or `migrations = "verify"`, so no credential that serves requests can alter the schema. Evaluated per domain database: a domain relocated to a database of its own is prepared on `[database.<domain>] migrate_url` when that is set, and on that domain's runtime DSN, which opens the gap, when it is not |
| `auth_off` | `[auth] enabled = true`, so no request is served unauthenticated (CRA Annex I Part I(2)(d)) |
| `rbac_off` | `[authz.rbac] enabled = true` whenever authentication is on, so a caller reaches only the operations its roles allow (CRA Annex I Part I(2)(d)). Not reported while `auth_off` is |
| `plaintext_listener` | `[server.tls]` set, or `[server] listen` on a loopback address, so data does not cross the network in clear (CRA Annex I Part I(2)(e)). A deployment behind a TLS-terminating ingress accepts `plaintext_listener` by name, which records that the ingress carries the encryption. The gap also opens when `management.listen` is set on an enabled management surface: that listener never speaks TLS, so unset it (the surface then shares the main listener) or accept the gap by name where it is unreachable off the host |
| `plaintext_audit_feed` | An `https://` URL for an enabled audit FHIR feed (`[audit.fhir_feed] url`), so the access records, which name the patient and the professional, do not cross the network in clear (CRA Annex I Part I(2)(e)). A deployment whose repository is reached on a segment it trusts accepts `plaintext_audit_feed` by name |
| `plaintext_object_store` | An `https://` `record.multimedia.url` on an enabled multimedia store, so the externalised clinical blobs travel over an `https://` endpoint (CRA Annex I Part I(2)(e)). A deployment whose object store is reached on a segment it trusts accepts `plaintext_object_store` by name |

An accepted gap is stated on every boot and on `/management/status`; it can be run,
not hidden. Environment form: `FERROEHR__DEPLOYMENT__PROFILE=production`,
`FERROEHR__DEPLOYMENT__ACCEPTS=shared_cluster`. There is no `research` value:
the key controls rigour, not purpose, and a secondary-use platform under an
EHDS data permit runs on real patient data at production rigour whatever it is
for.

**This is FerroEHR's own posture, not a legal requirement.** GDPR Art. 4(5)
asks that the additional information needed to re-identify a person be "kept
separately and … subject to technical and organisational measures", not that it
sit on a separate server; one cluster with separated schemas and roles is a
defensible reading. Two clusters are materially stronger, because a superuser,
an instance-wide point-in-time recovery and a single compromise are bridges no
grant can close, and that is a choice a deployment should make deliberately
rather than inherit from a quickstart. See
[Compliance](../compliance/index.md#the-pseudonymisation-boundary).

## Where each section is documented

| Section | What it covers | Page |
|---|---|---|
| `[deployment]` | The declared posture, the openEHR specification set and the licence token | This page |
| `[server]`, `[server.caller_rate]`, `[server.tls]` | The HTTP listener (the family section): timeouts, the body limit, the concurrency bound, the drain, the caller rate, TLS | [Server, database & telemetry](config-server.md) |
| `[api]`, `[api.options]`, `[api.cors]` | The openEHR REST API: the API base path, the system identifier, the OpenAPI UI, CORS, the clinical body tier, the ADMIN and terminology groups, the deployment's own identity | [Server, database & telemetry](config-server.md#api) |
| `[database]` | PostgreSQL connection, pool, migrations | [Server, database & telemetry](config-server.md) |
| `[telemetry]` | The console, the request log and the OpenTelemetry export | [Server, database & telemetry](config-server.md) |
| `[usage_report]` | The daily installation report to FerroPULSE, on by default; what it sends is on the [Usage report](../usage-report.md) page | [Server, database & telemetry](config-server.md#usage_report) |
| `[auth]`, `[authz]` | Authentication (Basic, OAuth2/OIDC) and RBAC/ABAC | [Authentication & access](config-auth.md) |
| `[management]` | The ops-introspection surface | [Authentication & access](config-auth.md) |
| `[auth.smart]` | SMART App Launch discovery and scope enforcement | [Authentication & access](config-auth.md) |
| `[privacy.access_delay]` | The EHDS Art. 3(3) delay of the person's own access to new data, off by default | [Authentication & access](config-auth.md#privacyaccess_delay) |
| `[record.signing]` | VERSION signing and read-time verification | [Authentication & access](config-auth.md) |
| `[record.versioning]` | The opt-in answer to an update whose content equals the current version's, off by default | [Authentication & access](config-auth.md#recordversioning) |
| `[record.query]` | AQL execution budgets and result ceilings | [Integrations](config-integrations.md) |
| `record.subject_index`, `[pixm]`, `[pixm.endpoint]` | Who holds the patient-to-EHR cross-reference: an IHE patient identity source over PIXm or the in-instance map | [Integrations](config-integrations.md#pixm) |
| `[record.change_feed]` | The commit-event outbox and the readers of its feed | [Integrations](config-integrations.md#recordchange_feed) |
| `[terminology]`, `[record.multimedia]` | The terminology server, multimedia externalization | [Integrations](config-integrations.md) |
| `[audit]`, `[audit.store]`, `[audit.syslog]`, `[audit.fhir_feed]` | The IHE ATNA audit trail and its sinks | [Audit](config-audit.md) |
| `[[authz.origin]]` | The origins of data the deployment recognises, which a retention period and an ABAC rule may name (EHDS Annex II 3.4) | [Audit](config-audit.md#authzorigin-the-origins-a-deployment-recognises) |
| `[privacy]`, `[privacy.identifier_scan]` | What the clinical side refuses to hold: the subject reference, identified parties, the identifier scanner | [Privacy & data minimisation](config-privacy.md) |
| `[privacy.cohort]`, `[privacy.cohort.predicates]` | Cross-domain cohort queries: the demographic predicate allow-list, the small-cell floor, the cohort ceiling | [Privacy & data minimisation](config-privacy.md) |
| `[privacy.identifier_protection]` | Sealing national identifiers in the demographic domain, and the key that opens them | [Privacy & data minimisation](config-privacy.md) |
| — | The CLI, the production checklist, file-versus-environment guidance | [CLI & production checklist](config-cli.md) |
