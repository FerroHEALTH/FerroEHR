# Docker Compose

Docker Compose is the quickest way to run FerroEHR together with a
preconfigured PostgreSQL 19, for local development and evaluation. The
quickstart is **one file and zero configuration**: download
`docker-compose.yml`, run `docker compose up`, and the published images are
pulled and started: no repository checkout, no bind mounts, no environment
variables. This chapter describes the three published images, the services, the
authentication posture the quickstart ships with, and the variables that tune
them. The overlays and optional profiles are on [Overlays and
profiles](compose-overlays.md). For a step-by-step first run, see
[Getting started](../getting-started.md).

> [!NOTE]
> This chapter applies equally to the **standalone** `docker-compose.yml`
> downloaded into an empty directory and to a bare `docker compose up` in a
> repository checkout; both run the published images. Building from source
> is an explicit opt-in (`-f docker-compose.yml -f docker-compose.dev.yml`),
> see [Repository development](compose-overlays.md#repository-development).

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
| `ghcr.io/ferrohealth/ferroehr-postgres` | `postgres:19.0` (with Debian security updates applied at image build) plus init scripts that pre-create the application login role, the `NOLOGIN` group roles (`ferroehr_migrator` and the four database roles `ferroehr_clinical`, `ferroehr_party`, `ferroehr_clinical_reader`, `ferroehr_party_reader`), the database and the schemas (`clinical`, `ext`, `audit`), so the app role never needs superuser. |
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

A least-privilege deployment sets `database.migrations = "verify"`, runs
`ferroehr db migrate` out of band under the migrator DSN, and serves on a
narrow runtime credential. It also has to name the credential that prepares
the schema, in `[database] migrate_url`: preparation reads all four
`_sqlx_migrations` bookkeeping tables even under `verify`, which no
least-privilege role can do, `ferroehr_clinical` included. Unset, `migrate_url`
falls back to `[database] url`, which is what the quickstart runs. See
[Operations → Which credential prepares the
schema](../database-operations.md#which-credential-prepares-the-schema) and
[Applying migrations](../database-operations.md#applying-migrations).

The PostgreSQL image is **init-scripts only**: it creates roles and schemas,
and bakes in no migration state. The schema needs no extension, so a plain
PostgreSQL 19 serves just as well. The server owns the schema content
and applies its migrations idempotently at every boot, so a fresh database
self-provisions and a restart is a no-op.

> [!NOTE]
> PostgreSQL init scripts run **only when the data volume is empty**. If you
> see startup notices like `skipping role creation (no CREATEROLE privilege)`
> or `roles absent`, your volume predates the image's role setup (or you are
> running a plain `postgres` image): either recreate the volume
> (`docker compose down -v`, which **destroys data**) or create the group roles
> once by hand as a superuser
> ([Operations](../database-operations.md#turning-the-schema-split-into-a-role-split)
> has the statements). The server runs fine either way: the
> grants are a defense-in-depth layer, not a functional requirement.

## Bringing up the stack

Download `docker-compose.yml` (attached to every
[release](https://github.com/FerroHEALTH/FerroEHR/releases/latest), and
pulling that release's images) into an empty directory and start it. The copy
in the repository pulls the `:main` images built from `main` and carries
`main`'s configuration, for running `main`:

```shell
curl -fsSLO https://github.com/FerroHEALTH/FerroEHR/releases/latest/download/docker-compose.yml
docker compose up --wait
```

This pulls and starts the core services (no profile needed): the database
and the server below, and the patient identity source described in
[The patient identity source (FerroPIX)](#the-patient-identity-source-ferropix).

- **`ferroehr-postgres`:** the database image, with a named data volume, a
  `pg_isready` healthcheck, `pg_stat_statements` preloaded, and a modest tuning
  floor (shared buffers, WAL size, work memory, lz4-compressed WAL full-page
  images (which cut per-commit WAL volume roughly in half) and a larger
  `/dev/shm` than Docker's default, which otherwise starves parallel workers).
- **`ferroehr`:** the server, which waits for the database to report healthy
  (`depends_on: condition: service_healthy`), then boots, migrates, and serves
  on port 8080. Its healthcheck is the binary's own `healthcheck` subcommand,
  because the image has no shell.

The API is then at `http://localhost:8080/ferroehr/rest/openehr/v1`, with
Swagger UI at `http://localhost:8080/ferroehr/rest/swagger-ui` (the browser asks
for the API credential; `api.openapi_ui` chooses who may read it).

> [!IMPORTANT]
> **Every published port binds the loopback interface by default**
> (`127.0.0.1:HOST:CONTAINER`), and that is a security property rather than a
> style choice: a published port is DNAT'd by rules Docker inserts *ahead* of
> the host firewall's own chains, so a `ufw deny 8080` does not stop a port
> published on `0.0.0.0`. To reach the stack from another machine, name the
> interface explicitly (`FERROEHR_BIND_HOST=0.0.0.0 docker compose up`, or a
> single address). The database publishes no host port at all; see
> [Connecting to the database](#connecting-to-the-database).

> [!NOTE]
> Third-party base images are pinned by digest (`name:tag@sha256:…`), not by a
> mutable tag, so a pull always resolves the exact same image. Each service also
> declares a CPU and memory ceiling (`deploy.resources.limits`), so a local
> stack cannot exhaust the host; the server's and the database's are overridable
> for constrained machines.

> [!WARNING]
> PostgreSQL 19's official image stores data in a major-version subdirectory,
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
  `podman-compose` package, flag coverage differs; prefer the Docker Compose
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
- **The user is a professional `PERSON`:** the server stores the
  [default demographic definitions](default-demographic-definitions.md) as
  it boots, and the configuration sets no mapping key, so the default party
  mapping names them and SCIM Users, Groups and representation work. The
  `ferroehr-provision` service then creates the user's `PERSON` through
  `/scim/v2/Users`, with a client credentials token the development
  `identity-issuer` signs for the client `quickstart-scim`. It stays up and
  reports healthy once the login is bound, so `docker compose up --wait`
  returns when the user can work.
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
- **The `sandbox` deployment profile, declared in the inline configuration:**
  the binary's default, stated so the file says what it runs. The server names every
  separation it has not made on the boot banner, in the log and on
  `GET /management/status`, and the viewer raises a notice saying the
  deployment must not hold real patient data. That is the quickstart telling
  the truth about itself, not a fault. See
  [`deployment.profile`](configuration.md#deploymentprofile).

> [!WARNING]
> These are development credentials and development defaults. Before exposing
> a server, replace the user store (or point it at an identity provider), turn
> RBAC on, restrict CORS, close the admin API, and terminate TLS in front of the
> server. The [configuration reference](configuration.md) and
> [Security](../security.md) cover each of these.

To change any of it without editing the Compose file, add `FERROEHR__*`
variables to the `ferroehr` service's `environment:` block; the environment
layer wins over the inline file. An array of tables, such as the Basic-auth
user store, is set by element index (`FERROEHR__AUTH__BASIC__USERS__0__USER`),
key by key over the file's; replacing the users is usually clearer in the
inline config or your own mounted TOML file.

## The patient identity source (FerroPIX)

FerroEHR keeps no patient-to-record cross-reference of its own by default. It
asks an IHE patient identity source over PIXm (ITI-83) which pseudonym the
clinical domain holds for a patient
([`[pixm]`](config-integrations.md#pixm)). The quickstart runs
FerroPIX as that source, pinned by tag and image-index digest (CI runs the
`config check` of every pinned image over its inline configuration), with four
services beside the server:

- **`identity-issuer`:** a development OpenID Provider (NAV's
  `mock-oauth2-server`) that signs any client credentials request, at
  `http://localhost:8180/quickstart`. It is not for anything real.
- **`ferropix`:** the PIX Manager, at `http://localhost:8090/fhir`. Its
  configuration declares the pseudonymisation domain `urn:oid:2.999.7.1` with
  FerroEHR's client `ferroehr` as its consumer, so it mints a pseudonym the
  first time FerroEHR asks, and a synthetic identifier domain
  `urn:oid:2.999.1.1` fed by the client `quickstart-source`.
- **`ferropix-postgres`** and **`ferropix-key`:** FerroPIX's own database, and
  a one-shot job that writes its sealing key into a volume, so the file
  carries no key.

The server's `[pixm]` names the endpoint and the grant, and
`[privacy] subject_namespaces` declares the clinical domain. The server reads
FerroPIX's `CapabilityStatement` at boot and does not start until it answers.

The issuer, FerroPIX and FerroEHR share the issuer's network namespace. A
server accepts a plain-`http` issuer only on loopback
([RFC 8414 §2](https://www.rfc-editor.org/rfc/rfc8414#section-2)), and the
mock issuer writes the host it was asked on into every token, so all three
reach it at `http://localhost:8180`. The issuer service therefore publishes the
ports of all three and answers to the names `ferroehr` and `ferropix` on the
compose network.

To register a synthetic patient and ask for the pseudonym by hand:

```shell
SRC=$(curl -s -d grant_type=client_credentials -d client_id=quickstart-source \
  -d client_secret=any -d 'scope=system/Patient.cud' \
  http://localhost:8180/quickstart/token | jq -r .access_token)
curl -s -X PUT -H "Authorization: Bearer $SRC" \
  -H 'Content-Type: application/fhir+json' \
  'http://localhost:8090/fhir/Patient?identifier=urn:oid:2.999.1.1%7CH-1' \
  -d '{"resourceType":"Patient","identifier":[{"system":"urn:oid:2.999.1.1","value":"H-1"}],"name":[{"family":"Synthetic"}]}'
```

Any conformant PIX Manager can take FerroPIX's place: point `[pixm.endpoint]` url,
`assigning_authority` and `[pixm.endpoint.credentials.oauth]` at it.

## The local EHR index

The other supported posture keeps the subject-to-EHR cross-reference in
FerroEHR ([`[pixm]`](config-integrations.md#pixm)): `EHR_STATUS.subject`
names a patient by its own namespace and identifier, and
`GET /ehr?subject_id=…&subject_namespace=…` finds the EHR by them. One
variable selects it, with no copy of the configuration to make:

```shell
FERROEHR_EHR_INDEX=local docker compose up
```

The server then runs the file's `ferroehr-config-local` block, which differs
from `ferroehr-config` in `[pixm]` and `[privacy]` alone: the local map,
and no pseudonym namespace, so a subject reference is stored as the client
sends it:

```shell
curl -u ferroehr:ferroehr -X POST -i -H 'Content-Type: application/json' \
  http://localhost:8080/ferroehr/rest/openehr/v1/ehr -d '{
  "_type": "EHR_STATUS",
  "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
  "archetype_details": {"_type": "ARCHETYPED", "rm_version": "1.1.0",
    "archetype_id": {"_type": "ARCHETYPE_ID", "value": "openEHR-EHR-EHR_STATUS.generic.v1"}},
  "name": {"_type": "DV_TEXT", "value": "EHR Status"},
  "subject": {"_type": "PARTY_SELF", "external_ref": {"_type": "PARTY_REF",
    "namespace": "benchmark", "type": "PERSON",
    "id": {"_type": "GENERIC_ID", "value": "1793131381", "scheme": "benchmark"}}},
  "is_queryable": true, "is_modifiable": true}'
curl -u ferroehr:ferroehr -i \
  'http://localhost:8080/ferroehr/rest/openehr/v1/ehr?subject_id=1793131381&subject_namespace=benchmark'
```

The identity services still start, and nothing asks them. Start without the
variable to return to FerroPIX; the EHRs created on the local index keep their
subject references, and the database refuses no stored row.

## Behind FerroSMART

Three files run FerroEHR behind FerroSMART, the family's SMART App Launch 2.x
authorization server, with an OpenID Connect provider upstream for login only
([Behind FerroSMART](../behind-ferrosmart.md)). They need a checkout, since
they mount FerroSMART's configuration, the upstream realm or bootstrap script
from `docker/ferrosmart/`:

```shell
bash docker/ferrosmart/secrets.sh
docker compose -f docker-compose.yml -f docker-compose.ferrosmart.yml \
  -f docker-compose.ferrosmart-keycloak.yml --profile viewer up -d --wait
```

- `docker-compose.ferrosmart.yml` adds FerroSMART, pinned by digest in
  `docs/VERSIONS.md`, with its own PostgreSQL, at `http://localhost:8280`.
  FerroEHR trusts it as a SMART App Launch issuer, binds people by
  FerroSMART's subject under `urn:example:ferrosmart-login`, and describes it
  in the discovery document.
  The viewer's [Launch button](../smart-app-launch.md#launching-an-app-from-the-viewer)
  registers its EHR launches there, for the example app `example-app`.
  FerroSMART's configuration is `docker/ferrosmart/ferrosmart.toml`
  (`FERROSMART_CONFIG_FILE` names another), its picker looks EHRs up under
  `FERROSMART_SUBJECT_NAMESPACE` (the FerroPIX domain `urn:oid:2.999.7.1` by
  default), and `docker/ferrosmart/secrets.sh` writes its secrets under
  `./secrets/ferrosmart` (`FERROSMART_SECRETS`).
- `docker-compose.ferrosmart-keycloak.yml` runs Keycloak upstream at
  `http://localhost:8380`, with the realm `docker/ferrosmart/keycloak/realm.json`
  and the demo user `clinician`/`clinician`.
- `docker-compose.ferrosmart-zitadel.yml`, in its place, runs ZITADEL at
  `http://localhost:8480` with its login at `http://localhost:3300/ui/v2/login`,
  bootstrapped by `docker/ferrosmart/zitadel/bootstrap.sh`; its header names
  the two secrets it adds.

Every party shares the issuer's network namespace and serves plain `http` on
loopback, which FerroSMART and FerroEHR admit for a loopback issuer only: a
local example, never a deployment. A deployment serves FerroSMART and its
provider over https on their own names. The hospital scenario of the
deployment probe runs on these files once per upstream
(`deploy-probe-hospital` in CI).

## Port already in use?

Every published port is a variable, so a conflict never requires editing any
file. When `docker compose up` refuses with `port is already allocated` (or
`address already in use`), pick a free port and pass it:

```shell
FERROEHR_PORT=8081 docker compose up
```

The same works for the other ports (`FERROEHR_VIEWER_PORT`,
`FERROEHR_PIX_PORT`, `FERROEHR_ISSUER_PORT`, `FERROEHR_S3_PORT`; PostgreSQL publishes no host port; see
[Connecting to the database](#connecting-to-the-database)). The defaults stay
fixed on purpose: every URL in this book assumes `localhost:8080`, and
Docker's automatic ephemeral-port allocation exists only when a mapping
[omits the host port entirely](https://docs.docker.com/engine/network/port-publishing/).
A server that silently moved ports would break every printed URL, so here
you always choose the port and always know it.

> [!NOTE]
> **On Windows** the refusal reads differently: `Ports are not available:
> exposing port TCP 127.0.0.1:8080 … bind: Only one usage of each socket
> address … is normally permitted` when another program holds the port, or
> `… An attempt was made to access a socket in a way forbidden by its access
> permissions` when the port sits in a Windows reserved range. The remedy is
> the same variable; to see the reserved ranges, run
> `netsh interface ipv4 show excludedportrange protocol=tcp`. The quickstart
> publishes no database port, so a natively installed PostgreSQL on 5432 does
> not collide with it.

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
`FERROEHR_BIND_HOST`; the warning above applies).

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
| `FERROEHR_PIX_PORT` | `8090` | Host port mapped to FerroPIX, the patient identity source. |
| `FERROEHR_ISSUER_PORT` | `8180` | Host port mapped to the development OpenID Provider. |
| `FERROPIX_POSTGRES_PASSWORD` | `postgres` | FerroPIX's database superuser password (init only). |
| `FERROEHR_DB_PORT` | not published | Host port for PostgreSQL, read only by the `db-publish` overlay (default `5432` there). |
| `FERROEHR_S3_PORT` | `8333` | Host port mapped to the S3 gateway (the `s3` profile). |
| `FERROEHR_BACKUP_CLINICAL_DIR` | `./backups/clinical` | Host directory the clinical dump job writes to (the `backup` profile). |
| `FERROEHR_BACKUP_PARTY_DIR` | `./backups/party` | Host directory the party dump job writes to. |
| `FERROEHR_BACKUP_USER` | `0:0` (root in the container) | `uid:gid` the dump jobs run as. Set it to `$(id -u):$(id -g)` and the files land owned by you. |
| `FERROEHR_CPUS` / `FERROEHR_MEM` | `4` / `4G` | Server container resource ceiling. |
| `FERROEHR_DB_CPUS` / `FERROEHR_DB_MEM` | `4` / `4G` | Database container resource ceiling. |
| `PG_INIT_USER` / `PG_INIT_PASSWORD` / `PG_INIT_DB` | `ferroehr` | App role, password, and database created by the DB image's init script. |
| `POSTGRES_PASSWORD` | `postgres` | Bootstrap superuser password (init only). |
| `FERROEHR__TELEMETRY__FORMAT` | `pretty` | Log rendering for `docker compose logs`. Set `json` for log collectors. |
| `FERROEHR__TELEMETRY__FILTER` | `info` | Log level filter. |
| `FERROEHR__DATABASE__MAX_CONNECTIONS` | `10` | Server connection-pool ceiling. |
| `FERROEHR__SERVER__MAX_CONCURRENT_REQUESTS` | `256` | Concurrent-request bound (`503` with `Retry-After` past it). |
| `FERROEHR__RECORD__SIGNING__ENABLED` | `true` | Version signing. |

A commercial licence token is not a passthrough variable: mount the signed
token file into the server container read-only and name its path with
`FERROEHR__DEPLOYMENT__LICENCE_FILE` (the compose file carries a commented example beside
`FERROEHR__RECORD__SIGNING__ENABLED`). Unset, the server runs under the embedded
non-commercial grant and reports which on `GET /management/status`.

The Keycloak overlay adds `KEYCLOAK_PORT` (default `8081`),
`KEYCLOAK_HOSTNAME`, `KEYCLOAK_ADMIN_USER` and `KEYCLOAK_ADMIN_PASSWORD` (both
`admin`); the observability overlay adds `GRAFANA_PORT` (default `3000`). The
`FERROEHR__RECORD__MULTIMEDIA__*` set is declared with no defaults on purpose, so an
unset key reaches the server as absent rather than as an empty string.

The server container is passed `FERROEHR__DATABASE__URL` (assembled from the database
variables), and its configuration file is the inline quickstart config,
delivered to `/etc/ferroehr/ferroehr.toml`, which `FERROEHR_CONFIG` names.
Any other setting from the [configuration reference](configuration.md) can be
added under the `ferroehr` service's `environment:` block and takes precedence
over the file. Every variable uses the same `FERROEHR__…` grammar as the rest of
that reference; a single-underscore spelling is rejected at startup, not
ignored.

## Next

- [Configuration reference](configuration.md): every setting you can pass.
- [Kubernetes & Helm](kubernetes.md): the production deployment.
- [Returning to the original state](../operations-reset.md): resetting the stack
  to the configuration and empty database it shipped with.
