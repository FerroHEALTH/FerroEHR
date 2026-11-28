# Docker Compose: overlays and profiles

The overlays that change the quickstart's configuration (Keycloak, FerroTERM, observability),
the optional services behind Compose profiles, and the repository's development override.
The quickstart itself is on [Docker Compose](compose.md).

<!-- toc -->

## The OIDC variant (Keycloak overlay)

A second standalone file adds a ready-made identity provider, so you can
exercise bearer-token authentication without registering a client anywhere.
Download `docker-compose.keycloak.yml` from the same release, beside the base
file, and stack the two:

```shell
docker compose -f docker-compose.yml -f docker-compose.keycloak.yml up
```

That starts **Keycloak** with a small demo realm (`ferroehr`) defined inline and
points the server's bearer validation at it. Basic auth from the base file keeps
working, so the server then advertises `Basic, Bearer`.

The overlay adds the realm's `[auth.issuer.<name>.party]` table, which changes the party
mapping. On a volume the base quickstart already started, the server refuses
to boot on the changed mapping until the binding side table is refilled, so
add or remove the overlay with the server stopped and refill first:

```shell
docker compose -f docker-compose.yml -f docker-compose.keycloak.yml up -d keycloak
docker compose -f docker-compose.yml -f docker-compose.keycloak.yml stop ferroehr
docker compose -f docker-compose.yml -f docker-compose.keycloak.yml run --rm --no-deps ferroehr party-binding refill
```

The server accepts a token issuer only over HTTPS, or plain HTTP on a loopback
host, so Keycloak serves the issuer over TLS at
`https://keycloak.localhost:8443/auth/realms/ferroehr`. The containers resolve
`keycloak.localhost` through a network alias on the compose network, and a
browser resolves every `*.localhost` name to the loopback address, where the
published port lands, so both sides use one issuer URL with no hosts entry. On
the first `up`, a one-shot `keycloak-tls` service generates a development CA
and a certificate for that name into the `keycloak-tls` volume and discards
the CA's private key. The CA publishes no revocation service, so the server
(`[auth.tls] trust_roots_without_revocation_file`) and the viewer
(`[oidc.tls] trust_roots_without_revocation_file`) trust it beside the public
ones with no revocation check, and both name it at the start.
`docker compose down --volumes` deletes the volume, and the next `up`
generates a new CA. Keycloak also answers plain HTTP on port 8081 for the token
call below, and the tokens it mints there name the HTTPS issuer. Keycloak's
access tokens carry `typ: JWT`, so the overlay adds the realm as the
`[auth.issuer.keycloak]` table accepting that type
(`FERROEHR__AUTH__ISSUER__KEYCLOAK__PROFILE__TOKEN_TYPES=JWT`), and the realm
puts `client_id` in every token with a hardcoded claim mapper, beside the
quickstart's `basic_users` issuer, which its Basic user binds under, and its
`quickstart` issuer, which signs its SCIM client's tokens.

Start the viewer too and sign in at <http://localhost:3000> as any user below:

```shell
docker compose -f docker-compose.yml -f docker-compose.keycloak.yml --profile viewer up
```

The viewer takes the authorization-code flow with PKCE through the
`ferroehr-viewer` client and calls the server with the user's token. Your
browser does not trust the development CA, so it warns once on the Keycloak
page: accept the warning, or trust the CA, which
`docker compose cp keycloak:/opt/keycloak/conf/tls/ca/ca.crt .` copies out. A
browser that does not resolve `*.localhost` needs `127.0.0.1 keycloak.localhost`
in its hosts file.

Every route checks SMART on openEHR scopes, so the realm puts them in every
token: `user/composition-*.cruds`, `user/template-*.cruds` and
`user/aql-*.cruds` grant a user every resource family, the service client gets
their `system/` twins, and `__ferroehr.platform` is the product scope the ADMIN
and DEMOGRAPHIC APIs need. The roles below then decide.

The overlay also **turns RBAC on**, deliberately: an identity provider whose
roles decide nothing is a login screen, not authorization. So the realm ships
four users whose roles are load-bearing:

