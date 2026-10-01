# Docker Compose

Docker Compose is the quickest way to run FerroEHR together with a
preconfigured PostgreSQL 18, for local development and evaluation. The
quickstart is **one file and zero configuration**: download
`docker-compose.yml`, run `docker compose up`, and the published images are
pulled and started: no repository checkout, no bind mounts, no environment
variables. This chapter describes the three published images, the services, the
authentication posture the quickstart ships with, the optional profiles and
overlays, and the variables that tune them. For a step-by-step first run, see
[Getting started](../getting-started.md).

> [!NOTE]
> This chapter applies equally to the **standalone** `docker-compose.yml`
> downloaded into an empty directory and to a bare `docker compose up` in a
> repository checkout — both run the published images. Building from source
> is an explicit opt-in (`-f docker-compose.yml -f docker-compose.dev.yml`) —
> see [Repository development](#repository-development) below.

<!-- toc -->

> [!NOTE]
> The quickstart file carries the server configuration **inline** (a Compose
> `configs` entry with `content:`), which requires **Docker Compose 2.23.1 or
> newer**. Check with `docker compose version`.

## The three images

FerroEHR publishes three container images to GHCR:

| Image | Contents |
|---|---|
| `ghcr.io/ferrohealth/ferroehr` | The `ferroehr` server binary on a distroless, non-root, shell-less multi-arch base (amd64 + arm64). Configured by a mounted TOML file and/or `FERROEHR__*` environment variables. |
| `ghcr.io/ferrohealth/ferroehr-postgres` | `postgres:18.6` (with Debian security updates applied at image build) plus init scripts that pre-create the application login role, the eight `NOLOGIN` group roles (`ferroehr_migrator`, `ferroehr_clinical`, `ferroehr_clinical_reader`, and the five domain roles `ferroehr_clinical`, `ferroehr_party`, `ferroehr_clinical_reader`, `ferroehr_party_reader`, `ferroehr_linkage`), the database, the schemas (`clinical`, `ext`, `audit`) and the one extension the schema needs, `btree_gist`, so the app role never needs superuser. |
| `ghcr.io/ferrohealth/ferroehr-viewer` | The [viewer](../viewer/index.md), a standalone web application that talks to the CDR strictly over ITS-REST. Optional; see the `viewer` profile below. |

Each image is published under several tags:

| Tag | Published from |
|---|---|
| `X.Y.Z`, `X.Y` | every release |
| `latest` | the newest release |
| `main` | every push to the default branch |
| `sha-<commit>` | every push, for exact pinning |

The quickstart Compose file pins the **exact release version it shipped with**
on all three images, so a downloaded file always runs one known-good, mutually
compatible set; a CI guard (`scripts/checks/compose-image-tags.sh`) fails the
build whenever those pinned tags disagree with the version being released, so
they cannot be forgotten at a cut. To run something else, set the image
variables in [the table below](#variables-the-compose-files-read).

The role the server connects as in the quickstart owns the database and is a
member of `ferroehr_migrator` and `ferroehr_clinical`, which is what lets it apply
migrations at boot, and of `ferroehr_clinical` and `ferroehr_party`, so the
schema separation is exercised on one credential. That single credential is
deliberate: one container with one DSN cannot demonstrate the credential
separation honestly.

A least-privilege deployment sets `db.migrate = "verify"`, runs
`ferroehr db migrate` out of band under the migrator DSN, and serves on a
narrow runtime credential. It also has to name the credential that prepares
the schema, in `[db] migrate_url`: preparation reads all five
`_sqlx_migrations` bookkeeping tables even under `verify`, which no
least-privilege role can do, `ferroehr_clinical` included. Unset, `migrate_url`
falls back to `[db] url`, which is what the quickstart runs. See
[Operations → Which credential prepares the
schema](../operations.md#which-credential-prepares-the-schema) and
[Applying migrations](../operations.md#applying-migrations).

The PostgreSQL image is **init-scripts only**: it creates roles, schemas and
`btree_gist`, and bakes in no migration state. That extension is the only one
the schema needs, and the server installs it itself at boot
(`CREATE EXTENSION IF NOT EXISTS btree_gist`), so a plain PostgreSQL 18 with a
superuser-capable migration credential serves just as well. The server owns the schema content
and applies its migrations idempotently at every boot, so a fresh database
self-provisions and a restart is a no-op.

> [!NOTE]
> PostgreSQL init scripts run **only when the data volume is empty**. If you
> see startup notices like `skipping role creation (no CREATEROLE privilege)`
> or `roles absent`, your volume predates the image's role setup (or you are
> running a plain `postgres` image): either recreate the volume
> (`docker compose down -v`, which **destroys data**) or create the group roles
> once by hand as a superuser
> ([Operations](../operations.md#turning-the-schema-split-into-a-role-split)
> has the statements). The server runs fine either way: the
> grants are a defense-in-depth layer, not a functional requirement.

## Bringing up the stack

Download `docker-compose.yml` (attached to every
[release](https://github.com/FerroHEALTH/FerroEHR/releases/latest)) into an
empty directory and start it:

```shell
docker compose up
```

This pulls and starts the two core services (no profile needed):

- **`ferroehr-postgres`:** the database image, with a named data volume, a
  `pg_isready` healthcheck, `pg_stat_statements` preloaded, and a modest tuning
  floor (shared buffers, WAL size, work memory, lz4-compressed WAL full-page
  images — which cut per-commit WAL volume roughly in half — and a larger
  `/dev/shm` than Docker's default, which otherwise starves parallel workers).
- **`ferroehr`:** the server, which waits for the database to report healthy
  (`depends_on: condition: service_healthy`), then boots, migrates, and serves
  on port 8080. Its healthcheck is the binary's own `healthcheck` subcommand,
  because the image has no shell.

The API is then at `http://localhost:8080/ferroehr/rest/openehr/v1`, with
Swagger UI at `http://localhost:8080/ferroehr/rest/swagger-ui` (the browser asks
for the API credential; `server.swagger_ui` chooses who may read it).

> [!IMPORTANT]
> **Every published port binds the loopback interface by default**
> (`127.0.0.1:HOST:CONTAINER`), and that is a security property rather than a
> style choice: a published port is DNAT'd by rules Docker inserts *ahead* of
> the host firewall's own chains, so a `ufw deny 8080` does not stop a port
> published on `0.0.0.0`. To reach the stack from another machine, name the
> interface explicitly (`FERROEHR_BIND_HOST=0.0.0.0 docker compose up`, or a
> single address). The database publishes no host port at all — see
> [Connecting to the database](#connecting-to-the-database).

> [!NOTE]
> Third-party base images are pinned by digest (`name:tag@sha256:…`), not by a
> mutable tag, so a pull always resolves the exact same image. Each service also
> declares a CPU and memory ceiling (`deploy.resources.limits`), so a local
> stack cannot exhaust the host; the server's and the database's are overridable
> for constrained machines.

> [!WARNING]
> PostgreSQL 18's official image stores data in a major-version subdirectory,
> so the data volume mounts at `/var/lib/postgresql` (the parent), **not** the
> pre-18 `/var/lib/postgresql/data`. The bundled Compose file already does this
> correctly; keep the convention if you adapt it.

## Container engines: Docker and Podman

The quickstart runs on Docker and on Podman. Both were verified first-hand on
2026-08-24: the core stack, the `viewer` and `s3` profiles, and both
overlays boot to healthy under `podman compose` exactly as under
`docker compose`, with the same commands:

```shell
podman compose up -d --wait
```

Podman-specific notes:

- `podman compose` delegates to an external Compose provider. With Docker's
  own Compose CLI installed (the common case on a machine that has ever run
  Docker Desktop) you get identical Compose behaviour; with the Python
  `podman-compose` package, flag coverage differs — prefer the Docker Compose
  provider.
- The default `podman machine` on macOS ships with 2048 MiB of memory. That
  is enough for the quickstart and both overlays, but not for building the
  server image from source inside the VM (a single spec-crate compile at
  release optimization holds more than that). If you want the
  build-from-source developer path under Podman, resize the machine first:
  `podman machine set --memory 8192`.
- Building from source under Podman (Buildah) is not part of the verified
  quickstart surface; the supported quickstart path is the published images.

The repository's own CI and measurement lanes run Docker; the compose-driven
scripts (`scripts/conformance.sh`, the deployment probe, the UI journey
battery) are exercised against Docker only.

## The isolation posture

Every service in every committed compose file drops **all** Linux capabilities
and adds back only what its entrypoint provably needs, sets
`no-new-privileges:true`, bounds its file descriptors, restarts unless stopped,
and keeps Docker's default seccomp profile (the posture is the *absence* of an
`unconfined` override, so that absence is what a CI guard checks). No service is
privileged and none mounts the Docker daemon socket.

Most services additionally run a read-only root filesystem with a tmpfs for the
paths that must be writable. Two do not, and each says why in place: the S3
gateway writes its volume store, and the server container hits a real mutual
exclusion rather than an oversight: Compose refuses an inline `config` in a
read-only service, and that inline config is what makes this file standalone. A
deployment that mounts its configuration from a file instead can add
`read_only: true` and a tmpfs at `/tmp`. On Kubernetes the conflict does not
arise: the configuration arrives as a mounted ConfigMap and the chart sets
`readOnlyRootFilesystem: true`.

## The quickstart's authentication posture

The server configuration travels inside the Compose file and is written for
evaluation, not for production:

- **Basic auth with one user:** `ferroehr` / `ferroehr` (stored as an Argon2id
  hash), carrying the `ADMIN` and `USER` roles.
- **RBAC disabled:** so any authenticated caller may use every enabled
  surface, including the admin API. The `ADMIN` / `USER` / `READONLY`
  separation is switched on by setting `[authz.rbac] enabled = true` and giving
  each user an explicit `roles` list; see
  [Security](../security.md).
- **Admin API enabled:** so the optional viewer's panels work.
- **Management introspection enabled**, with `info` and `metrics` at `private`,
  `prometheus` at `public`, and `env` and `loggers` at `admin_only`.
- **Permissive CORS:** any origin may call the API from a browser.
- **No TLS:** plain HTTP on port 8080.
- **The default `sandbox` deployment profile:** the server names every
  separation it has not made on the boot banner, in the log and on
  `GET /ferroehr/rest/status`, and the viewer raises a notice saying the
  deployment must not hold real patient data. That is the quickstart telling
  the truth about itself, not a fault. See
  [`deployment_profile`](configuration.md#deployment_profile).

> [!WARNING]
> These are development credentials and development defaults. Before exposing
> a server, replace the user store (or point it at an identity provider), turn
> RBAC on, restrict CORS, close the admin API, and terminate TLS in front of the
> server. The [configuration reference](configuration.md) and
> [Security](../security.md) cover each of these.

To change any of it without editing the Compose file, add `FERROEHR__*`
variables to the `ferroehr` service's `environment:` block; the environment
layer wins over the inline file. The Basic-auth user store is the one setting
the environment cannot express (it is an array of tables), so replacing the
users means editing the inline config or mounting your own TOML file.

## The OIDC variant (Keycloak overlay)

A second standalone file adds a ready-made identity provider, so you can
exercise bearer-token authentication without registering a client anywhere.
Download `docker-compose.keycloak.yml` from the same release, beside the base
file, and stack the two:

```shell
docker compose -f docker-compose.yml -f docker-compose.keycloak.yml up
```

That starts **Keycloak** on port 8081 with a small demo realm (`ferroehr`)
defined inline and points the server's bearer validation at it. Basic auth from
the base file keeps working, so the server then advertises `Basic, Bearer`.

The overlay also **turns RBAC on**, deliberately: an identity provider whose
roles decide nothing is a login screen, not authorization. So the realm ships
four users whose roles are load-bearing:

| User | Password | Realm roles | What it demonstrates |
|---|---|---|---|
| `ferroehr` | `ferroehr` | `ADMIN`, `USER` | the full surface, admin API included |
| `clinician` | `clinician` | `USER` | the clinical API without the admin routes |
| `auditor` | `auditor` | `USER`, `READONLY` | reads succeed, every write is refused `403` |
| `nobody` | `nobody` | none | an authenticated caller with no grant |

One confidential client is defined (`ferroehr` /
`ferroehr-quickstart-secret`, with the password grant enabled so a token can be
fetched by curl). Fetch a token and call the API with it:

```shell
TOKEN=$(curl -s -d client_id=ferroehr -d client_secret=ferroehr-quickstart-secret \
  -d username=ferroehr -d password=ferroehr -d grant_type=password \
  http://localhost:8081/auth/realms/ferroehr/protocol/openid-connect/token \
  | jq -r .access_token)

curl -H "Authorization: Bearer $TOKEN" -X POST -i \
  http://localhost:8080/ferroehr/rest/openehr/v1/ehr
```

> [!WARNING]
> The realm, the client secret and the user passwords are demo values, served
> over plain HTTP by a Keycloak in `start-dev` mode, which is also why the
> overlay sets the server's `allow_insecure_issuer` opt-in. For a real
> deployment, drop this overlay and point `[auth.oidc]` (or
> `FERROEHR__AUTH__OIDC__*`) at your own issuer over HTTPS.

## The terminology overlay (FerroTERM)

[FerroTERM](https://github.com/rubentalstra/FerroTERM) is the Ferro family's
FHIR terminology server: R4, R4B and R5 endpoints over a precomputed index, no
JVM, no database, one distroless image. The overlay
`docker-compose.terminology.yml` starts it beside the CDR and switches the CDR's
external terminology on, so archetype value-set bindings resolve at commit and
AQL `TERMINOLOGY()` expands through it (the `/terminology/*` extension routes
answer from it too once `[terminology] api_enabled = true`, which is off by
default):

```shell
docker compose -f docker-compose.yml -f docker-compose.terminology.yml up
curl 'http://localhost:8090/r4b/metadata?mode=terminology'
```

It is an overlay rather than a profile for the same reason as the Keycloak and
observability ones: enabling it has to change the CDR's environment, which a
profile cannot do. FerroTERM's image carries its own health probe (the binary's
`healthcheck` subcommand, since the distroless image has no shell), and the CDR
is declared to start only once that probe reports healthy, so `docker compose up
--wait` returns with both serving.

Out of the box FerroTERM serves the licence-free shaped seed this repository
ships (two code systems and two value sets under the reserved `example.test`
domain, the content the conformance lane binds to), mounted read-only straight
out of the `ferroehr` image. No licensed terminology is distributed with
FerroEHR or FerroTERM. To serve a release you hold a licence for, build its
index once into the named volume and tell the server to open it:

```shell
FERROTERM_RF2=/path/to/SnomedCT_Release.zip \
  docker compose -f docker-compose.yml -f docker-compose.terminology.yml \
  --profile terminology-build run --rm ferroterm-build
FERROTERM_INDEX=/data/index \
  docker compose -f docker-compose.yml -f docker-compose.terminology.yml up
```

The archive is mounted read-only and unpacked to a tmpfs that dies with the
build container. FerroTERM's [loading page](https://ferroterm.eu/docs/operate/loading-snomed.html)
covers the other code systems and their build flags.

Three things the operator owns:

- **The code-system licence.** SNOMED CT needs an Affiliate Licence (free in
  Member countries, see [snomed.org/get-snomed](https://www.snomed.org/get-snomed));
  LOINC, ICD and RxNorm have their own terms. Every surface that shows SNOMED CT
  content carries the notice its licence prescribes (clause 8.3.1) and the
  release version and date.
- **Who can reach the server.** The overlay publishes FerroTERM on the loopback
  interface at port 8090 (`FERROEHR_TERMINOLOGY_PORT`) so you can query it while
  developing. FerroTERM has no authentication and no rate limit of its own; a
  deployment that serves other machines puts a reverse proxy with both in front,
  or removes the `ports:` block and lets the CDR be the only caller, which is
  what the [hosted sandbox](https://sandbox.ferroehr.eu) does.
- **FerroTERM's licence.** BUSL-1.1 from the same Licensor as FerroEHR:
  non-commercial production use is free, any other production use needs a
  commercial licence. Its README says which is which.

The CDR keeps its shipped fail-open posture: a binding the server cannot resolve
is accepted. `FERROEHR__TERMINOLOGY__EXTERNAL__FAIL_ON_ERROR=true` turns an
unreachable server into a `422` refusal instead; the
[terminology page](../beyond-core/terminology.md#when-the-terminology-server-cannot-answer)
explains both. A code the server resolves as outside its value set is refused
with `422` under either posture.

## Optional services (Compose profiles)

The core services are profile-less and start on every `up`. Further services
sit behind a [Compose
profile](https://docs.docker.com/compose/how-tos/profiles/) and stay down until
you ask for them:

- **`ferroehr-viewer`** (`--profile viewer`): the
  [viewer](../viewer/index.md) on port 3000, pointed at the server
  inside the Compose network. Start the stack with it:

  ```shell
  docker compose --profile viewer up
  # → http://localhost:3000  (log in with ferroehr / ferroehr)
  ```

- **`seaweedfs`** and **`seaweedfs-init`** (`--profile s3`): an S3 gateway for
  large `DV_MULTIMEDIA` externalization (development and test only). Point the
  server at it and bring the profile up:

  ```shell
  export FERROEHR__MULTIMEDIA__ENABLED=true
  export FERROEHR__MULTIMEDIA__ENDPOINT=http://seaweedfs:8333
  export FERROEHR__MULTIMEDIA__BUCKET=openehr-multimedia
  export FERROEHR__MULTIMEDIA__ALLOW_HTTP=true    # dev only; production S3 is HTTPS

  docker compose --profile s3 up -d --wait ferroehr seaweedfs
  ```

  The compose file passes the whole `FERROEHR__MULTIMEDIA__*` set through from
  your shell, so there is no file to edit, and `seaweedfs-init` creates the
  bucket by issuing an idempotent `CreateBucket` against the gateway, which
  ships with none, and answers an S3 write into a missing bucket with
  `403 AccessDenied`, a reply that reads as a credentials problem and is not
  one. It reads the same bucket variable the server does, so the two cannot
  drift.

  To turn it off again, `unset` those variables (or set
  `FERROEHR__MULTIMEDIA__ENABLED=false`) and re-up: an unset variable is
  *removed* from the container's environment rather than passed as empty, so
  the server falls back to its own default of `enabled = false`.

  Confirm the server took them with
  `curl -s -u ferroehr:ferroehr http://localhost:8080/management/env | jq .multimedia`
  (`"enabled": true` and a non-empty `endpoint` mean the wiring is right). In
  production, point the multimedia settings at a real, credentialed, HTTPS S3
  endpoint instead; see [S3 multimedia](../beyond-core/s3-multimedia.md).

- **`ferroehr-backup-clinical`, `ferroehr-backup-party` and
  `ferroehr-backup-linkage`** (`--profile backup`): one `pg_dump` job per
  pseudonymisation domain, run on demand rather than started with the stack:

  ```shell
  docker compose --profile backup run --rm ferroehr-backup-clinical
  docker compose --profile backup run --rm ferroehr-backup-party
  docker compose --profile backup run --rm ferroehr-backup-linkage
  ```

  Each writes a timestamped custom-format dump into its own host directory
  (`FERROEHR_BACKUP_CLINICAL_DIR`, `FERROEHR_BACKUP_PARTY_DIR`,
  `FERROEHR_BACKUP_LINKAGE_DIR`, defaulting to `./backups/clinical`,
  `./backups/demographic` and `./backups/linkage`). Set
  `FERROEHR_BACKUP_USER="$(id -u):$(id -g)"` and the files land owned by you;
  unset, the job runs as root inside the container with one capability,
  `DAC_OVERRIDE`, which is what writing a directory it does not own actually
  requires.

  Three dumps rather than one is the point: a single whole-database dump holds
  the clinical record, the identities of its subjects and the map between them,
  and whoever reads that file re-identifies every record in it. This stack runs
  one login credential, so it separates the artefacts and not the authority to
  produce them; [Operations → Dump each domain
  separately](../operations.md#dump-each-domain-separately) is the production
  procedure, including why a backup credential needs `BYPASSRLS`.

Two things that used to be profiles of this file are not any more. Both have to
*change* the server's configuration to be useful, which a profile cannot do, so
each is a separate file instead:

- **OIDC / Keycloak** is the `docker-compose.keycloak.yml` overlay above (and,
  in a repository checkout, a `keycloak` profile of the development override
  described below).
- **A terminology server** is the `docker-compose.terminology.yml` overlay
  [above](#the-terminology-overlay-ferroterm), FerroTERM beside the CDR; the
  repository's conformance stack additionally runs its own seeded server for
  the acceptance lane, see [Terminology servers](../beyond-core/terminology.md).

## Port already in use?

Every published port is a variable, so a conflict never requires editing any
file. When `docker compose up` refuses with `port is already allocated` (or
`address already in use`), pick a free port and pass it:

```shell
FERROEHR_PORT=8081 docker compose up
```

The same works for the other ports (`FERROEHR_VIEWER_PORT`,
`FERROEHR_S3_PORT`; PostgreSQL publishes no host port — see
[Connecting to the database](#connecting-to-the-database)). The defaults stay
fixed on purpose: every URL in this book assumes `localhost:8080`, and
Docker's automatic ephemeral-port allocation exists only when a mapping
[omits the host port entirely](https://docs.docker.com/engine/network/port-publishing/)
— a server that silently moved ports would break every printed URL, so here
you always choose the port and always know it.

> [!NOTE]
> **On Windows** the refusal reads differently: `Ports are not available:
> exposing port TCP 127.0.0.1:8080 … bind: Only one usage of each socket
> address … is normally permitted` when another program holds the port, or
> `… An attempt was made to access a socket in a way forbidden by its access
> permissions` when the port sits in a Windows reserved range. The remedy is
> the same variable; to see the reserved ranges, run
> `netsh interface ipv4 show excludedportrange protocol=tcp`. (The classic
> Windows collision — a natively installed PostgreSQL service holding 5432 —
> no longer affects the quickstart, which publishes no database port.)

## Connecting to the database

The quickstart publishes no PostgreSQL host port: the server reaches the
database over the compose network, so nothing needs one, and a published
5432 collided with natively installed PostgreSQL (on Windows the installer
registers an auto-started `postgresql-x64-*` service). For a psql session,
no port is needed at all:

```shell
docker compose exec ferroehr-postgres psql -U postgres -d ferroehr
```

For a GUI client on the host (pgAdmin, DBeaver), add the `db-publish`
overlay, downloaded from the same release beside the base file:

```shell
docker compose -f docker-compose.yml -f docker-compose.db-publish.yml up
```

It publishes `127.0.0.1:5432` (retune with `FERROEHR_DB_PORT`, widen with
`FERROEHR_BIND_HOST` — the warning above applies).

## Variables the compose files read

Set these in your shell (or an `.env` file) to retune without editing anything:

| Variable | Default | Effect |
|---|---|---|
| `FERROEHR_IMAGE` | `ghcr.io/ferrohealth/ferroehr:<release>` | Server image to run. |
| `FERROEHR_POSTGRES_IMAGE` | `ghcr.io/ferrohealth/ferroehr-postgres:<release>` | Database image to run. |
| `FERROEHR_VIEWER_IMAGE` | `ghcr.io/ferrohealth/ferroehr-viewer:<release>` | Viewer image (the `viewer` profile). |
| `FERROEHR_BIND_HOST` | `127.0.0.1` | Host interface every published port binds. |
| `FERROEHR_PORT` | `8080` | Host port mapped to the server. |
| `FERROEHR_VIEWER_PORT` | `3000` | Host port mapped to the viewer. |
| `FERROEHR_DB_PORT` | not published | Host port for PostgreSQL, read only by the `db-publish` overlay (default `5432` there). |
| `FERROEHR_S3_PORT` | `8333` | Host port mapped to the S3 gateway (the `s3` profile). |
| `FERROEHR_BACKUP_CLINICAL_DIR` | `./backups/clinical` | Host directory the clinical dump job writes to (the `backup` profile). |
| `FERROEHR_BACKUP_PARTY_DIR` | `./backups/party` | Host directory the party dump job writes to. |
| `FERROEHR_BACKUP_LINKAGE_DIR` | `./backups/linkage` | Host directory the linkage dump job writes to. |
| `FERROEHR_BACKUP_USER` | `0:0` (root in the container) | `uid:gid` the dump jobs run as. Set it to `$(id -u):$(id -g)` and the files land owned by you. |
| `FERROEHR_CPUS` / `FERROEHR_MEM` | `4` / `4G` | Server container resource ceiling. |
| `FERROEHR_DB_CPUS` / `FERROEHR_DB_MEM` | `4` / `4G` | Database container resource ceiling. |
| `PG_INIT_USER` / `PG_INIT_PASSWORD` / `PG_INIT_DB` | `ferroehr` | App role, password, and database created by the DB image's init script. |
| `POSTGRES_PASSWORD` | `postgres` | Bootstrap superuser password (init only). |
| `FERROEHR__LOG__FORMAT` | `pretty` | Log rendering for `docker compose logs`. Set `json` for log collectors. |
| `FERROEHR__LOG__FILTER` | `info` | Log level filter. |
| `FERROEHR__DB__MAX_CONNECTIONS` | `10` | Server connection-pool ceiling. |
| `FERROEHR__SERVER__MAX_IN_FLIGHT` | `256` | In-flight request admission cap (`503` past it; `0` disables). |
| `FERROEHR__SIGNING__ENABLED` | `true` | Version signing. |

A commercial licence token is not a passthrough variable: mount the signed
token file into the server container read-only and name its path with
`FERROEHR__LICENCE__FILE` (the compose file carries a commented example beside
`FERROEHR__SIGNING__ENABLED`). Unset, the server runs under the embedded
non-commercial grant and reports which on `GET /ferroehr/rest/status`.

The Keycloak overlay adds `KEYCLOAK_PORT` (default `8081`),
`KEYCLOAK_HOSTNAME`, `KEYCLOAK_ADMIN_USER` and `KEYCLOAK_ADMIN_PASSWORD` (both
`admin`); the observability overlay adds `GRAFANA_PORT` (default `3000`). The
`FERROEHR__MULTIMEDIA__*` set is declared with no defaults on purpose, so an
unset key reaches the server as absent rather than as an empty string.

The server container is passed `FERROEHR__DB__URL` (assembled from the database
variables), and its configuration file is the inline quickstart config,
delivered to `/etc/ferroehr/ferroehr.toml` where the server auto-discovers it.
Any other setting from the [configuration reference](configuration.md) can be
added under the `ferroehr` service's `environment:` block and takes precedence
over the file. Every variable uses the same `FERROEHR__…` grammar as the rest of
that reference; a single-underscore spelling is rejected at startup, not
ignored.

## Repository development

A bare `docker compose up` in a checkout runs the same published-images
quickstart as the downloaded file. Building from the current sources is an
explicit opt-in: the `docker-compose.dev.yml` overlay, passed as a
[second file](https://docs.docker.com/compose/how-tos/multiple-compose-files/),
switches the stack to the from-source developer posture:

```shell
docker compose -f docker-compose.yml -f docker-compose.dev.yml up --build
```

That builds the server, database and viewer images from the current sources
(the `:local` tags) instead of pulling published ones, and replaces the inline
quickstart configuration with `docker/ferroehr.dev.toml`: three Basic users
(`ferroehr`, `ferroehr-admin`, `ferroehr-readonly`, all with password
`ferroehr`), **RBAC enabled** so the role separation is exercised, and trust for
the development Keycloak realm. The override also defines a `keycloak` profile
that imports the full development realm
(`docker compose --profile keycloak up`).

Downloaders of the standalone quickstart file never see any of this; it is
purely a convenience for working on FerroEHR itself, and it only ever
applies when its `-f` is passed — nothing merges it in silently.

> [!TIP]
> A cold from-source build compiles at two parallel rustc jobs by default,
> and a release-optimization compile can hold several gigabytes per job. If
> the build gets OOM-killed on a memory-constrained host (observed on a
> 24 GB machine that was also running another stack), rerun it serially —
> slower, but it completes:
>
> ```shell
> CARGO_BUILD_JOBS=1 docker compose -f docker-compose.yml -f docker-compose.dev.yml build
> ```

## Build provenance

Images built by CI (and any `docker compose build` you drive from a checkout)
embed a build SHA reported at `/management/info` and on the
`ferroehr_build_info` metric. The build does not read `.git`; instead the SHA is
passed as the standard `REVISION` build argument, the same value that fills the
`org.opencontainers.image.revision` image label (CI uses the commit SHA; the
project's own scripts export `git rev-parse --short=12 HEAD`). The argument is
declared by the files that carry `build:` blocks (the development override and
the conformance stack) not by the pull-only quickstart file. When no value is
supplied the identity falls back to the workspace version with an `unknown`
SHA; the build never fails for lack of it.

## Observability overlay

A further standalone file adds a full local telemetry stack: an OTLP
collector, Prometheus, Tempo, Loki and Grafana with a provisioned
service-overview dashboard. Like the Keycloak overlay it travels inline, so
downloading it beside `docker-compose.yml` is enough:

```shell
docker compose -f docker-compose.yml -f docker-compose.observability.yml up
# Grafana → http://localhost:3000
```

The overlay reconfigures the server for that stack: it exports **traces and
metrics** over OTLP/gRPC (`FERROEHR__TELEMETRY__OTLP_ENDPOINT` plus
`FERROEHR__TELEMETRY__METRICS_PUSH=true`), switches stdout to JSON lines
(`FERROEHR__LOG__FORMAT=json`), and enables the management surface on its own
internal port 9464 with `info`, `metrics` and `prometheus` reachable there. That
port is only published on the Compose network; Grafana's 3000 is the sole
published port.

Metrics are **pushed, not scraped**, and that is a property of the bundled
image rather than a preference: `grafana/otel-lgtm` runs Prometheus with a
config file carrying no `scrape_configs` at all, and receives metrics over OTLP
from its own collector. A scrape job dropped into that image is read by nothing,
which is what an earlier version of this overlay did: every metric panel was
empty with no error anywhere. Since one meter provider feeds both readers, the
push carries every metric family the scrape endpoint would, and
`/management/prometheus` stays reachable on the Compose network for anyone who
wants to compare the two.

Grafana's port 3000 is the same one the viewer would use, but the two
never collide by accident: the viewer only starts when you ask for the
`viewer` profile. If you want both at once, move one of them
(`FERROEHR_VIEWER_PORT=3001` or `GRAFANA_PORT=3001`).

This is the easiest way to see the server's metrics and traces without wiring
up a collector by hand. See [Operations](../operations.md) for what the server
exports and how to consume it in production.

## Next

- [Configuration reference](configuration.md) — every setting you can pass.
- [Kubernetes & Helm](kubernetes.md) — the production deployment.
