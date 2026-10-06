# CLI & production checklist

The binary's command-line surface, what a zero-configuration boot actually
gives you, the minimum a production deployment sets, and which material belongs
in a mounted file rather than an environment variable. Precedence, the
environment-name grammar, and file discovery are on the
[Configuration reference](configuration.md) index.

<!-- toc -->

## The command line

Two flags are global: they apply to the server and to every subcommand:

| Flag | Description |
|---|---|
| `--config <path>` | The configuration file to load, overriding the search order. Fatal if missing or unreadable. |
| `--set <key>=<value>` | A dotted-path override, repeatable, highest precedence of all layers (e.g. `--set db.max_connections=40`). |

With no subcommand, the binary boots the server. `ferroehr --version` prints
the version and the manufacturer of the release:

```text
ferroehr <version>
Manufactured by Cadasto B.V., Comeniusstraat 2d, 1817 MS Alkmaar, The Netherlands, info@cadasto.com
https://www.cadasto.com/contact/
```

### `ferroehr config …`

```text
ferroehr config default             # print the annotated default ferroehr.toml
ferroehr config check [--config P]  # validate file + environment + --set
```

`config default` writes the fully-commented template every key's default comes
from, the starting point for a real `ferroehr.toml`.

`config check` runs the same validation the server runs at boot (the strict
unknown-key sweep, the type pass, the aggregated semantic rules, and the
"authentication enabled needs a mechanism" rule) and **touches no database**,
so it is safe in CI and before a rollout. It exits 0 when the configuration is
valid and 1 otherwise; on success it prints the effective configuration as TOML
with every secret redacted, and notes on stderr when `db.url` is still the
built-in development default.

### `ferroehr db …`

```text
ferroehr db migrate   # apply the embedded migrations and exit
ferroehr db verify    # verify, issuing no DDL, that the schema matches this build
```