| User | Password | Realm roles | What it demonstrates |
|---|---|---|---|
| `ferroehr` | `ferroehr` | `ADMIN`, `USER` | the full surface, admin API included |
| `clinician` | `clinician` | `USER` | the clinical API without the admin routes |
| `auditor` | `auditor` | `USER`, `READONLY` | reads succeed, every write is refused `403` |
| `nobody` | `nobody` | none | an authenticated caller with no grant |

The server refuses a person whose login no demographic party carries, so the
overlay binds each of the four before you sign in. The realm import gives
every user a fixed `id`, which Keycloak puts in `sub`, and the one-shot
`ferroehr-provision-keycloak` creates a SCIM User for each with that id as its
login key, as a second client of the development issuer, which the one-shot
`ferroehr-grant-keycloak` first grants `scim_provision` in the CDR with the
realm's identifier issuer, `urn:example:keycloak-quickstart-login`, as its
`binding_issuer`. The service
account acts for nobody, so it binds to no party.

Two confidential clients are defined, and both put `ferroehr`, the audience the
server checks, in every access token:

| Client | Secret | Grants |
|---|---|---|
| `ferroehr-viewer` | `ferroehr-viewer-quickstart-secret` | the authorization-code flow with PKCE (`S256`), redirecting only to the viewer's origin on port 3000, and the password grant so a token can be fetched by curl |
| `ferroehr-service` | `ferroehr-service-quickstart-secret` | the client-credentials grant only |

Fetch a user's token and call the API with it:

```shell
TOKEN=$(curl -s -d client_id=ferroehr-viewer -d client_secret=ferroehr-viewer-quickstart-secret \
  -d username=ferroehr -d password=ferroehr -d grant_type=password \
  http://localhost:8081/auth/realms/ferroehr/protocol/openid-connect/token \
  | jq -r .access_token)

curl -H "Authorization: Bearer $TOKEN" -X POST -i \
  http://localhost:8080/ferroehr/rest/openehr/v1/ehr
```

