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
| `--config <path>` | The configuration file to load, overriding `FERROEHR_CONFIG`. Fatal if missing or unreadable. |
| `--set <key>=<value>` | A dotted-path override, repeatable, highest precedence of all layers (e.g. `--set database.max_connections=40`). |

With no subcommand, or with `serve`, the binary boots the server. `ferroehr --version` prints
the version and the manufacturer of the release:

```text
ferroehr <version>
Manufactured by Cadasto B.V., Comeniusstraat 2d, 1817 MS Alkmaar, The Netherlands, info@cadasto.com
https://www.cadasto.com/contact/
```

### `ferroehr config …`

```text
ferroehr config check [--config P]  # validate file + environment + --set
```

The fully-commented template every key's default comes from, the starting
point for a real `ferroehr.toml`, is
[`app/ferroehr/assets/ferroehr.default.toml`](https://github.com/FerroHEALTH/FerroEHR/blob/main/app/ferroehr/assets/ferroehr.default.toml).

`config check` runs the same validation the server runs at boot (the strict
unknown-key sweep, the type pass, the aggregated semantic rules, and the
"authentication enabled needs a mechanism" rule) and **touches no database**,
so it is safe in CI and before a rollout. It exits 0 when the configuration is
valid and 1 otherwise; on success it prints the effective configuration as TOML
with every secret redacted, and notes on stderr when `database.url` is still the
built-in development default.

### `ferroehr db …`

```text
ferroehr db migrate   # apply the embedded migrations and exit
ferroehr db verify    # verify, issuing no DDL, that the schema matches this build
```

These exist so a least-privilege deployment can separate the two database
identities: run `db migrate` once under a DSN that holds DDL rights (a
Kubernetes Job, an init container, a CI/CD stage), then boot the server with
[`database.migrations = "verify"`](config-server.md#database) under a DSN with no DDL rights at
all. `verify` still reads all five `_sqlx_migrations` bookkeeping tables, which
no least-privilege role can do, so point `[database] migrate_url` at the credential
that prepares the schema. See
[Operations](../operations.md#which-credential-prepares-the-schema).

### `ferroehr definitions load`

```sh
ferroehr definitions load person.adls role.adls   # store each file, then exit
```

Stores each file in the definition store, in the order given, before
`serve` starts. An `.opt` file is an OPT 1.4 template and an `.adls` file an
ADL 2 archetype or template, whose parents and slot fillers come before it in
the list. Each file is validated and versioned exactly as the REST upload
(`POST /definition/template/adl1.4` or `/adl2`) would, and the command stops
at the first refusal, naming the file. A file whose id is already stored with
the same text is left alone, so the step runs at every start; one stored
under the same id with other text is refused, because a stored definition
never changes and a changed one takes a new version in its id.

A deployment that maps its professionals to demographic parties runs it
first, so the [party mapping](config-auth.md#the-party-mapping) finds its
templates when the server boots: as a one-shot compose service, a Helm init
container or a setup step. The repository's
`deploy/definitions/professional/` holds the PERSON and ROLE pair the
development lanes and the quickstart load, adapted from openEHR CKM
archetypes; a production deployment loads the templates its own modelling
chose.

### `ferroehr party-binding refill`

```sh
ferroehr party-binding refill   # rebuild the party binding side table under the configured mapping
```

Rebuilds the binding side table from the party versions under the
configured [party mapping](config-auth.md#the-party-mapping) and stores the
mapping's digest. Run it with every replica stopped after a mapping change:
it refuses while a replica holds a live lease, and refuses a store in which
two parties carry one selector value, naming both.

### `ferroehr party-binding check`

```text
ferroehr party-binding check [--admin <PARTY_ID>]... [--admin-key <ISSUER> <KEY>]...
                             [--migration <COMMITTER_NAME>]...
                             [--expected <FILE> [--expected-issuer <ISSUER>]]
```

The pre-flight of a migration to party binding. It reads the party versions
under the configured [party mapping](config-auth.md#the-party-mapping) and
writes nothing: the side-table categories are read from a refill inside a
transaction that is rolled back, so it runs before any boot fill and beside a
running server. It connects with the configured database credentials and
runs no migration.

The report is a JSON document on stdout:

- `issuers`: per `identifier_issuer`, the `PERSON`s that carry a binding key;
- `duplicates`: every binding key, `userName` or `externalId` two `PERSON`s
  carry, which a fill refuses;
- `inactive` and `abandoned`: the key-bearing `PERSON`s whose head is
  `800|inactive|` or `801|abandoned|`;
- `side_table.provisional`: the provisional `PERSON`s that
  [just-in-time creation](config-auth.md#just-in-time-creation) created more
  than `[authz.party_binding] provisional_report_after` days ago (default 30)
  and that SCIM or an administrator has not completed, by container id and
  creation time, oldest first, with no name or other personal data;
- `side_table.unadopted`: the key-bearing `PERSON`s that perform no
  professional `ROLE`, which `adopt` has not reached;
- `side_table.basic_users`: every `[[authz.party_binding.basic_user]]` with
  the `PERSON` it binds today, or none;
- `provenance`: for each key, the committer of the version that introduced
  it and who that committer is under the provenance rule;
- `expected`: with `--expected`, every login of the file (one per line) that
  no live `PERSON` carries.

A key passes the provenance rule when the version that introduced it was
committed by a `[[auth.scim.client]]` (its client identifier under its token
issuer), by an administrator, by a declared migration, or by just-in-time
creation at a first login of an `[[authz.party_binding.issuer]]` with
`jit = true` (a `249|creation|` version carrying its description, committed
by the new `PERSON` itself). A stored version
records no role its committer held, so you name the administrators: by the
id of their `PERSON` with `--admin`, or by the `identifier_issuer` and value
of a key their `PERSON` carries with `--admin-key`. `--migration` declares a
migration by the committer name its versions carry. The report repeats what
you named.

The command exits non-zero, after printing the report, on a duplicate, a
mapped Basic user or expected login with no `PERSON`, or a key whose committer
fails the provenance rule.

### `ferroehr party-binding adopt`

```text
ferroehr party-binding adopt (--admin <PARTY_ID> | --admin-key <ISSUER> <KEY>)...
                             [--migration <COMMITTER_NAME>]... <PARTY_ID>...
```

Gives each listed `PERSON` a professional `ROLE`, so a prepared `PERSON` of
an existing deployment binds: a `ROLE` of the mapping's
`[authz.party_binding.professional_role]` template with the `PERSON` as
`performer`, and a new `PERSON` version listing it in `roles`. Every
adoption of one run is one demographic CONTRIBUTION, committed by the first
administrator you name (a `PARTY_IDENTIFIED` committer whose `external_ref`
is that administrator's `PERSON`), and each adopted `PERSON` gets one access
record through the configured `[audit]` trail. Adopt the existing persons
before the first SCIM provisioning run: without the `ROLE`, a `userName eq`
filter finds no User and the identity provider creates a duplicate.

The command applies the provenance rule of `check` with the same
`--admin`, `--admin-key` and `--migration` operator input, and refuses the
whole run, naming them, while any listed `PERSON`'s key was introduced by
another committer. It also refuses a party that is no live key-bearing
`PERSON` of a mapped template, an inactive or abandoned one, and a store in
which two `PERSON`s carry one selector value. A `PERSON` that already
performs the `ROLE` is reported under `already` and left alone. The report
is a JSON document on stdout with the CONTRIBUTION and each new `ROLE`.

### `ferroehr healthcheck`

Asks the running server's `GET /health/readiness` and exits 0 on `200`, 1
otherwise, with the reason on stderr: the container `HEALTHCHECK` and the
Kubernetes exec-probe fallback. A draining server answers `503`, so it reads
unhealthy. The subcommand loads the same configuration as the server (file,
environment, `--set`) and asks the `server.listen` port under
`server.base_path`, on the loopback when the server listens on every
interface. With `[server.tls]` set it speaks TLS and accepts exactly the leaf in
`server.tls.certificate_file`, presenting `server.tls.healthcheck_identity_file`
to a listener that requires a client certificate. It waits three seconds for
the answer.

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
| `build` | version, commit, build date, `rustc`, the active `deployment.spec_profile` and the specification versions it selects, the PostgreSQL target, the audit posture, and the manufacturer |
| `image` | named in the manifest only: a running binary cannot read the digest of the image it was started from, so attach `docker inspect --format '{{index .RepoDigests 0}}' <image>` or the pod's `status.containerStatuses[].imageID` |
| `target` | the CPU architecture and operating system the binary was built for, and whether it is a debug build |
| `features` | the optional integrations compiled in (`fhir`, `multimedia`) |
| `openehr_crates` | each `openehr-*` crate the binary links, with its version |
| `openehr_crates_origin` | `published` in a release build, whose `openehr_crates` versions are the ones that release published to crates.io; `source_tree` in any other build, a pre-release included, whose versions are the source tree's manifests |
| `licence` | the licence summary `GET /management/status` serves (state, use, licensee, last valid day, what the configured token contributed); never the token |
| `deployment` | the deployment profile, the open gaps and the accepted ones, measured over the databases, or from the configuration alone when they do not answer |
| `schema` | for each database, the highest migration version applied and the highest this build embeds, per schema |
| `configuration` | the effective configuration as `config check` prints it |

The crate versions on `main` are a floor below the line on crates.io: the
release that publishes the crates picks the version and writes it into its
checkout before it builds the server, so only a release build can name the
published version. A pre-release publishes no crates, and a build from source
names the version its manifests hold, which may be an older published version
than the code it links. Read `openehr_crates_origin` before you look a version
up on crates.io.

Nothing in it identifies a person: it reads no clinical, demographic or audit
table. Every secret in the configuration shows `***`, every URL shows `***` in
place of its user name and password, and a database error quoted as a reason
has each configured database user name and password masked too. A database
that does not answer within 15 seconds leaves the `schema` part out and names
it `unavailable` with `unreachable: …` as the reason.

## Zero-config boot and the production checklist

With no file and no environment, the effective configuration is: listener
`0.0.0.0:8080` at the ITS-REST base path with Swagger UI, reading bodies up to
64 MiB; the database at the
built-in development DSN with migrations applied at boot; RBAC on; signing on in
`digest` mode with read-time verification strict; the audit trail on with only
the local store; no caller rate limit; logs in `auto` format at `info`; **and every
integration off**. The one outbound connection on by default is the
[usage report](../usage-report.md), which `usage_report.enabled = false`
switches off.

One thing that configuration does *not* do is serve requests.
`auth.enabled` defaults to `true`, and **authentication enabled with no
mechanism configured is a boot error** rather than a running server that refuses
everything: RFC 9110 §11.6.1 requires a `401` challenge to name a scheme
applicable to the resource, and a server with no mechanism has none: it could
only refuse every request while advertising a scheme it does not implement. The
error names the three ways out: add `[[auth.basic.users]]`, add an `[auth]`
issuer, or set `auth.enabled = false` for development. So a bare `docker run` of
the image with no configuration stops at startup with that message, while the
downloadable Compose quickstart boots because it ships a user.

For production, set at least:

- **`database.url`:** the real DSN, via `FERROEHR__DATABASE__URL` from a secret or a
  `url_file`-mounted value, never inline in a world-readable file. Leaving the
  development default in place is warned about loudly at every boot.
- **`database.migrations = "verify"`** with the schema applied out of band, so the
  serving role needs no DDL rights, plus **`database.migrate_url`** naming the
  credential that reads every schema's migration state at boot. The serving
  credential cannot.
- **`deployment.profile = "production"`** once the deployment holds real
  personal data. The server then refuses to start while a separation is open
  and not accepted by name, instead of running as a sandbox that looks
  identical.
- **an authentication mechanism:** a Basic user store and/or an `[auth]` issuer.
- **`telemetry.format = "json"`** for cluster log collectors.
- **`api.cors`** stays `false`; **`api.openapi_ui`** per
  posture.
- **`api.system_id`:** this deployment's own openEHR system identifier.
  Choose it before the first EHR is created: it is stored with every EHR, audit
  entry and version identifier, and changing it later never rewrites what is
  already committed.
- **`management.*`** per posture. A dedicated `management.listen` is recommended
  so the introspection surface is never reachable on the clinical listener, and
  every endpoint stays `off` until you name a level for it.
- **TLS everywhere a transport supports it:** `server.tls` (or a
  TLS-terminating ingress), HTTPS for the object store; the
  audit syslog feed always speaks TLS.
- **real secrets via the environment or a `*_file` sibling**, never inline.

Before a deployment holding real patient data goes live, work through the
[go-live checklist](../security/go-live-checklist.md): it turns each item above
into something you run and read back, and adds the privacy and audit checks a
controller is asked for.

## What belongs in a mounted file (versus the environment)

The environment sets an array of tables only by element index
(`FERROEHR__AUTH__ISSUER__0__ISSUER`), so the **issuers**, the **party
tables** and the **Basic-auth user store** read more clearly from a file.

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
pointer), the build-stamp variables, and the Compose parameterization (image tags, host ports,
CPU and memory limits). They keep a single `_` by design, which is exactly what
distinguishes them from configuration keys, and why a single-underscore
misspelling of a real key is reported at boot with the uniform spelling it
should have had.
