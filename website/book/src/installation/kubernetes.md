# Kubernetes & Helm

The `ferroehr` Helm chart deploys FerroEHR as a hardened, production-shaped
Kubernetes workload (non-root, read-only root filesystem, a NetworkPolicy that
admits its serving port only, its own user namespace) against an **external**
PostgreSQL 19. This chapter
covers installing it, verifying what you installed, the database role model it
expects, its secrets, and staying available while things move. Upgrading the
release is on [Upgrades](../upgrades.md#upgrading-the-helm-release). The security posture, the health probes
and authentication are on [Kubernetes: security posture, probes and
authentication](kubernetes-security.md), and the optional integrations on
[Kubernetes: integrations](kubernetes-integrations.md).

<!-- toc -->

> [!IMPORTANT]
> **The chart requires Kubernetes 1.36 or newer** (`kubeVersion: ">=1.36.0-0"`).
> That is a compatibility floor, not a support opinion: 1.36 is the release where
> the newest field the chart renders (`hostUsers`, which gives every pod its own
> user namespace) became stable, so nothing has to be version-gated into silence.
> Your nodes must be Linux with containerd 2.0 or newer, or CRI-O 1.25 or newer;
> without that support the pod does not start, which is the loud failure rather
> than a silent downgrade. Set `hostUsers: true` to opt out and share the host's
> user namespace. What the user namespace buys is in
> [Cluster hardening](./kubernetes-hardening.md).

> [!IMPORTANT]
> There is **no in-chart PostgreSQL**. A CDR stores PHI, so its database must be
> an externally managed, backed-up, point-in-time-recoverable PostgreSQL 19 (a
> managed service or an operator-run cluster). The chart carries only the
> connection string, preferably from an existing Secret.

## Installing

The chart is published to GHCR as an **OCI artifact**, beside the images it
deploys. Create a Secret holding the app-role connection string, then install:

```shell
kubectl create namespace ferroehr
kubectl -n ferroehr create secret generic ferroehr-db \
  --from-literal=FERROEHR__DATABASE__URL='postgres://ferroehr_clinical:***@pg-host:5432/ferroehr?sslmode=verify-full'

helm install ferroehr oci://ghcr.io/ferrohealth/charts/ferroehr \
  --version 43.0.2 -n ferroehr \
  --set config.deployment.profile=sandbox \
  --set database.existingSecret=ferroehr-db \
  --set image.tag=5.7.1-rc1 \
  --set ferropix.enabled=false --set config.record.subject_index=local
```

`config.deployment.profile=sandbox` names the posture of a trial install,
which must not hold real patient data; the chart has no default and refuses to
render without one. A deployment holding patient data starts from the
[production overlay](#the-production-overlay) instead. The last line opts in to
the in-instance EHR index, for a CDR run on its own.
By default the chart runs FerroPIX, the IHE patient identity source, beside the
CDR and points the EHR index at it, which needs FerroPIX's own Secrets and an
issuer: see [FerroPIX](kubernetes-integrations.md#ferropix-the-patient-identity-source-on-by-default).
With neither, the render is refused and names both choices, as the server's
boot would be.

> [!IMPORTANT]
> **`helm repo add` does not work for this chart, and never will.** There is no
> HTTP chart repository and no `index.yaml`: OCI is the only publication path, so
> that there is exactly one place a chart version can exist. Every command takes
> the `oci://` reference above. Helm has treated OCI registries as first-class
> since 3.8, so the cost of this choice is real but narrow: a client older than
> Helm 3.8 cannot install this chart at all.

`helm show`, `helm pull`, `helm template` and `helm upgrade` all take the same
reference. To read the chart's metadata without installing it:

```shell
helm show chart oci://ghcr.io/ferrohealth/charts/ferroehr --version 43.0.2
```

### Pin two versions, not one

**The chart version and the image tag move independently, and this is the thing
people get wrong first.** `--version` selects the *chart* (its templates and
values schema); `image.tag` selects the *server binary*. The chart's `appVersion`
is only the default for the second, and it names the release the chart was cut
against.

| | Selects | Pin with | Line |
|---|---|---|---|
| Chart version | templates, values schema, defaults | `--version 43.0.2` | SemVer over the chart's own contract |
| Image tag | the server binary | `--set image.tag=5.7.1-rc1` (or `image.digest`) | the application's SemVer line |

Always pin the image to an immutable version or, better, a `@sha256` digest,
never `latest`. Pin the two deliberately: the `config` tree is passed through to
the server, so a key the chart's defaults carry and your chosen image does not know
is a boot refusal (`unknown configuration key …`), which presents as
`CrashLoopBackOff`. A chart version is never republished with different content
(the publish lane refuses to overwrite one) so a pinned chart version is a fixed
artifact.

> [!NOTE]
> A published chart version being immutable is a property of that refusal, not of
> the registry: an OCI tag is mutable, and `helm push` over an existing one would
> silently replace it. That is why a correction always ships as a new chart
> version, and why the `6.0.3` → `6.0.4` bump exists: the chart's `appVersion`
> moved to a new release while its own version had already been published.

### Verifying what you installed

Everything published here is signed **keyless** through
[Sigstore](https://docs.sigstore.dev/), bound to this repository's build
identity, so every claim is checkable rather than asserted. Two different
artifacts answer two different questions, and you can ask both.

**Who signed this chart:** a [cosign](https://docs.sigstore.dev/cosign/)
signature over the chart's digest:

```shell
cosign verify ghcr.io/ferrohealth/charts/ferroehr:<chart-version> \
  --certificate-identity-regexp '^https://github\.com/FerroHEALTH/FerroEHR/\.github/workflows/build-chart\.yml@' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

Both flags matter. Without them `cosign verify` would accept a signature from
*any* identity in the transparency log; with them you are requiring that this
repository's chart-publishing workflow, authenticated by GitHub's OIDC issuer,
is what signed the bytes you pulled.

**What it was built from:** a SLSA build provenance attestation:

```shell
# the chart
gh attestation verify oci://ghcr.io/ferrohealth/charts/ferroehr:<chart-version> -R FerroHEALTH/FerroEHR
# the image it deploys
gh attestation verify oci://ghcr.io/ferrohealth/ferroehr:<tag> -R FerroHEALTH/FerroEHR
```

The publish lane reads both back from the registry before it reports success, so
a run that produced only one of them fails instead of going green.

> [!IMPORTANT]
> Both commands verify what the publishing lanes produce now. A published
> artifact is never replaced, so if one you pinned answers `HTTP 404: Not Found`
> it carries no attestation, which is the honest state rather than a verification
> failure, and the fix is to pin a current version. Image signing landed during
> the `3.17.4` cycle, so image tags from before it have nothing to
> verify.

> [!NOTE]
> `helm install --verify` and `helm verify` do **not** apply: they check a PGP
> `.prov` file, and this chart ships none. That is deliberate: a `.prov` needs a
> long-lived private key in CI, which is the exact thing this project's publishing
> lanes are built to avoid (the crates.io lane uses OIDC Trusted Publishing and
> holds no token at all). The two keyless commands above are what replace it:
> nothing to leak, and the same trust root as the images.

### Your values file is checked before anything is applied

The chart ships a `values.schema.json`, so `helm install`, `helm upgrade`, `helm
lint` and `helm template` **refuse** a values file that misspells one of the
chart's own keys, gets a type wrong, or names a value outside the permitted set,
rather than rendering and silently ignoring it:

```text
Error: values don't meet the specifications of the schema(s) in the following chart(s):
ferroehr:
- at '/image/pullPolicy': value must be one of 'Always', 'IfNotPresent', 'Never'
```

**The `config:` tree is deliberately exempt.** Those keys are the *server's*
(see the [configuration reference](configuration.md)), the binary validates them
at boot, and copying that vocabulary into the chart's schema would fork it: a
new configuration key would then be rejected by the chart until someone
remembered to widen the schema. So a mistake under `config:` is reported when the
pod starts, not when the chart renders; `--skip-schema-validation` disables the
check entirely if you ever need to bypass it.

The chart is also listed on **[Artifact
Hub](https://artifacthub.io/packages/helm/ferroehr/ferroehr)**, which renders the
chart's metadata plus a security report over the four images the chart's own
metadata lists: the server, the optional viewer, the backup jobs' `pg_dump`
image, and FerroTERM.

> [!WARNING]
> Between releases the chart's `config` defaults track development and can be
> **ahead of `appVersion`'s image**. Check the pairing before you install, with
> the image itself as the authority:
>
> ```shell
> helm template ferroehr oci://ghcr.io/ferrohealth/charts/ferroehr --version 43.0.2 \
>   -s templates/configmap.yaml --set database.existingSecret=ferroehr-db \
>   | sed -n '/ferroehr.toml/,$p' | sed '1d;s/^    //' > /tmp/ferroehr.toml
> docker run --rm -v /tmp/ferroehr.toml:/etc/ferroehr/ferroehr.toml:ro \
>   -e FERROEHR_CONFIG=/etc/ferroehr/ferroehr.toml \
>   -e FERROEHR__DATABASE__URL=postgres://u:p@db:5432/ferroehr \
>   --entrypoint /usr/local/bin/ferroehr ghcr.io/ferrohealth/ferroehr:<tag> config check
> ```
>
> Exit 0 means the image accepts the rendered configuration. A reported unknown
> key means the image is older than the chart's defaults: use a newer tag, or set
> that key to `null` for this deployment. A *published* chart is checked this way
> before it is published: the publish lane refuses to ship a chart whose defaults
> its own `appVersion` image rejects, and it repeats the check against every values
> overlay the chart carries, so this matters mainly when you install from a
> checkout of `main`, or pin an image older than the chart.

That command alone **is refused at render**, deliberately:
`config.auth.enabled` is on and no mechanism is configured yet, and a server that
authenticates nothing is not a safe default. The refusal names the two ways
forward: a `config.auth.issuer` table by name (with its `issuer` and its
`audience`), or a `config.auth.basic.users` entry, as
`deploy/helm/ci/basic-auth-values.yaml` in the repository carries.

A credential is not yet a caller. A Basic user is a person: it maps to the key
of a `PERSON` under `config.authz.party_binding.basic_user`, and until that
`PERSON` exists every request it makes answers `403` with
`refused-unbound`. A token's user is bound the same way through the `party`
table of its issuer. Create the first persons and grants as
[The first grant](../operational-policy.md#the-first-grant) describes, with
`ferroehr party-binding grant` run in a server pod
(`kubectl -n ferroehr exec deploy/ferroehr -c ferroehr -- /usr/local/bin/ferroehr party-binding grant …`).
Every route also checks the SMART on openEHR scopes of the caller: the scopes
a Basic user's `PERSON` holds, a token's `scope` claim.

## Configuration: one key, one file

The chart carries the server's whole configuration under one key, **`config`**,
rendered into a `ferroehr.toml` ConfigMap mounted at
`/etc/ferroehr/ferroehr.toml`. Its keys are therefore exactly the TOML keys of
the [configuration reference](configuration.md) (`config.server.listen`,
`config.authz.rbac.enabled`, `config.deployment.spec_profile`, and so on) so anything
that reference documents can be set without waiting for a bespoke chart key.

**Secret-bearing keys are the one exception: a credential never reaches that
ConfigMap.** A ConfigMap is not a sensitive object: it is readable with namespace
read, quoted wholesale into issues and support tickets, collected by backup tooling
that skips Secrets, and not covered by Secret encryption at rest. The chart
therefore classifies every key it renders and takes one of two actions.

**A secret the chart routes:** `record.signing.key_passphrase`,
`record.multimedia.secret_access_key`, a Basic user's `password_hash`, a terminology
`client_secret`, and the two URL-shaped ones (`database.url` and
`audit.fhir_feed.endpoint.url`) has a `secrets:` key of its own, so
a value under `config:` is a mistake and fails the render, naming the key that
belongs there:

```text
Error: execution error at (ferroehr/templates/deployment.yaml:…):
  refusing to render a secret into the ConfigMap (a ConfigMap is not a sensitive object …):
  - config.record.signing.key_passphrase: set `secrets.signingKeyPassphrase` instead
```

**A secret the chart cannot route** moves the **whole rendered `ferroehr.toml` into
the chart's Secret, and no ConfigMap is created at all**, the safe direction,
taken automatically. **No key reaches that branch today**: every credential the
server models now has either a `*_file` sibling or a Secret-borne environment
route. The branch exists for the next one that does not, so that a secret key
added upstream fails safe instead of landing in a ConfigMap. When it is taken, the
install notes say which object your release used, and the configuration is read
with:

```shell
kubectl -n ferroehr get secret ferroehr-config \
  -o jsonpath='{.data.ferroehr\.toml}' | base64 -d
```

Classification is by **name shape**, not by a list of today's keys: any key whose
name carries `password`, `passphrase`, `secret`, `credential`, `private_key`,
`api_key` or a trailing `token` is treated as a credential unless it ends in
`_file`, `_path` or `_dir` (those hold a path), and the four URL-shaped secrets are
matched by path, because `url` carries no shape a classifier can see. That is what
makes a secret key added to the server's configuration tree in a future release
move to the Secret rather than leak silently. `extraEnv` is the escape hatch for
anything neither `config:` nor `secrets:` surfaces.

### The deployment profile and the access log

The chart's values declare three keys, so a rendered `ferroehr.toml` always
says what it runs:

- **`config.deployment.profile`:** no default; the chart refuses to render
  until it names `sandbox` or `production`, so an installation that follows a
  quickstart instead of the instructions for use does not end up in the sandbox
  posture by omission (EHDS Annex II 1.2). `sandbox` must not hold real
  patient data: the server names every separation it has not made on its boot
  banner and on `/management/status`, and reports the profile on
  `GET /ferroehr/rest/status`. A deployment holding patient data sets
  `production`, which refuses to start while a separation is missing and not
  named in `config.deployment.accepts`. The chart defaults leave several open:
  one DSN for every domain, an empty `config.privacy.subject_namespaces`, the
  server's `open` default for `config.authz.rbac.ehr_access_default`, `config.database.migrations: apply` on the
  runtime credential, and a plaintext listener behind the ingress; see [`deployment.profile`](configuration.md#deploymentprofile).
  The chart's [production overlay](#the-production-overlay) closes them and
  accepts the plaintext listener by name.
- **The access log retention** is no chart value: it is the controller's
  access policy (`ACCESS_POLICY.log_retention`), written through the admin
  API once the server runs ([Audit trail](../ehds-logging.md#retention-and-who-chooses-it)).
- **The category map** is no chart value: it is the
  [audit category register](config-audit.md#the-audit-category-register), an
  [admin register](../access-policy.md#the-admin-registers) you write
  through the admin API once the server runs. Until then the
  category map FerroEHR ships applies. The access delay is an admin register
  too.

The [Audit](config-audit.md) page has the category vocabulary, and the
[audit trail](../ehds-logging.md#retention-and-who-chooses-it) page the access log
retention the controller declares in its access policy.

### The production overlay

The chart ships `values-production.yaml` beside `values.yaml`. Layered over
the defaults, it runs `config.deployment.profile: production` with every
separation that profile checks made, except one it accepts by name:

| Gap | In the overlay |
|---|---|
| `shared_credential` | the clinical and party domains each read their own Secret (`database.existingSecret`, `database.party.existingSecret`), and the audit domain reads `database.audit.existingSecret` |
| `shared_cluster` | yours to make: the server compares the cluster each domain pool reaches, so the separation is three DSNs on three PostgreSQL clusters. Each cluster is prepared on its own migrator: the migration Job applies the schema in every cluster, and the server verifies it there, on `database.<domain>.migrateExistingSecret` (`database.migrateExistingSecret` for the clinical cluster) |
| `migrate_on_runtime_credential` | `config.database.migrations: verify`, with `migrations.job.enabled` preparing the schema on `database.migrateExistingSecret` |
| `open_subject_namespace` | `config.privacy.subject_namespaces` declared |
| `audit_off` | the audit trail on (`config.audit.enabled: true`), which refuses an access it cannot record |
| `no_category_map` | the shipped category map (the default), or templates and archetypes of your own in the audit category register, which refuses a write that opens the gap |
| `open_ehr_access_default` | `config.authz.rbac.ehr_access_default: restricted` |
| `auth_off` | OIDC authentication on |
| `rbac_off` | the role gate on (`config.authz.rbac.enabled: true`, the chart's default) |
| `plaintext_audit_feed`, `plaintext_object_store` | the audit FHIR feed and the multimedia store off; an `https://` URL for either when you turn it on |
| `plaintext_listener` | accepted by name: TLS ends at the Ingress (`ingress.tls`), the pod listens in plain HTTP, and the NetworkPolicy admits only the ingress controller's namespace (`networkPolicy.ingressFrom`, with `networkPolicy.ingressAllowAll: false`) |

The database hop is TLS with the server's certificate checked: end every DSN in
`?sslmode=verify-full&sslrootcert=/etc/ferroehr-db-ca/ca.crt`, and put the CA
your database certificates chain to in a ConfigMap under the key `ca.crt`. The
overlay names it `ferroehr-db-ca` (`database.caConfigMap`), and the chart mounts
it read-only at `/etc/ferroehr-db-ca` in the server, the migration Job and the
backup CronJobs. A CA the image already trusts needs no ConfigMap; leave
`database.caConfigMap` empty then.

Every Secret name, the CA ConfigMap name, the identity provider, the pseudonym
namespace, the host, the TLS Secret and the ingress controller's namespace in it
are examples.
Replace them in a values file of your own and pass both:

```shell
helm pull oci://ghcr.io/ferrohealth/charts/ferroehr --version 43.0.2 --untar
helm install ferroehr ./ferroehr -n ferroehr \
  -f ./ferroehr/values-production.yaml -f my-site-values.yaml
```

A name you forget to replace fails loudly: a Secret that does not exist keeps
the pod from starting, and the server refuses to start under `production`
while a separation is open, naming each gap and its remedy. The chart's
validation renders the overlay and asserts each separation above, and its
boot check has the server evaluate the posture from the rendered
configuration. Neither reaches a database. The deployment probe
(`scripts/deploy-probe-k8s.sh --production-only`) installs the overlay on a
real cluster against three PostgreSQL clusters that refuse a connection without
encryption: it observes the migration Job prepare all three and the server start
under `production` with `plaintext_listener` as its only gap, both over
`sslmode=verify-full` against the CA the overlay mounts, and the server refuse
all three domains on one cluster by naming `shared_cluster`. Against your own databases, `shared_cluster` is first
judged when the pod starts.

## Database roles: who runs migrations

The chart expects a PostgreSQL role model in which the runtime pod is never a
superuser. Four roles cover the single-domain posture:

| Role | Purpose |
|---|---|
| owner | owns the database (provisioning only) |
| `ferroehr_migrator` | runs the append-only schema migrations |
| `ferroehr_clinical` | day-to-day reads and writes; **the running pod connects as this** |
| `ferroehr_clinical_reader` | read-only, for replicas and reporting |

Migrations are DDL, so whoever applies them can rewrite the schema. Two flows,
and `config.database.migrations` is where you choose:

- **(a) The pod migrates itself** (`config.database.migrations: apply`, the default).
  Simplest for single-tenant or small deployments, and the runtime DSN must
  then be a member of `ferroehr_migrator`, so the serving process holds DDL
  rights on the clinical schema for its whole life.
- **(b) A separate migration step** (`config.database.migrations: verify`) under a
  migrator DSN, with the pods on a narrow runtime DSN. The server issues no DDL
  at all and refuses to boot against a database that has not been migrated to
  its build, so the two versions cannot race the schema. Recommended for
  production. It needs `database.migrateExistingSecret` as well, for the reason
  the paragraph further down gives: `verify` reads all four bookkeeping tables,
  which a narrow runtime role cannot do.

Set `migrations.job.enabled` and the chart runs (b) for you as a
`pre-install,pre-upgrade` hook `Job` that runs `ferroehr db migrate`. Helm creates
it before the Deployment and waits for it, so a failed migration fails the release
rather than rolling pods against a schema that was never applied. The Job
authenticates from its **own** Secret (deliberately a different credential from
`database.existingSecret`) and rendering is refused if you enable it without one:

```yaml
database:
  existingSecret: ferroehr-db                 # postgres://ferroehr_clinical:…
  migrateExistingSecret: ferroehr-db-migrator # postgres://ferroehr_migrator:…
migrations:
  job:
    enabled: true
    existingSecret: ferroehr-db-migrator      # the same credential, its own Secret mount
config:
  database:
    migrations: verify
```

The demographic domain takes its own credential the same way,
and that is what turns the schema separation into a credential separation. One
more goes with them, because no runtime credential can prepare the schema:

```yaml
database:
  existingSecret: ferroehr-db             # postgres://ferroehr_clinical:…
  party:
    existingSecret: ferroehr-db-party     # postgres://ferroehr_party:…
  migrateExistingSecret: ferroehr-db-migrator        # postgres://ferroehr_migrator:…
```

All three are mounted as files, so no DSN enters the pod's environment. Unset
the demographic one and that pool shares the first, which is
the schema-only posture, still separated and still verified at boot. Creating
the four domain
roles is a database step the chart cannot do for you;
[Operations](../database-operations.md#turning-the-schema-split-into-a-role-split) has
the statements and what `ferroehr db verify` reports.

`database.migrateExistingSecret` is what the pod prepares the schema with, and
it is required as soon as `database.existingSecret` is anything narrower than a
credential reaching every schema. Preparation spans them all at once: the DDL
of all four migration sets under `config.database.migrations: apply`, all four
`_sqlx_migrations` bookkeeping tables under `verify`. So **`verify` is not the
exception**, and this is not only about the domain roles. `ferroehr_clinical` holds
one pseudonymisation domain, `ferroehr_clinical` holds the clinical schemas, and
neither can read the `party` bookkeeping at all; without
this value the pod is refused on the first set it cannot read and does not
start. It is normally the same credential
`migrations.job.existingSecret` carries; the Job needs its own copy because it
runs before the Deployment exists. Unset, preparation falls back to
`database.existingSecret`, which is what a single-credential install has
always done.

`database.migrateExistingSecret` reaches one database. A domain whose DSN
reaches another database or cluster is prepared there, on that database's own
migrator, which `database.<domain>.migrateExistingSecret` names:

```yaml
database:
  party:
    existingSecret: ferroehr-db-party            # postgres://ferroehr_party:…@party-db/…
    migrateExistingSecret: ferroehr-db-party-migrate   # postgres://ferroehr_migrator:…@party-db/…
```

The migration Job mounts it and applies the schema in that database; the server
mounts it and verifies the schema there under `config.database.migrations: verify`. The
domain's own role cannot do either, because it holds one domain and cannot read
the `ext` set's bookkeeping. The server refuses to start when the two DSNs of
one domain reach different databases.

Give the migrator DSN a short `lock_timeout`
(`?options=-c%20lock_timeout%3D5s`) so DDL blocked behind live traffic fails
fast instead of queueing; `migrations.job.activeDeadlineSeconds` is the hard
ceiling on the step either way. `migrations.runByMigratorRole` remains an
informational marker surfaced in the install notes, which also tell you when
the Job is enabled but `config.database.migrations` is still `apply`, a combination
that buys nothing, because the server would migrate itself anyway.

You can check the posture from outside the cluster at any time: `ferroehr db
verify` exits 0 only when the database carries exactly that build's migrations,
and issues no DDL doing it.

## Secrets and mounted config

Some material is file-shaped rather than a value: ATNA TLS certificates, terminology-server client certificates, a JWKS blob, the PGP
signing key, and a commercial licence token. Supply these under `config.files`,
whose entries the chart mounts read-only from a Secret at `/etc/ferroehr/<key>`
(and which is deliberately *not* part of the rendered TOML); point the matching
in-TOML `*_file` / `*_path` key at the mounted path. A commercial licence goes
the same way: put the token the licensor issued under `config.files` as
`licence.asc` and set `config.deployment.licence_file` to `/etc/ferroehr/licence.asc`;
`GET /management/status` then reports `licence.use = "commercial"`. Without
it the pod runs under the `non-commercial` grant every build embeds (see
[Licensing & legal](../licensing.md#installing-a-commercial-licence)). Secret-bearing scalar values go under
`secrets:`: `signingKeyPassphrase`,
`auditFhirFeedUrl`, `basicUserPasswordHashes`,
`multimediaAccessKeyId`, `multimediaSecretAccessKey`,
`terminologyClientSecret`, and the database DSN comes from
`database.existingSecret` (key `database.existingSecretKey`, default
`FERROEHR__DATABASE__URL`). None of these ever reach the ConfigMap, and all but two are
delivered as mounted files rather than environment values.

How a secret reaches the process differs by whether the configuration key has a
`*_file` sibling, and the difference is a security one: an environment variable
is readable through `/proc/<pid>/environ` and is inherited by every child
process, so the [OWASP Kubernetes Security Cheat
Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Kubernetes_Security_Cheat_Sheet.html)
asks for a read-only volume instead.

| Secret | How the chart delivers it |
|---|---|
| the database DSN | mounted at `/etc/ferroehr-secrets/database.url`, reached through `FERROEHR__DATABASE__URL_FILE`, projected from `database.existingSecret` when you supply one, so **the credential that reaches patient data never enters the pod's environment** |
| `secrets.signingKeyPassphrase` | mounted at `/etc/ferroehr-secrets/record.signing.key_passphrase` |
| `secrets.multimediaSecretAccessKey` | mounted at `/etc/ferroehr-secrets/record.multimedia.secret_access_key` |
| `secrets.basicUserPasswordHashes` (per user-id) | mounted at `/etc/ferroehr-secrets/auth.basic.users.<user>.password_hash`; the chart injects the matching `password_hash_file` |
| `secrets.terminologyClientSecret` | mounted at `/etc/ferroehr-secrets/[terminology].oauth.client_secret`; the chart injects the matching `client_secret_file` into `config.[terminology].oauth`, which must declare the grant |
| `secrets.auditFhirFeedUrl` | env; `audit.fhir_feed.endpoint.url` is the only credential-bearing key with no `*_file` sibling |
| `secrets.multimediaAccessKeyId` | env; an access key *id* is not secret (it is reported unredacted by the management surface's `env` endpoint) |

The mount is read-only, `0440`, owned `root:65532` so the non-root process reads
it through the group bit, and it is deliberately **not** a `subPath` mount,
because a `subPath`-mounted Secret never receives updates and a rotation would
not propagate.

> [!NOTE]
> A Basic user's Argon2id hash is delivered as a mounted file like the others: put
> it in `secrets.basicUserPasswordHashes` under the user-id, declare only the
> `user` under `config.auth.basic.users`.
> Setting `password_hash`
> under `config:` is refused, and the error names the key that carries it. An
> Argon2id hash is not a plaintext password, but it is an offline cracking target,
> which is what the boot-time OWASP parameter floor exists to make expensive.

## Staying available while things move

Five defaults keep the API serving through the events that routinely interrupt
it. None needs configuring; each is listed because the reason matters when you
tune it.

**Replicas land on different nodes.** With nothing telling the scheduler
otherwise, two replicas can share one node and one node failure is a total
outage. The chart ships a soft spread constraint (`maxSkew: 1` over
`kubernetes.io/hostname`, `whenUnsatisfiable: ScheduleAnyway`) so replicas
prefer separate nodes but a single-node or full cluster still schedules them
rather than leaving a pod `Pending`. Setting `topologySpreadConstraints`
**replaces** it wholesale, so give the complete constraint including its own
`labelSelector`; add a `topology.kubernetes.io/zone` entry if your cluster spans
zones.

**A terminating pod stops receiving requests before it shuts down.** Deleting a
pod removes it from the EndpointSlice and sends `SIGTERM` *concurrently*, and
the removal still has to propagate to every node. `preStopSleepSeconds` (default
5) holds the container for that window first. It uses the native `sleep` hook
action rather than an `exec` hook, because the image ships no shell to run one.

**A node drain does not hang on unhealthy pods.** The PodDisruptionBudget sets
`podDisruptionBudget.unhealthyPodEvictionPolicy: AlwaysAllow`, the
[documented recommendation](https://kubernetes.io/docs/concepts/workloads/pods/disruptions/).
The API default, `IfHealthyBudget`, makes a drain wait for pods to become
healthy, which never completes when they are unhealthy *because* of the drain.

**A migration interrupted by a drain is not counted as a failure.** The
migration Job carries a `podFailurePolicy` that ignores pod failures caused by
disruption, so ordinary cluster maintenance during a release cannot exhaust
`migrations.job.backoffLimit` and fail the upgrade with no migration error
anywhere in the logs.

**A reused connection is not closed under a client.** The server closes a
kept-alive connection only after `config.server.idle_timeout` (two minutes by
default) with no request, and counts `config.server.header_read_timeout`
from the first byte of each request. Keep the idle limit longer than the idle
timeout of your clients (90 s for a Go client) and of the ingress controller or
load balancer in front, so they close an idle connection first. A client that
sends a request into a connection the server is closing gets no answer, and a
client that never retries a `POST` reports the commit as failed.

Two more are available and off by default. `service.trafficDistribution:
PreferSameZone` keeps traffic inside the caller's zone, for lower latency and
inter-zone cost, at the price of even load, so measure before setting it. And
`autoscaling.behavior` passes scaling policies straight through; the documented
defaults already scale up immediately and wait out a five-minute stabilization
window before scaling down, so change it only to be *more* conservative.