A service with no user behind it takes the client-credentials grant with
`ferroehr-service`. Its service account holds `USER`, so the token reaches the
clinical API and the admin routes refuse it with `403`. Its `system/aql-*`
grant counts because the CDR grants the client `query_system_wide`, and it
creates EHRs because the CDR grants it `commit_composition`, the only way a
caller acting for no person commits: the `ferroehr-provision-keycloak`
one-shot writes its `AGENT` and a `ROLE` coding both operations through the
Demographic API, and the overlay admits them in `[authz.grants] allowed`
(`FERROEHR__AUTHZ__GRANTS__ALLOWED`)
([Operational policy](../operational-policy.md#what-a-system-client-may-do)).
The `commit_composition` grant names no template, so it commits no
COMPOSITION:

```shell
TOKEN=$(curl -s -d client_id=ferroehr-service -d client_secret=ferroehr-service-quickstart-secret \
  -d grant_type=client_credentials \
  http://localhost:8081/auth/realms/ferroehr/protocol/openid-connect/token \
  | jq -r .access_token)
```

> [!WARNING]
> The realm, the client secret, the user passwords and the CA are demo values,
> served by a Keycloak in `start-dev` mode. For a real deployment, drop this
> overlay and point an `[auth.issuer.<name>]` table (or
> `FERROEHR__AUTH__ISSUER__<NAME>__ISSUER` and its siblings, `<NAME>` the
> table's name) at your own issuer over HTTPS.

## The terminology overlay (FerroTERM)

[FerroTERM](https://github.com/FerroHEALTH/FerroTERM) is the Ferro family's
FHIR terminology server: R4, R4B and R5 endpoints over a precomputed index, no
JVM, no database, one distroless image. The overlay
`docker-compose.terminology.yml` starts it beside the CDR and switches the CDR's
terminology server on, so archetype value-set bindings resolve at commit and
AQL `TERMINOLOGY()` expands through it. It also switches on the read-only
`/terminology/*` extension API (`FERROEHR__API__TERMINOLOGY`, off by
default): a term, subsumption or value-set call there for a terminology the
bundled openEHR terminology does not carry is answered by FerroTERM, while
listing and describing terminologies stays with the bundle:

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
is accepted. `FERROEHR__TERMINOLOGY__ON_ERROR=true` turns an
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
  export FERROEHR__RECORD__MULTIMEDIA__ENABLED=true
  export FERROEHR__RECORD__MULTIMEDIA__S3__URL=http://seaweedfs:8333   # dev only; production S3 is HTTPS
  export FERROEHR__RECORD__MULTIMEDIA__S3__BUCKET=openehr-multimedia

  docker compose --profile s3 up -d --wait ferroehr seaweedfs
  ```

  The compose file passes the whole `FERROEHR__RECORD__MULTIMEDIA__*` set through from
  your shell, so there is no file to edit, and `seaweedfs-init` creates the
  bucket by issuing an idempotent `CreateBucket` against the gateway, which
  ships with none, and answers an S3 write into a missing bucket with
  `403 AccessDenied`, a reply that reads as a credentials problem and is not
  one. It reads the same bucket variable the server does, so the two cannot
  drift.

  To turn it off again, `unset` those variables (or set
  `FERROEHR__RECORD__MULTIMEDIA__ENABLED=false`) and re-up: an unset variable is
  *removed* from the container's environment rather than passed as empty, so
  the server falls back to its own default of `enabled = false`.

  Confirm the server took them with
  `curl -s -u ferroehr:ferroehr http://localhost:8080/management/env | jq .multimedia`
  (`"enabled": true` and a non-empty `endpoint` mean the wiring is right). In
  production, point the multimedia settings at a real, credentialed, HTTPS S3
  endpoint instead; see [S3 multimedia](../beyond-core/s3-multimedia.md).

- **`ferroehr-backup-clinical` and `ferroehr-backup-party`**
  (`--profile backup`): one `pg_dump` job per
  pseudonymisation domain, run on demand rather than started with the stack:

  ```shell
  docker compose --profile backup run --rm ferroehr-backup-clinical
  docker compose --profile backup run --rm ferroehr-backup-party
  ```

  Each writes a timestamped custom-format dump into its own host directory
  (`FERROEHR_BACKUP_CLINICAL_DIR` and `FERROEHR_BACKUP_PARTY_DIR`, defaulting
  to `./backups/clinical` and `./backups/party`). Set
  `FERROEHR_BACKUP_USER="$(id -u):$(id -g)"` and the files land owned by you;
  unset, the job runs as root inside the container with one capability,
  `DAC_OVERRIDE`, which is what writing a directory it does not own actually
  requires.

  Three dumps rather than one is the point: a single whole-database dump holds
  the clinical record, the identities of its subjects and the map between them,
  and whoever reads that file re-identifies every record in it. This stack runs
  one login credential, so it separates the artefacts and not the authority to
  produce them; [Operations → Dump each domain
  separately](../database-operations.md#dump-each-domain-separately) is the production
  procedure, including why a backup credential needs `BYPASSRLS`.

Two additions change the server's configuration, which a profile cannot do, so
each is a separate overlay file:

- **OIDC / Keycloak** is the `docker-compose.keycloak.yml` overlay above (and,
  in a repository checkout, a `keycloak` profile of the development override
  described below).
- **A terminology server** is the `docker-compose.terminology.yml` overlay
  [above](#the-terminology-overlay-ferroterm), FerroTERM beside the CDR; the
  repository's conformance stack additionally runs its own seeded server for
  the acceptance lane, see [Terminology servers](../beyond-core/terminology.md).

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
applies when its `-f` is passed; nothing merges it in silently.

> [!TIP]
> A cold from-source build compiles at two parallel rustc jobs by default,
> and a release-optimization compile can hold several gigabytes per job. If
> the build gets OOM-killed on a memory-constrained host (observed on a
> 24 GB machine that was also running another stack), rerun it serially,
> which is slower but completes:
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
metrics** over OTLP/gRPC (`FERROEHR__TELEMETRY__OTLP__ENDPOINT` with
`FERROEHR__TELEMETRY__OTLP__METRICS=true`), switches stdout to JSON lines
(`FERROEHR__TELEMETRY__FORMAT=json`), and enables the management surface on its own
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
