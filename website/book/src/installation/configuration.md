# Configuration reference

FerroEHR is configured by **one file, `ferroehr.toml`**, whose sections cover
the entire server, with `FERROEHR_*` environment variables (and repeatable
`--set` flags) as per-key overrides on top. This page is the entry point: the
quickstart, how configuration loads and how environment names map onto the
file, the one key that selects the openEHR specification generation set, and a
map of which page documents which section.

<!-- toc -->

## Quickstart

Generate an annotated template, edit it, and run:

```bash
# Write a fully-commented ferroehr.toml with every key at its default.
ferroehr config default > ferroehr.toml

# Edit it — at minimum set db.url and an auth mechanism.
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

- `DATABASE_URL` → `db.url`, and `RUST_LOG` → `log.filter`.
- `PORT` → a `0.0.0.0:<PORT>` value for `server.bind` (the port container
  platforms such as Vercel, Cloud Run and Heroku inject and route traffic
  to).
- The [libpq environment set](https://www.postgresql.org/docs/current/libpq-envars.html)
  (`PGHOST`, `PGUSER`, `PGPASSWORD`, `PGDATABASE`, `PGPORT`, `PGSSLMODE`),
  which managed-Postgres integrations such as Neon inject: when `PGHOST` is
  set and no URL form is, the server assembles the DSN from them. An
  explicit `DATABASE_URL` beats the assembled form, and `FERROEHR__DB__URL`
  beats both.

Nothing else has a non-`FERROEHR_` name.

### The environment-variable mapping

Every key has one mechanical environment spelling: **`FERROEHR` + the TOML
path, upper-cased, with a double underscore (`__`) between every segment,
including after the `FERROEHR` prefix.** A single underscore only ever appears
*inside* a key word.

| TOML | Environment variable |
|---|---|
| `[db] max_connections = 20` | `FERROEHR__DB__MAX_CONNECTIONS=20` |
| `[auth.oidc] issuer = "…"` | `FERROEHR__AUTH__OIDC__ISSUER=…` |
| `[management.endpoints] env = "off"` | `FERROEHR__MANAGEMENT__ENDPOINTS__ENV=off` |
| `[terminology.external.providers.default] url = "…"` | `FERROEHR__TERMINOLOGY__EXTERNAL__PROVIDERS__DEFAULT__URL=…` |

Scalars are typed automatically (bool / int / float, else string).
**List-typed keys take comma-separated values**
(`FERROEHR__AUTH__OIDC__AUDIENCES=ferroehr,other`). Map-keyed tables are
reachable too: the map key is just another segment
(`FERROEHR__TERMINOLOGY__EXTERNAL__PROVIDERS__SNOMED__URL`). Arrays of tables (the
Basic-auth user store) are **file-only**, because the environment grammar has
no way to spell an array index.

> [!NOTE]
> Enum values are lowercase / `snake_case` tokens, exactly as the tables show.
> Secret-typed keys are redacted everywhere the configuration is rendered (the
> `/management/env` snapshot, `ferroehr config check`, logs), and each has a
> `*_file` sibling that reads the value from a file, for Kubernetes and Docker
> secret mounts. Setting a secret and its `*_file` sibling at once is a boot
> error.

### File discovery

The first of these that exists is loaded (later layers still override its
values):

1. `--config <path>`,
2. `FERROEHR_CONFIG=<path>`,
3. `./ferroehr.toml` (current directory),
4. `/etc/ferroehr/ferroehr.toml`.

An explicitly pointed-at file (1–2) is fatal if absent; the search-order files
(3–4) are simply skipped when absent, but fatal if present and unreadable or
unparseable.

### Strict validation

Configuration is validated at boot (and by `ferroehr config check`), and the
server refuses to start on any error:

- **Unknown keys are rejected:** in the file, with the offending line number
  and a did-you-mean suggestion, and in the `FERROEHR_` environment namespace.
  A variable in that namespace that is neither a known section nor one of the
  reserved non-configuration names is a boot error, so a misspelled security
  key can never be silently ignored. A single-underscore near miss
  (`FERROEHR_DB_URL`) is reported with the exact uniform spelling it should
  have had.
- **Type errors are boot errors**, naming the key and what was expected.
- **Semantic errors are aggregated:** one pass reports every problem at once,
  so a broken configuration is fixed in a single iteration.

## `spec_profile`

```toml
# The openEHR specification generation set the server runs.
spec_profile = "development"   # or "stable"
```

openEHR publishes released specification versions and keeps developing the
next ones. FerroEHR generates **both**, and this key decides which set the
running server serves.

| Value | RM | BASE | LANG | Choose it when |
|---|---|---|---|---|
| `development` *(default)* | 1.2.0 | 1.3.0 | 1.1.0 | You want the generations this build is developed against. This is the default for every deployment that does not set the key. |
| `stable` | 1.1.0 | 1.2.0 | 1.0.0 | Your governance requires running on released openEHR specifications only. |

Environment form: `FERROEHR__SPEC_PROFILE=stable`. The key is a top-level
scalar, not a section; there is no `[spec_profile]` table.

### Why it is one key and not three

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

## `deployment_profile`

```toml
# The declared posture: what this deployment may hold.
deployment_profile = "sandbox"   # or "production"
deployment_accepts = []          # production only: gaps run without, by name
```

Every separation FerroEHR can make is a configuration key an operator can leave
unset, and a deployment that has made none of them looks, from its own logs and
its own API, like one that has made all of them. This key gives the server a
declared posture and makes the two impossible to confuse.

| Value | Asserts | On boot |
|---|---|---|
| `sandbox` *(default)* | Nothing; the deployment must not hold real personal data | Says which separations are missing: a red notice on the banner, one `warn` line per gap with the structured field `posture = "deployment"`, and the same list on `GET /ferroehr/rest/status` under `deployment` |
| `production` | Every separation below holds, or is accepted by name in `deployment_accepts` | Refuses to start otherwise, naming each open gap, what it found and what to change |

The separations, each a real property the server checks rather than a box
ticked by being present:

| Gap token | What `production` requires |
|---|---|
| `shared_credential` | `[storage.party] url` and `[storage.linkage] url` set, so the domains connect on their own database roles |
| `shared_cluster` | The four pools reach four different PostgreSQL clusters, read from `pg_control_system().system_identifier` on each pool, never from the DSN text. The audit pool counts: Swiss law keeps the access log on a system separate from the one processing the data |
| `open_subject_namespace` | `[privacy] subject_namespaces` declared, so an `EHR_STATUS` subject is an opaque pseudonym |
| `audit_off` | `[audit]` enabled with a durable sink (the local store, syslog or the FHIR feed) |
| `audit_fails_open` | `[audit] fail_mode = "closed"`, so an operation whose access record cannot be taken is refused instead of succeeding unlogged |
| `open_ehr_access_default` | `[authz.rbac] ehr_access_default = "restricted"`, so an EHR carrying no `ACCESS_CONTROL_SETTINGS` is reachable only by an admin. The shipped default is `open` |
| `migrate_on_runtime_credential` | `[db] migrate_url` set, or `migrate = "verify"`, so no credential that serves requests can alter the schema. Evaluated per domain database: a domain relocated to a database of its own is prepared on that domain's runtime DSN |

An accepted gap is stated on every boot and on `/rest/status`; it can be run,
not hidden. Environment form: `FERROEHR__DEPLOYMENT_PROFILE=production`,
`FERROEHR__DEPLOYMENT_ACCEPTS=shared_cluster`. There is no `research` value:
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
| `[server]`, `[server.limits]`, `[server.rate_limit]`, `[server.connection]`, `[server.tls]`, `[server.identity]` | The HTTP listener, request limits, rate limiting, connection bounds, TLS, the deployment's own identity | [Server, database & telemetry](config-server.md) |
| `[db]` | PostgreSQL connection, pool, migrations | [Server, database & telemetry](config-server.md) |
| `[log]`, `[telemetry]` | Log rendering and OpenTelemetry export | [Server, database & telemetry](config-server.md) |
| `[usage_report]` | The daily installation report to FerroPULSE, on by default; what it sends is on the [Usage report](../usage-report.md) page | [Server, database & telemetry](config-server.md#usage_report) |
| `[auth]`, `[authz]` | Authentication (Basic, OAuth2/OIDC) and RBAC/ABAC | [Authentication & access](config-auth.md) |
| `[admin]`, `[management]` | The ADMIN API group and the ops-introspection surface | [Authentication & access](config-auth.md) |
| `[smart]` | SMART App Launch discovery and scope enforcement | [Authentication & access](config-auth.md) |
| `[signing]` | VERSION signing and read-time verification | [Authentication & access](config-auth.md) |
| `[licence]` | The commercial licence token | [Authentication & access](config-auth.md) |
| `[query]` | AQL execution budgets and result ceilings | [Integrations](config-integrations.md) |
| `[events]`, `[fhir]` | Change eventing and the FHIR connector | [Integrations](config-integrations.md) |
| `[terminology]`, `[multimedia]` | External terminology servers, multimedia externalization | [Integrations](config-integrations.md) |
| `[audit]`, `[audit.store]`, `[audit.syslog]`, `[audit.fhir_feed]` | The IHE ATNA audit trail and its sinks | [Audit](config-audit.md) |
| `[privacy]`, `[privacy.identifier_scan]` | What the clinical side refuses to hold: the subject reference, identified parties, the identifier scanner | [Privacy & data minimisation](config-privacy.md) |
| `[cohort]`, `[cohort.predicates]` | Cross-domain cohort queries: the demographic predicate allow-list, the small-cell floor, the cohort ceiling | [Privacy & data minimisation](config-privacy.md) |
| `[demographic.identifier_protection]` | Sealing national identifiers in the demographic domain, and the key that opens them | [Privacy & data minimisation](config-privacy.md) |
| — | The CLI, the production checklist, file-versus-environment guidance | [CLI & production checklist](config-cli.md) |