These exist so a least-privilege deployment can separate the two database
identities: run `db migrate` once under a DSN that holds DDL rights (a
Kubernetes Job, an init container, a CI/CD stage), then boot the server with
[`db.migrate = "verify"`](config-server.md#db) under a DSN with no DDL rights at
all. `verify` still reads all five `_sqlx_migrations` bookkeeping tables, which
no least-privilege role can do, so point `[db] migrate_url` at the credential
that prepares the schema. See
[Operations](../operations.md#which-credential-prepares-the-schema).

### `ferroehr healthcheck`

Probes the running server's status endpoint and exits 0 on a 2xx, 1 otherwise:
the container `HEALTHCHECK` and the Kubernetes exec-probe fallback.

| Variable | Type | Default | Description |
|---|---|---|---|
| `FERROEHR_HEALTHCHECK_URL` | URL | derived: `http://127.0.0.1:<server.bind port><REST root>/status` (`http://127.0.0.1:8080/ferroehr/rest/status` with the defaults) | The URL the subcommand probes; also settable as `--url`. Not part of `ferroehr.toml`. Unset, the subcommand loads the same configuration as the server (file, environment, `--set`) and follows `server.bind` and `server.base_path`, so a shortened base path moves the probe with it. |

### `ferroehr usage-report --print`

```text
ferroehr usage-report --print                  # the start report
ferroehr usage-report --print --event daily    # the daily report, with its metrics
```

Prints the exact JSON body of the [usage report](../usage-report.md) this
instance would send now, on one line to stdout, and sends nothing. It loads the
same configuration as the server and connects to the database to read the
instance id and the shared metrics window, writing nothing. Notes go to stderr:
a sample id when the database holds none yet, and a line saying so when the
report is off. `--print` is required; printing is the only action.

### `ferroehr report`

```text
ferroehr report                              # writes ferroehr-report-<UTC time>.json here
ferroehr report --output /tmp/report.json    # writes to the path you name
ferroehr report --output -                   # writes the JSON to stdout
```

Writes one JSON document naming what this deployment runs. Attach it to a
complaint, a non-conformity finding or a serious-incident report: it is the
"which FerroEHR does this deployment run" answer those start from. It loads the
same configuration as the server and must pass the same validation. It reads
the databases and writes nothing to them.

| Part | What it holds |
|---|---|
| `manifest` | every part below with its status (`included`, `partial` or `unavailable`) and, for the last two, the reason |
| `build` | version, commit, build date, `rustc`, the active `spec_profile` and the specification versions it selects, the PostgreSQL target, the audit posture, and the manufacturer |
| `image` | named in the manifest only: a running binary cannot read the digest of the image it was started from, so attach `docker inspect --format '{{index .RepoDigests 0}}' <image>` or the pod's `status.containerStatuses[].imageID` |
| `target` | the CPU architecture and operating system the binary was built for, and whether it is a debug build |
| `features` | the optional integrations compiled in (`events`, `fhir`, `multimedia`) |
| `openehr_crates` | each `openehr-*` crate the binary links, with its version |
| `licence` | the licence summary `GET /ferroehr/rest/status` serves (state, use, licensee, last valid day, what the configured token contributed); never the token |
| `deployment` | the deployment profile, the open gaps and the accepted ones, measured over the databases, or from the configuration alone when they do not answer |
| `schema` | for each database, the highest migration version applied and the highest this build embeds, per schema |
| `configuration` | the effective configuration as `config check` prints it |

Nothing in it identifies a person: it reads no clinical, demographic or audit
table. Every secret in the configuration shows `***`, every URL shows `***` in
place of its user name and password, and a database error quoted as a reason
has each configured database user name and password masked too. A database
that does not answer within 15 seconds leaves the `schema` part out and names
it `unavailable` with `unreachable: …` as the reason.

## Zero-config boot and the production checklist

With no file and no environment, the effective configuration is: listener
`0.0.0.0:8080` at the ITS-REST base path with Swagger UI; the database at the
built-in development DSN with migrations applied at boot; RBAC on; signing on in
`digest` mode with read-time verification strict; the audit trail on with only
the local store; rate limiting on; logs in `auto` format at `info`; **and every
integration off**. The one outbound connection on by default is the
[usage report](../usage-report.md), which `usage_report.enabled = false`
switches off.

One thing that configuration does *not* do is serve requests.
`auth.enabled` defaults to `true`, and **authentication enabled with no
mechanism configured is a boot error** rather than a running server that refuses
everything: RFC 9110 §11.6.1 requires a `401` challenge to name a scheme
applicable to the resource, and a server with no mechanism has none: it could
only refuse every request while advertising a scheme it does not implement. The
error names the three ways out: add `[[auth.basic.users]]`, add an `[auth.oidc]`
issuer, or set `auth.enabled = false` for development. So a bare `docker run` of
the image with no configuration stops at startup with that message, while the
downloadable Compose quickstart boots because it ships a user.

For production, set at least:

- **`db.url`:** the real DSN, via `FERROEHR__DB__URL` from a secret or a
  `url_file`-mounted value, never inline in a world-readable file. Leaving the
  development default in place is warned about loudly at every boot.
- **`db.migrate = "verify"`** with the schema applied out of band, so the
  serving role needs no DDL rights, plus **`db.migrate_url`** naming the
  credential that reads every schema's migration state at boot. The serving
  credential cannot.
- **`deployment_profile = "production"`** once the deployment holds real
  personal data. The server then refuses to start while a separation is open
  and not accepted by name, instead of running as a sandbox that looks
  identical.
- **an authentication mechanism:** a Basic user store and/or `[auth.oidc]`.
- **`log.format = "json"`** for cluster log collectors.
- **`server.cors_permissive`** stays `false`; **`server.swagger_ui`** per
  posture.
- **`server.system_id`:** this deployment's own openEHR system identifier.
  Choose it before the first EHR is created: it is stored with every EHR, audit
  entry and version identifier, and changing it later never rewrites what is
  already committed.
- **`management.*`** per posture. A dedicated `management.port` is recommended
  so the introspection surface is never reachable on the clinical listener, and
  every endpoint stays `off` until you name a level for it.
- **TLS everywhere a transport supports it:** `server.tls` (or a
  TLS-terminating ingress), `audit.syslog.transport = "tls"`, `events.tls`,
  `fhir.outbound.tls`, HTTPS for the object store.
- **real secrets via the environment or a `*_file` sibling**, never inline.

Before a deployment holding real patient data goes live, work through the
[go-live checklist](../security/go-live-checklist.md): it turns each item above
into something you run and read back, and adds the privacy and audit checks a
controller is asked for.

## What belongs in a mounted file (versus the environment)

The environment cannot carry an array of tables, so the **Basic-auth user
store** (`[[auth.basic.users]]`) is file-only.

Genuinely file-shaped material (the **PGP signing key**, **Cedar policies**,
**ATNA and terminology-server PEMs**, a **JWKS blob**) is referenced by an
in-TOML `*_path` / `*_file` key pointing at a mounted path. On Kubernetes the
chart's `config.files` map materialises each entry under `/etc/ferroehr/`,
read-only, from a Secret.

Prefer a `*_file` sibling over the environment form for any secret in a
container: an environment value is readable through `/proc/<pid>/environ` and is
inherited by every child process the container spawns.

Everything else is a plain key you can set in the file or override with a
`FERROEHR_*` variable.

For a worked development example (the server section, CORS, admin, management
and the Basic-auth user store) read the configuration carried inline in the
quickstart `docker-compose.yml`; see [Docker Compose](compose.md).

## Variables outside the server's namespace

The PostgreSQL init container's variables are `PG_INIT_USER`, `PG_INIT_PASSWORD`
and `PG_INIT_DB`; they configure the database container, not the server, and
sit outside the server's reserved `FERROEHR_` namespace.

Inside that namespace, a handful of names are deliberately **not** configuration
keys and pass the strict sweep untouched: `FERROEHR_CONFIG` (the config-file
pointer), `FERROEHR_HEALTHCHECK_URL` (the container healthcheck), the
build-stamp variables, and the Compose parameterization (image tags, host ports,
CPU and memory limits). They keep a single `_` by design, which is exactly what
distinguishes them from configuration keys, and why a single-underscore
misspelling of a real key is reported at boot with the uniform spelling it
should have had.
