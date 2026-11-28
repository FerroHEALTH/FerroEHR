# Upgrades

How to move a running deployment to a new release, and how long a release is supported.

<!-- toc -->

- **Upgrade in place within v6; the migrations are append-only.** A released
  migration is never edited again, so a database created by an earlier v6
  release takes the new release's migrations on top of the ones it already
  carries. The server
  records the checksum of every applied migration and refuses to start against
  a database whose recorded checksums differ from the files it carries
  (`migration 1 was previously applied but has been modified`), which is why
  editing one would lock every existing installation out of its own database
  rather than revise its history. A correction ships as a new migration, and a
  CI guard fails any pull request that modifies, renames or deletes one the
  base branch already has. A rolling upgrade must still stay compatible with
  the _previous_ schema for the window in which both versions run: additive
  changes first, destructive changes a release later.

  A 5.x installation is not upgraded: see [Coming from a 5.x release](#coming-from-a-5x-release).
- **Sweep for stale decomposition after a release that changes how content is
  decomposed.** The version body is unaffected and every read serves it
  correctly; what is stale is the decomposed index over it, which a
  row-level query predicate depends on. `POST /ferroehr/rest/extensions/v1/admin/integrity/verify`
  reports each such version as `stale_decomposition` and
  `POST /ferroehr/rest/extensions/v1/admin/integrity/rebuild-nodes`, run once unscoped, rewrites the
  rows from the stored document. The changelog names the affected object type
  at each release that needs it. Both routes are on
  [Admin & messaging APIs](storage-admin-apis.md#storage-integrity).
- **Bound the DDL yourself.** Every pooled connection carries the
  `[database] statement_timeout` value (60 seconds by default), and the migration
  step runs on that pool, so a runaway statement is cut off, but there is **no
  `lock_timeout`**, so a migration that waits behind a long transaction waits
  as long as that transaction lasts. On a busy table use
  `CREATE INDEX CONCURRENTLY`, add constraints `NOT VALID` and `VALIDATE`
  later, and set `lock_timeout` in the migration session (or on the migrator
  role) if you need the wait bounded.
- **Pin the image.** Deploy an immutable tag or, better, a `@sha256` digest,
  never `latest`; roll back by re-pinning the prior digest, since the schema's
  backward compatibility makes that safe.
- **Stay available.** Keep at least two replicas (or autoscaling) and a pod
  disruption budget so node drains and upgrades never fully interrupt the
  API. The default 30-second termination grace period covers the server's
  shutdown; the audit records are stored before each response, so a
  replica that stops leaves nothing in memory.

## Coming from a 5.x release

v6 is the first stable release, and no 5.x installation is upgraded to it: a
v6 installation starts fresh, on a new, empty PostgreSQL 19 database, and you
reimport your data into it.

## How long the version you pinned is supported

Plan the upgrade cadence around this:

- **Only the newest release is supported.** When a new release is
  published, every earlier release stops receiving fixes: there are no
  maintenance branches, no long-term-support line and no backports. If you are
  not on the newest release, upgrade; upgrading is free. Under the Cyber
  Resilience Act, Cadasto B.V. commits to keep publishing security fixes for at
  least five years after each release (Art 13(8)) and ships them only in the
  newest release (Art 13(10)), which every user of an earlier release may run
  under the same licence terms. Staying on an older release means running
  without the fix.
- **A fix normally arrives as the next patch** on the current minor, so taking
  it does not oblige you to take new behaviour, but that is the usual case,
  not a promise. Where a fix is only correct alongside a behavioural change,
  the release carrying it carries the change, and the changelog entry says so.
- **A published release is never repaired in place.** Release immutability
  means the assets and the tag of a published release cannot be modified at
  all, so the remedy for any defect is a new version.
- **The Helm chart and the published crates follow their own version lines**,
  each supported only in its newest published version, which is where its
  fixes ship. A chart-only fix ships as a new chart version between server
  releases.

**The running server states the end month of the commitment.** Cadasto B.V.
keeps publishing fixed releases for at least five years after each release
(CRA Art 13(8)), and the server reads its release date from its changelog
heading at build time. The end month, the date until which fixed releases
keep coming, does not make the running release supported: a release is
supported only while it is the newest. The end month is on the boot banner (`Support` line), in the boot log, in
`ferroehr --version`, under `support` in `GET /management/info`, and under
`support` in `GET /management/status` as
`{release_date, support_ends, status}`, where `support_ends` is `YYYY-MM` and
`status` is `supported` (the commitment is still running), `ended` or
`unreleased`. Once the end month has passed,
`status` reads `ended` and the server logs a `warn` line at boot and once a day
(CRA Art 13(19)). A build of a version whose changelog heading carries no date
is not a release: it reports `unreleased` and no end month.

The server does not contact anyone to look for newer releases. To hear about
them, subscribe to the release feed
(`https://github.com/FerroHEALTH/FerroEHR/releases.atom`) and poll the
published security advisories
(`GET https://api.github.com/repos/FerroHEALTH/FerroEHR/security-advisories`).

The consequence for change control: budget for taking every release, or budget
for maintaining a fork. There is no third option, and the full policy (with
the reasoning and what to do if you need something stronger) is
[SECURITY.md](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md#supported-versions).

## Upgrading the Helm release

Migrations are **append-only** (a schema change is a new file, never an edit to
an applied one) so a rolling upgrade stays compatible with the previous schema
during the window where both versions run: additive DDL first, destructive
changes in a later release once all pods are on the new version. Keep
`replicaCount` at two or more (or `autoscaling.enabled`) and the default
PodDisruptionBudget so upgrades and node drains never fully interrupt the API;
the default `terminationGracePeriodSeconds` covers the binary's shutdown drain.
Roll back by re-pinning the prior image tag or digest.

A 5.x release is not upgraded to v6: a v6 installation starts fresh and you
reimport your data ([Coming from a 5.x release](#coming-from-a-5x-release)).

A change anywhere under `config`, `config.files` or `secrets` changes the
`checksum/config` pod annotation, so `helm upgrade` rolls the pods for a
configuration-only change too, including a rotated secret or an edited
mounted file, both of which are read at boot and would otherwise reach the volume while
every running pod kept using the old value.

`helm uninstall` removes everything the chart created; the chart declares no
PersistentVolumeClaim, so nothing is left behind; your database, and the Secret
holding its DSN, are yours and survive.

Preview an upgrade against what you have installed with
`helm diff`, or render the new chart version and read it:

```shell
helm template ferroehr oci://ghcr.io/ferrohealth/charts/ferroehr --version 43.1.0 \
  -n ferroehr -f my-values.yaml | less
```

Working from a checkout instead, `deploy/helm/validate.sh` runs the chart's full
render gate: the helm-version pin, lint, YAML validity, the structural
Restricted-profile and selector-immutability gates, the secret-leak gate, the
values-schema probes, and the golden-render diff. It is the same gate CI runs on
every change to the chart, so a local run and a pull request agree by
construction.

### Check your values before you deploy them

A render that lints is not a deployment that boots. `validate.sh` never runs the
server, so it cannot see a configuration the server refuses: a missing
authentication mechanism, an `HS*` algorithm, a password hash that is not a
real Argon2id PHC string, SMART enabled without its public base URL. Every one of those renders perfectly and crash-loops the pod. The
script prints exactly which properties it does not check, on success as well as
on failure, so a green run is not mistaken for a working deployment.

The check that closes that gap runs the image against your rendered
configuration:

```shell
FERROEHR_IMAGE=ghcr.io/ferrohealth/ferroehr:5.7.1-rc1 \
  deploy/helm/ci/boot-check.sh my-values.yaml
```

It renders the chart, mounts the ConfigMap and the Secret exactly as the
Deployment does (with their real values, not placeholders) replays the
declared environment, and runs `ferroehr config check` inside the image. Point
`FERROEHR_IMAGE` at the tag you intend to deploy: the answer is specific to that
image, since a key your values carry may be newer than the server that has to
read it.

It validates configuration only; it opens no socket, so it cannot tell you the
issuer resolves, the broker is reachable or the database accepts the DSN. CI
runs the same script over every values overlay the chart ships.
