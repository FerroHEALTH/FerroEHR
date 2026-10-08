# Operations

Running a clinical data repository in production means more than starting the
binary: the database must be backed up and least-privileged, traffic must be
encrypted, upgrades must be safe while the service stays up, and you need to
see what the system is doing. This chapter is a production checklist (database
roles, TLS, backup and point-in-time recovery, upgrades and migrations,
observability, the health probes, and the management surface) drawn from how
the container image and Helm chart are built to run. The
operator-facing HTTP surfaces have their own page:
[Admin & messaging APIs](operations-admin-apis.md).

<!-- toc -->

## Database roles and least privilege

FerroEHR connects to an **external** PostgreSQL 18: a managed service or an
operator-run cluster, never a chart-side sidecar, because a database holding
PHI must be independently backed up and recoverable. The server carries only a
connection string, ideally sourced from a secret.

The database never runs as a superuser at runtime. Two roles cover
provisioning and migration:

| Role | Purpose | Used by |
|---|---|---|
| owner | owns the database | provisioning only |
| `ferroehr_migrator` | runs the schema migrations; owns the helper functions | the migration step |

and five cover serving, split by **pseudonymisation domain** — the clinical
record, the identity of its subject, and the map between the two:

| Role | Reads and writes | Barred from |
|---|---|---|
| `ferroehr_clinical` | `clinical`, archival tier included | `party`, `linkage` |
| `ferroehr_party` | `party`, archival tier included | `clinical`, `linkage` |
| `ferroehr_clinical_reader` | read-only over `clinical` | `party`, `linkage` |
| `ferroehr_party_reader` | read-only over `party` | `clinical`, `linkage` |
| `ferroehr_linkage` | `linkage` (the party-to-EHR map) | `clinical`, `party` |

The migrations create these roles idempotently, apply the per-schema grants,
**and revoke every other domain explicitly in both directions**, and revoke the
ability to create objects in the public schema. All five are `NOINHERIT` and
none is a member of another, so a boundary cannot be crossed by picking up a
membership. GDPR Art. 4(5) defines pseudonymisation as processing where
attribution to a person needs additional information "kept separately and
subject to technical and organisational measures", and Art. 32(1)(a) names it a
security measure for health data
(<https://eur-lex.europa.eu/eli/reg/2016/679/oj>); EDPB Guidelines 01/2025
require that separation to hold against internal actors, operators with
database access included.

Each serving role holds `SELECT`/`INSERT`/`UPDATE`/`DELETE` on its own domain's
tables and `EXECUTE` on the `ext` helper functions, and nothing else: it is not
a superuser, does not bypass row-level security, and cannot create, alter or
drop a table, index, schema or role. On the audit trail it is narrower still:
it may record an event and stamp it forwarded, and it holds no privilege that
can rewrite or remove one (see [Audit](audit.md)).

### Turning the schema split into a role split

The **schema** separation is unconditional: the server always reads and writes
parties in `party`, whatever it authenticates as, and each of the four pools
carries only its own schema on its `search_path`. The **role** separation is a
deployment choice, and it is one configuration table per domain:

```toml
[storage.party]
url = "postgres://ferroehr_party:***@pg:5432/ferroehr"

[storage.linkage]
url = "postgres://ferroehr_linkage:***@pg:5432/ferroehr"
```

(or `url_file`, for a mounted secret). With a domain's `url` set the server
opens that pool on that credential, and a flaw that reaches one of them reaches
one domain. Left unset, the domain uses `[db].url` and the separation is
schema-only. `linkage` holds which party is the subject of which EHR — the one
map that re-joins the other two — so `ferroehr_linkage` is barred from both of
them and both of them from it. Set every domain and no credential the server
uses can perform that join in SQL; the crossing happens in the application, over
two connections, and is recorded as an access event (see [Security → Resolving
across the boundary](security.md#resolving-across-the-boundary)).

A DSN that names another host moves the domain to a **database or cluster of its
own**, which is the separation a schema cannot make: a base backup, WAL
archiving and physical replication carry every schema of a database together
([PostgreSQL 18, Backup and
Restore](https://www.postgresql.org/docs/18/backup-dump.html)). One constraint:
the `linkage` migration set revokes a function the `party` set creates, so those
two are prepared in the same database, and a layout that splits them is refused
at boot with the remedy rather than failing partway through a migration.

Two boot checks make the posture honest. Two domains configured on **different**
DSNs that authenticate as the **same** database role are refused — a separation
that exists only in the configuration reads as one that holds, which is worse
than none. And a domain role that does not exist is a warning under
`deployment_profile = "sandbox"` and a refusal under `production`: absent roles
mean absent grants, and the boundary check would otherwise pass by having
nothing to measure.

### Which credential prepares the schema

**None of the domain credentials.** Preparing a database spans every schema in
it at once: the DDL of each resident migration set under `db.migrate = "apply"`,
and their `_sqlx_migrations` bookkeeping tables under `"verify"` — a read, but a
read across the whole database. Each runtime role holds exactly one domain, so
none can do it, and `verify` is not the exception: a role that is a member of
`ferroehr_clinical` and nothing else is refused on the very first set: it
cannot read the `ext`, `party`, `linkage` or `audit` bookkeeping.

So the credential that prepares the schema is named separately, and the server
uses it for that one boot step:

```toml
[db]
url = "postgres://ferroehr_clinical:***@pg:5432/ferroehr"
migrate_url = "postgres://ferroehr_migrator:***@pg:5432/ferroehr"

[storage.party]
url = "postgres://ferroehr_party:***@pg:5432/ferroehr"

[storage.linkage]
url = "postgres://ferroehr_linkage:***@pg:5432/ferroehr"
```

`migrate_url` prepares every domain that reaches the same DATABASE it does,
which is all of them in the posture above: three credentials, one database. A
domain whose DSN reaches a DIFFERENT database is prepared on its own DSN
instead, because `migrate_url` names one database and a relocated domain is not
in it. Which case a domain is in is read from the server itself
(`pg_control_system()` and `current_database()`), never from the DSN text — two
DSNs that differ only in their credential reach the same database.

(or `migrate_url_file`, for a mounted secret). The connection is opened for
preparation and closed again: no pool is held on it, and no request is ever
served through it. Left unset it falls back to `url`, which is exactly what a
single-credential deployment has always run — its behaviour is unchanged by
this key existing.

A credential that cannot read a set is told which schema and which role, so
the fix is visible from the message:

```text
database role `app_ehr` cannot read the migration state of schema
`ext`: … point `[db] migrate_url` (or `migrate_url_file`) at the credential
that prepares the schema — unset, it falls back to `[db] url`
```

Either way the server **refuses to boot** when the grants themselves are wrong:
a self-check enumerates every table, view, sequence and function in each
domain and fails, naming the role and the object, if either runtime role can
read across the boundary. `ferroehr db verify` runs the same check. When the
five roles do not exist at all (the development, compose and test-harness case,
where the migrator holds no `CREATEROLE`) the check warns rather than inventing
a failure — under `deployment_profile = "production"` it refuses, because absent
roles mean absent grants and there is then nothing to measure.

On Kubernetes the same choice is chart values, mounted as files the same way
the clinical DSN is, so no credential enters the pod's environment:

```yaml
database:
  existingSecret: ferroehr-db                 # postgres://ferroehr_clinical:…
  party:
    existingSecret: ferroehr-db-party         # postgres://ferroehr_party:…
  linkage:
    existingSecret: ferroehr-db-linkage       # postgres://ferroehr_linkage:…
  audit:
    existingSecret: ferroehr-db-audit         # the audit repository's own DSN
  migrateExistingSecret: ferroehr-db-migrator # postgres://ferroehr_migrator:…
```

Leave a domain's block unset and that pool uses the shared DSN, which is the
schema-only posture.

Provisioning the five roles is yours in both cases. The migrations create them
only when the migrator holds `CREATEROLE` and skip them with a `NOTICE`
otherwise, so on a managed database — where the migrator usually does not —
create them before the first deploy:

```sql
CREATE ROLE ferroehr_clinical NOLOGIN NOINHERIT;
CREATE ROLE ferroehr_party NOLOGIN NOINHERIT;
CREATE ROLE ferroehr_clinical_reader NOLOGIN NOINHERIT;
CREATE ROLE ferroehr_party_reader NOLOGIN NOINHERIT;
CREATE ROLE ferroehr_linkage NOLOGIN NOINHERIT;
```

then give each login role membership of exactly one of them. `ferroehr db
verify` tells you whether the boundary holds afterwards.

The local Audit Record Repository is written on its own pool
(`[storage.audit]`), and the `audit` schema is granted to `ferroehr_clinical`
(record an event, stamp it forwarded, run the retention reaper, verify the
chain) and to `ferroehr_clinical_reader` (read it and verify the chain). A
clinical login role that is a member of `ferroehr_clinical` alone writes its own
access log; no extra membership is needed. The audit trail is not a
pseudonymisation domain, so this grant adds no reach into `party` or `linkage`.

The compose stacks create all five and grant the clinical and party domains to
the single dev login role, which owns the database. That
demonstrates the schema separation and exercises the boot self-check; it is
deliberately **not** the credential separation, because one container with one
DSN cannot show that half honestly.

### Which of these postures is exercised, and which is not

A recommendation nothing runs is a guess. This is what the suites cover.

Exercised against a real PostgreSQL 18:

- Two runtime credentials serving both domains through the assembled router,
  each of the server's own pools refused every relation of the other domain
  (`app/ferroehr-rest/tests/it/credential_separation.rs`), and the single-DSN
  fallback still serving both.
- Three runtime credentials across the whole identity-to-party-to-EHR
  crossing, with the linkage credential refused the identifier map
  (`app/ferroehr/tests/it/pseudonymisation_boundary.rs`).
- All five domain roles refused every relation in the domains they do not own,
  enumerated from `information_schema` rather than from a written list (same
  file).
- The boot sequence on separated credentials with `[db] migrate_url`, under
  both `verify` and `apply`, and the single-DSN fallback under both
  (`app/ferroehr/tests/it/schema_preparation_credential.rs`).
- The compose stack's grant boundary read back from the running database by
  `scripts/deploy-probe.sh`, which also records that the stack is
  single-credential by design.

Not exercised by anything, and stated rather than left to inference:

- **A separated `migrate_url` alongside the linkage credential in one boot.**
  The preparation tests configure the clinical and party DSNs.
- **A domain actually relocated to another database**, as opposed to a second
  credential on the same one. The layout, the per-database preparation and the
  refusals are covered in process; no suite runs two PostgreSQL databases.
- **A real deployment on separated DSNs**: a container booting with the DSN
  files mounted, and the chart wiring them through
  `database.party.existingSecret`, `database.linkage.existingSecret` and
  `database.migrateExistingSecret`. What is covered is the in-process half.
- **Role provisioning on a managed database** where the migrator holds no
  `CREATEROLE` and the roles are created by the manual step above.

Which posture you actually get depends on `db.migrate`, because the server's
embedded migrations are DDL: a self-migrating deployment necessarily runs as a
role that can execute DDL. The single-container quickstart takes that path: its
DSN authenticates as a non-superuser role that owns the database and is a member
of `ferroehr_migrator`, `ferroehr_clinical`, `ferroehr_party` and
`ferroehr_linkage`.

## Applying migrations

`db.migrate` decides who runs the schema:

- **`apply`** (the default): the server applies its embedded migrations at
  boot. This is what makes a fresh checkout and an empty database work with no
  configuration at all, and it is the right choice for development and for
  small single-tenant deployments. The runtime DSN must be a member of
  `ferroehr_migrator`, so the serving process holds DDL rights for its whole
  life. Least isolation.
- **`verify`:** the server issues **no DDL at all**. At boot it checks that the
  database carries exactly this build's migrations and refuses to start
  otherwise, naming the schema and what is wrong with it. The pools can then
  hold no schema rights at all, which is the least-privilege production
  posture: an application-level SQL flaw can reach rows, never the schema. The
  check itself still reads every schema's bookkeeping, so it runs on
  `db.migrate_url` — see [which credential prepares the
  schema](#which-credential-prepares-the-schema), and set that key whenever
  `db.url` is a role that holds one domain.

Migrations are **append-only**, so upgrading an existing database in place is
the supported path and always has been the one your data takes. A released
migration is never edited: sqlx records a checksum of each applied file and
refuses a database whose recorded checksum no longer matches, so a corrected
schema arrives as a new migration rather than as a change to an old one. A CI
guard fails any pull request that modifies, renames or deletes a migration the
base branch already carries.

With `verify`, something else has to run the migrations first. Use the binary's
own subcommand under the migrator DSN: a CI/CD stage, a one-shot job, or the
Helm chart's `migrations.job.enabled` pre-install/pre-upgrade hook Job, which
Helm waits on so a failed migration fails the release (it takes its own
`migrations.job.existingSecret` holding the migrator DSN, deliberately a
different credential from the runtime one, and rendering fails without it):

```shell
FERROEHR__DB__URL='postgres://ferroehr_migrator:***@pg:5432/ferroehr' \
  ferroehr db migrate     # applies; exits when done
FERROEHR__DB__URL='postgres://ferroehr_clinical:***@pg:5432/ferroehr' \
FERROEHR__DB__MIGRATE_URL='postgres://ferroehr_migrator:***@pg:5432/ferroehr' \
  ferroehr db verify      # read-only check; exit 0 iff the schema is current
```

`db verify` splits the same way the boot sequence does, and for the same
reason: the schema state is read on `migrate_url`, while the pseudonymisation
boundary is measured on the runtime DSNs — asking the migrator credential
whether it can reach every domain would answer yes by design and tell you
nothing about the roles that serve requests.

Gate the rollout on the migration step so two server versions never race the
schema. Note the difference in failure shape: a `verify` server refuses to boot
against an unmigrated database (loud, immediate), while an `apply` server that
loses its schema later stays up and reports readiness `DOWN`, the warning
below.

> [!WARNING]
> **Migration is a boot step, and nothing re-runs it.** A running instance whose
> database is replaced, wiped, or reachable-but-empty does not migrate. It reports
>
> ```json
> {"status":"DOWN","components":{"db":{"status":"UP"},
>  "migrations":{"status":"DOWN","detail":"core schema tables missing (migrations not applied)"}}}
> ```
>
> on `/health/readiness` (`503`), leaves the load balancer's rotation, and keeps
> passing liveness (correctly, since the process is healthy) so nothing restarts
> it. Under Kubernetes that is a Deployment sitting at `0/N` ready with no error
> after the first one.
>
> The readiness check re-tests the schema on **every probe**, so recovery does not
> require a restart *of that instance*: it goes back to `UP` within one probe
> interval of the schema existing, whoever created it. What needs a restart is the
> case where the only thing that would migrate is the instance itself: then
> `kubectl rollout restart deploy/ferroehr` (or a migration job) is the remedy.
>
> For the out-of-band flow this means: the migration step must **complete before**
> the first instance starts, or that instance sits unready until the schema
> appears, harmless but confusing, and it delays the rollout rather than failing
> it. Gate the rollout on the migration job.

### Recovering a partially wiped database

A wipe that removes *some* of the server's schemas is not a fresh start, and the
server refuses to migrate over one rather than doing something plausible with it.

The server owns five schemas: `ext`, `clinical`, `party`, `linkage` and `audit`.
A fresh start drops all five. Each domain's cold archival tier is a **partition**
of the relation it archives, inside that domain's own schema, so a
`DROP SCHEMA clinical CASCADE` takes the archived rows with the live ones. There
is no separate mirror schema left to survive a wipe of the tier it mirrors.

Dropping the clinical schemas (`clinical`, `ext`, `audit`) while `party` and
`linkage` survive is a supported reset: the clinical migration set rebuilds its
schema on the next boot.

A database written by a release before the storage rewrite is refused outright:

```text
this database predates the storage rewrite: schema `ehr` carries its own
migration bookkeeping, which only a release before the rewrite wrote.
```

The new migration sets replace the old ones rather than upgrading them, so there
is no in-place path. Dump anything worth keeping, recreate the database, and
start the server against it.

> [!TIP]
> When you wipe a FerroEHR database deliberately, drop the **database**, not a
> schema. `DROP DATABASE` cannot leave half a repository behind, and it is the
> only wipe with no partial-state failure mode. `ferroehr db erase` drops all
> five schemas of a database in one transaction, for taking an instance out of
> service ([Decommissioning](operations-decommissioning.md)).

## TLS and database security

These are database-side settings that belong to whoever provisions PostgreSQL;
the deployment references them but cannot enforce them:

- **TLS in transit.** Require `hostssl` on the server and put
  `?sslmode=verify-full` in the DSN so the client verifies the server
  certificate.
- **pgaudit.** Run pgaudit as the database-layer complement to the openEHR
  audit and the ATNA trail (for example `pgaudit.log = 'ddl, role,
  connection'` globally plus object-level audit on the PHI tables) and ship
  the audit log to an immutable store with long (roughly six-year) retention.
- **Encryption at rest.** Encrypt at the volume or disk layer. Do not encrypt
  the stored clinical JSON with pgcrypto: it would break AQL's ability to
  query inside the data.

## Backup and point-in-time recovery

Enable WAL archiving and point-in-time recovery from day one (pgBackRest or a
managed PITR), because a CDR's data is not reconstructible. Clinical and audit
tables are never `UNLOGGED`. Test your restore, not just your backup.

### Dump each domain separately

A logical backup is where the three pseudonymisation domains are easiest to
re-join by accident. One `pg_dump` of the whole database produces a single file
holding the pseudonymised clinical record, the identities of its subjects and
the map between them, and whoever can read that file can re-identify every
record in it — which is the separation the schema split and the database roles
exist to maintain. **Three domains, three dumps**, into targets with different
access control.

**How many dumps, by DSN layout.** The number of dumps follows the domains, not
the databases: co-located, the four domains share one database and are dumped
`--schema` by `--schema` as below; relocated, each database is dumped for the
domains that live in it. What never changes is that the clinical record, the
identities and the map land in three different artefacts with three different
audiences. `ferroehr config check` prints the resolved layout, which is the
list a backup runbook is written from. The audit repository travels with the
clinical dump while it shares that database (`--schema=audit` below) and needs a
dump of its own once `[storage.audit]` names another one. On Kubernetes that
fourth dump is not yours to schedule: the chart renders a `backup.audit`
CronJob as soon as `database.audit` carries a Secret of its own, and drops
`--schema=audit` from the clinical job in the same render.

```bash
# The clinical domain
pg_dump --dbname="$CLINICAL_DSN" --format=custom --no-owner \
  --schema=clinical --schema=ext --schema=audit \
  --file=/backups/clinical/clinical-$(date -u +%Y%m%dT%H%M%SZ).dump

# The identities, into a different directory, owned by a different group
pg_dump --dbname="$PARTY_DSN" --format=custom --no-owner \
  --schema=party \
  --file=/backups/party/party-$(date -u +%Y%m%dT%H%M%SZ).dump

# The party-to-EHR map, into a third directory with the narrowest audience
pg_dump --dbname="$LINKAGE_DSN" --format=custom --no-owner \
  --schema=linkage --extension=btree_gist \
  --file=/backups/linkage/linkage-$(date -u +%Y%m%dT%H%M%SZ).dump

# The audit trail, ONLY when [storage.audit] names a database of its own —
# co-located it is in the clinical dump above, and dumping it twice is a second
# copy of who read what
pg_dump --dbname="$AUDIT_DSN" --format=custom --no-owner \
  --schema=audit \
  --file=/backups/audit/audit-$(date -u +%Y%m%dT%H%M%SZ).dump
```

> [!IMPORTANT]
> `--extension=btree_gist` on the linkage dump is load-bearing. A `--schema`
> dump carries no extension — *"pg_dump makes no attempt to dump any other
> database objects that the selected schema(s) might depend upon"* — and
> `linkage.subject_ehr`'s temporal `UNIQUE … WITHOUT OVERLAPS` is a GiST
> index over that extension's operator classes. Without the flag the restore
> reports *"data type uuid has no default operator class for access method
> gist"*, `pg_restore` ignores the error by default, and the table comes back
> with its rows and without the key that admits one open mapping per party.

The third dump is a separate artefact for the same reason the first two are,
and it is the one that matters most. `linkage.subject_ehr` says which party is
the subject of which EHR: it is the additional information that turns a
pseudonymised record back into a person (GDPR Art. 4(5)), so a file carrying it
beside either side of that map rebuilds the join the split exists to withhold.
Folding it into the party dump would put the identities and the map in one
holder's hands — the artefact this whole section is written to prevent.

All three examples ship. In Compose they are the opt-in `backup` profile:

```bash
docker compose --profile backup run --rm ferroehr-backup-clinical
docker compose --profile backup run --rm ferroehr-backup-party
docker compose --profile backup run --rm ferroehr-backup-linkage
```

Set `FERROEHR_BACKUP_CLINICAL_DIR`, `FERROEHR_BACKUP_PARTY_DIR` and
`FERROEHR_BACKUP_LINKAGE_DIR` to the three targets; they default to
`./backups/clinical`, `./backups/party` and `./backups/linkage`.
Run it as yourself — `FERROEHR_BACKUP_USER="$(id -u):$(id -g)"` — and the dump
lands owned by you. Left unset, the job runs as root inside the container and
keeps one capability, `DAC_OVERRIDE`, because that is what writing a directory
it does not own actually requires: the container drops every other capability,
and without this one uid 0 is just another user against your directory's
permission bits. Everything else stays off — no privilege escalation, read-only
root filesystem.
Under Kubernetes the chart renders one `CronJob` per domain — see the chart's
`backup` values.

> [!WARNING]
> **A backup credential is not the application role.** Each domain's runtime
> role is revoked from the other domains, which is the pseudonymisation
> boundary doing its job — so a dump taken through one of them is silently
> partial rather than refused. Give each backup job its own role, read-only on
> its own domain and nothing else.

Two further properties are yours to arrange, because no configuration file can
enforce them: the three targets carry **different** access control, and the
credential each job uses reaches **one** domain. Give the demographic job the demographic
DSN once you run two ([Deploying](installation/kubernetes.md)); with a single
credential you have separated the artefacts but not the authority to produce
them.

> [!NOTE]
> Point-in-time recovery stays instance-wide. All three domains live in one
> cluster, so a WAL archive covers them together and a recovery target restores
> them together. The separation this section is about is the logical dump.

### Restoring

A restore is not finished when `pg_restore` exits. The grants are the
boundary — a database whose roles came back wrong is a database where the
clinical role can read the identities — so re-apply the role grants before the
server starts, then let the server check them:

```bash
createdb ferroehr_restored
pg_restore --dbname=ferroehr_restored --no-owner clinical-….dump
pg_restore --dbname=ferroehr_restored --no-owner demographic-….dump
pg_restore --dbname=ferroehr_restored --no-owner linkage-….dump
# then, before anything serves traffic:
ferroehr db verify
```

Restore the clinical dump first. It is the one carrying the `ext` schema, whose
helper functions the other domains' relations depend on.

`ferroehr db verify` issues no DDL. It checks that the database carries exactly
this build's migrations, **all five sets**, so a restore that skipped a domain
is refused by name, and that no runtime role can read across the domain
boundary, exiting non-zero when either is untrue. It is the same check the
server runs at boot, which is why a server pointed at a badly restored database
refuses to start rather than serving from it.

What it does **not** read is a constraint. `pg_restore` ignores a failed
statement unless you pass `--exit-on-error`, so a table can come back with its
rows and without a key while every check above stays green. Read the restore's
own output, and confirm the map kept its temporal key:

```bash
psql -d ferroehr_restored -c \
  "SELECT conname, pg_get_constraintdef(oid) FROM pg_constraint
     WHERE conrelid = 'linkage.subject_ehr'::regclass AND contype = 'u'"
# expect: uq_subject_ehr_party | UNIQUE (party_id, sys_period WITHOUT OVERLAPS)
```

Restoring only one domain is a supported outcome, not a mistake: a demographic
dump restored on its own gives a database with identities and no clinical
record, which is what a rehearsal of the identity domain's recovery looks like.
Such a database is not one the server will serve from — `ferroehr db verify`
refuses it for the domains that are missing, which is the correct answer for a
rehearsal target.

## Rotating the national-identifier key

Only relevant when `[demographic.identifier_protection]` is on. With it on, a
national identifier of a configured scheme never sits in the versioned body:
the value lives in `party.national_identifier`, sealed with AES-256-GCM under a
key derived per domain from your root key, with an HMAC-SHA-256 digest beside it
so an identifier can be looked up without decrypting anything.

**The root key is load-bearing.** Lose it and the sealed identifiers cannot be
read back by anything, including you. Hold it the way you hold the database
credentials: in your secret manager, injected as `key_file`, never in the TOML
you commit.

Rotation re-encrypts; it is not an edit. Both keys have to be present while it
runs, because every row is opened under the old key and sealed under the new
one, and the lookup digest changes with the key too — so the rotation rewrites
the digest column and every stored reference keeps pointing at the same row.

The procedure:

1. Generate the new key: `openssl rand -hex 32`, or any source of 32 random
   bytes rendered as hex.
2. Take a backup and verify it restores. A rotation rewrites every row in the
   table; a half-finished one you cannot roll back is the failure mode here.
3. Stop writes to the demographic domain, or accept that identifiers written
   during the rotation are sealed under whichever key that write saw. A short
   maintenance window is simpler than reasoning about the alternative.
4. Run the rotation under the migrator role, which is the only identity with
   rights on the whole table.
5. Swap `key_file` to the new key and restart. Verify by resolving one known
   identifier and reading one party back.
6. Destroy the old key once step 5 is verified, not before.

> [!WARNING]
> There is no automated rotation command yet. Until there is, the rotation is a
> scripted job you run against the demographic database with both keys
> available, and this page is the procedure it has to follow. Treat the absence
> of a command as a reason to rehearse the rotation on a copy first.

The subkeys are derived from the root key rather than stored, so rotating the
root rotates all of them together. The pseudonymisation domain is part of that
derivation: a subkey derived for the clinical domain opens nothing in the party
one, so the per-schema backups above are separate artefacts under separate keys
even where one root key is configured.

## The container image and pod hardening

The published image is distroless and non-root (shell-less, with no package
manager) and is multi-architecture (amd64 and arm64) on GHCR. It is not a
static binary: the server links glibc and `libgcc_s` dynamically, which is why
the base image is the `cc` distroless variant rather than the smaller one, and
why the runtime needs no OpenSSL, no JVM and nothing else from a package
manager.

Under Kubernetes the pod runs the Kubernetes `restricted` profile: non-root
uid/gid 65532, read-only root filesystem with one writable `/tmp`, no
privilege escalation, an empty capability bounding set, `RuntimeDefault`
seccomp, and no service-account token (the workload never calls the Kubernetes
API). The chart's own gates assert that per container on every render, and the
posture read back from a running container, plus the two things the chart
cannot do for you (applying the namespace enforcement label, and narrowing the
NetworkPolicy's ingress sources: the shipped policy narrows the *ports* and,
until you set `networkPolicy.ingressFrom`, admits every source on them; set
`networkPolicy.ingressAllowAll: false`, and `viewer.networkPolicy.ingressAllowAll`
for the viewer, to have that state refused at render instead), are covered in
[Installation → The workload: security context & admission](installation/hardening-workload.md)
and [Namespaces, network & policy →
Ingress](installation/hardening-network-policy.md#ingress-ports-are-narrowed-sources-are-yours).

The Compose path carries a comparable floor, and one guard keeps it:
`scripts/checks/compose-hardening.sh` runs over every committed compose
artifact and fails on a service definition without `cap_drop: [ALL]` and
`no-new-privileges:true`, on any `privileged: true`, on a `seccomp=unconfined`
or `apparmor=unconfined` override, on a mounted Docker daemon socket, and on a
published port that names no host address. The compose files go further than
the guard checks (capabilities are added back one at a time only where an
entrypoint provably needs them, file-descriptor limits are bounded, ports
default to the loopback interface, and most services run a read-only root
filesystem) but those extras are conventions, not enforced properties, so
check them when you adapt a file.

Two services in the quickstart file deliberately do not run read-only, and each
says why in place. The S3 gateway writes its volume store; the server cannot,
because Compose refuses an inline `config` in a read-only service and that
inline config is what makes the file standalone. A deployment that mounts its
configuration from a file instead can add `read_only: true` and a tmpfs at
`/tmp`.

### What the host owes, and we cannot enforce

Four controls belong to whoever runs the daemon. They are stated here because a
container hardening story that ignores them is misleading: the strongest pod
security context in the world sits on top of these.

**Keep the host kernel and Docker Engine current.** A container is a kernel
namespace, not a virtual machine: a kernel privilege-escalation bug is a container
escape, and every runtime hardening above assumes the kernel enforcing it is
patched. Track your distribution's kernel updates and the Docker Engine release
notes with the same urgency you would give a public-facing service.

**Prefer rootless mode.** Running the daemon as a non-root user means a container
escape lands as an unprivileged user rather than as root on the host
(<https://docs.docker.com/engine/security/rootless/>). The images here need no
privileged operation, no host networking, and no daemon socket, so nothing in this
deployment prevents rootless: the constraints are usually the host's (cgroup v2,
`newuidmap`/`newgidmap`, and no privileged ports below 1024, which is why the
server binds 8080 rather than 80).

**Set the daemon log level to `info` (the default) and keep it.** Docker's `debug`
level records request payloads and can put secret material into daemon logs, which
are typically world-readable to anyone with host log access and are shipped
wholesale to log aggregators.

**Control who can pull and push your images.** GHCR access is the deployment's
authorization boundary for what runs in production: whoever can push a tag your
manifests reference can run their code with your database credentials. Restrict
push rights, prefer digest pins over mutable tags for anything you deploy (the
compose files pin the third-party images by digest for exactly this reason), and
verify the published attestations before rollout; the verification command is in
[Installation → Kubernetes & Helm](installation/kubernetes.md).

None of these four is something this project can assert on your behalf, which is
why they are written as your checklist rather than as our claim.

## Upgrades

- **Upgrade in place; the migrations are append-only.** A released migration
  is never edited again, so a database created by an earlier release takes the
  new release's migrations on top of the ones it already carries. The server
  records the checksum of every applied migration and refuses to start against
  a database whose recorded checksums differ from the files it carries
  (`migration 1 was previously applied but has been modified`), which is why
  editing one would lock every existing installation out of its own database
  rather than revise its history. A correction ships as a new migration, and a
  CI guard fails any pull request that modifies, renames or deletes one the
  base branch already has. A rolling upgrade must still stay compatible with
  the _previous_ schema for the window in which both versions run: additive
  changes first, destructive changes a release later.

  Releases up to and including 4.0.18 predate this policy and did edit
  migration files in place. A database created by one of those and never
  upgraded since is recreated rather than migrated.
- **Sweep for stale decomposition after a release that changes how content is
  decomposed.** The version body is unaffected and every read serves it
  correctly; what is stale is the decomposed index over it, which a
  row-level query predicate depends on. `POST {base}/admin/integrity/verify`
  reports each such version as `stale_decomposition` and
  `POST {base}/admin/integrity/rebuild-nodes`, run once unscoped, rewrites the
  rows from the stored document. The changelog names the affected object type
  at each release that needs it; 4.2.0 is one, for demographic parties. Both
  routes are on
  [Admin & messaging APIs](operations-admin-apis.md#storage-integrity).
- **Bound the DDL yourself.** Every pooled connection carries the
  `db.statement_timeout_ms` value (60 seconds by default), and the migration
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
  short shutdown drain of the audit and event outboxes.

### How long the version you pinned is supported

Plan the upgrade cadence around this:

- **Each release is supported for five years from the month it is
  published, and its fixes ship in the newest release.** There are no
  maintenance branches, no long-term-support line and no backports: a
  vulnerability in a release inside its support period is fixed in the next
  release, which every user of an earlier release may run under the same
  licence terms (CRA Art 13(10)). Staying on an older release means running
  without the fix.
- **A fix normally arrives as the next patch** on the current minor, so taking
  it does not oblige you to take new behaviour, but that is the usual case,
  not a promise. Where a fix is only correct alongside a behavioural change,
  the release carrying it carries the change, and the changelog entry says so.
- **A published release is never repaired in place.** Release immutability
  means the assets and the tag of a published release cannot be modified at
  all, so the remedy for any defect is a new version.
- **The Helm chart and the published crates follow their own version lines**,
  each with the same five-year period and its fixes in the newest published
  version. A chart-only fix ships as a new chart version between server
  releases.

**The running server states its own support period.** Each release is
supported for five years from the month of its release date (CRA Art 13(8)),
and the server reads that date from its changelog heading at build time. The
end month is on the boot banner (`Support` line), in the boot log, in
`ferroehr --version`, under `support` in `GET /management/info`, and under
`support` in `GET /ferroehr/rest/status` as
`{release_date, support_ends, status}`, where `support_ends` is `YYYY-MM` and
`status` is `supported`, `ended` or `unreleased`. Once the end month has passed,
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

## Observability

`tracing` is the single instrumentation API. From it, three signal families
fan out, and **identified data never enters any of them**: telemetry uses
only closed-set labels and opaque request/trace ids, so correlation to a
patient is possible only through the [audit trail](audit.md).

- **Logs** go to stdout (JSON when not attached to a terminal, pretty on a
  TTY) each line stamped with the trace and span id. Shipping and rotation
  are the platform's job. `FERROEHR__LOG__FORMAT` (`auto`/`json`/`pretty`) and
  `FERROEHR__LOG__FILTER` (or `RUST_LOG`, default `info,ferroehr=info`) control
  them, and the level can be changed at runtime through the `loggers` endpoint
  below. On boot the server prints a one-time ASCII banner (version, maintainer,
  project URL, and spec pins) to stdout ahead of the logs; it is suppressed
  under `FERROEHR__LOG__FORMAT=json` so machine log consumers see only structured
  lines.
- **Traces** export to any OpenTelemetry collector (Tempo, Jaeger, and so on)
  over OTLP, but only when you configure an endpoint; with none set, the
  tracing layer is not installed at all (zero overhead). Root spans are named
  by route template, never by a path containing ids.
- **Metrics** come from **one** OpenTelemetry meter provider with up to two
  readers: a Prometheus reader behind `/management/prometheus`, and (when
  `telemetry.metrics_push` is on) a periodic OTLP reader. Every instrument
  reaches both by construction, so a family can never exist on the scrape
  surface and be missing from the push. The catalogue covers HTTP request
  duration, active requests and body sizes; authentication failures and
  authorization decisions (Cedar and remote-PDP); database pool state,
  acquire latency and transaction counts; AQL query counts, latency and plan-
  cache events; compositions committed (by openEHR audit change type);
  validation failures and version-signature faults; WebTemplate cache events;
  events published; the whole ATNA audit pipeline; Tokio runtime gauges;
  process start time; and the `ferroehr_build_info` identity.

  Instrument names carry **no** `_total` suffix and no unit suffix: units are
  declared on the instrument and the Prometheus exporter derives
  `_total`/`_seconds`/`_bytes` itself. Read the exposition to learn the exact
  rendered names rather than assuming either spelling.

The telemetry environment variables:

| Environment variable | Default | Meaning |
|---|---|---|
| `FERROEHR__TELEMETRY__OTLP_ENDPOINT` | unset (layer not installed) | OTLP collector endpoint |
| `FERROEHR__TELEMETRY__SERVICE_NAME` | `ferroehr` | reported service name |
| `FERROEHR__TELEMETRY__ENVIRONMENT` | `dev` | reported deployment environment |
| `FERROEHR__TELEMETRY__TRACES_SAMPLE_RATIO` | `1.0` | head sampling ratio (start at `0.1` in production) |
| `FERROEHR__TELEMETRY__METRICS_PUSH` | `false` | add the periodic OTLP metrics reader beside the Prometheus one |
| `FERROEHR__TELEMETRY__FLAME_FILE` | unset (layer not installed) | write folded span-timing samples to this file for offline rendering, diagnostic sessions only |

Neither reader is reachable until you say so: the Prometheus surface needs
`management.enabled` plus an access level on the `prometheus` endpoint (see
[the management surface](#the-management-surface)), and the OTLP reader needs
both `otlp_endpoint` and `metrics_push`. A server with neither still records
every instrument; nothing exports it.

> [!TIP]
> A single-container dev stack (`grafana/otel-lgtm`, bundling an OTLP
> collector, Prometheus, Tempo, Grafana, and Loki) ships as a Compose overlay,
> together with a provisioned Grafana dashboard (request rate/errors/duration,
> database pool, AQL latency, validation failures, audit health) and a starter
> alert pack; point the server at it with the two OTLP variables above.

**On Kubernetes**, the same keys arrive through the chart's `config`
passthrough, and the metrics half has a second switch that is easy to miss:

```yaml
# values.yaml
config:
  telemetry:
    otlp_endpoint: http://otel-collector.observability:4317
    environment: production
    traces_sample_ratio: 0.1
    metrics_push: true
  management:
    enabled: true
    endpoints:
      prometheus: admin_only   # the scrape endpoint is off until you name it
metrics:
  enabled: true                # adds the prometheus.io/* pod annotations
  serviceMonitor:
    enabled: false             # or true, with the Prometheus Operator CRDs installed
```

`metrics.enabled` only adds the scrape **annotations**; the endpoint itself is
opened by `config.management.endpoints.prometheus`. Both are needed for an
annotation-discovering Prometheus, and neither is needed if you push over OTLP
instead. **To turn telemetry off**, drop `otlp_endpoint`; the tracing layer is
not installed at all when it is unset.

**The default dashboard.** The "FerroEHR — service overview" Grafana
dashboard (request rate/errors/latency, AQL rate and phase latency,
plan-cache hit ratio, database pool, Tokio runtime, audit throughput — every
query written against the served metric names) reaches Grafana three ways:

- **Compose**: the observability overlay provisions it automatically;
  nothing to do.
- **Kubernetes**: set `metrics.grafanaDashboard.enabled: true` and the chart
  ships it as a ConfigMap labelled `grafana_dashboard: "1"`, which the
  Grafana Helm chart's dashboard sidecar (kube-prometheus-stack includes it)
  discovers and imports. The sidecar watches its own release namespace by
  default — install FerroEHR there, or widen the sidecar's
  `searchNamespace`. `metrics.grafanaDashboard.folder` sets the
  `grafana_folder` annotation for sidecars with folder routing configured.
- **Anywhere else**: import
  `deploy/helm/ferroehr/files/dashboards/ferroehr-overview.json` by hand
  (Grafana → Dashboards → Import).

> [!WARNING]
> With the chart's default-deny egress policy on, add the collector to
> `networkPolicy.egress.rules` (port 4317). An OTLP exporter that cannot reach
> its collector fails **silently**: no traces, no error.

### The usage report

Apart from your own telemetry, the server sends one report of its own: the
[usage report](usage-report.md) to FerroPULSE, the collector Cadasto B.V.
runs. It is on by default and goes out at each start and once a day, with the
version, the licence grant type and coarse latency figures, and no patient
data. `ferroehr usage-report --print` shows the exact JSON, and
`usageReport.enabled=false` (Helm) or `FERROEHR__USAGE_REPORT__ENABLED=false`
switches it off. With the chart's egress policy on, it needs a rule for
`report.ferropulse.eu` on port 443; without one it fails quietly.

## The admin and messaging APIs

Two operator-facing HTTP surfaces have a page of their own, because each route
carries its own switch, authorization class and status-code contract:

- **The admin API** (`{base}/admin/…`, off by default): physical deletion, the
  activity report, archiving to the cold tier, and whole-repository dump and
  load.
- **The messaging API** (`{base}/message/…`, always mounted): EHR Extract
  export and import, and Template Data Document import.

The full reference is [Admin & messaging APIs](operations-admin-apis.md).

> [!WARNING]
> Enabling the admin API puts irreversible, whole-repository operations on the
> wire: `DELETE {base}/admin/ehr/all` with no parameter empties the
> repository. Keep it off unless a workflow needs it, turn RBAC on, and gate
> the admin role tightly.

## Health probes

The health endpoints are **always served, on the main API port, without
authentication**. There is nothing to enable and nothing to remember: they are
mounted outside the API's authentication and overload-shedding layers, so an
orchestrator can probe a server whose management surface, admin API, and every
optional integration are switched off, and a saturated server still answers
its own probes.

### Choosing a health endpoint

| Endpoint | Contract | Use it for |
|---|---|---|
| `GET /health` | constant `200 OK` (plain text `OK`), touches nothing | load balancers, `docker` `HEALTHCHECK`, anything that must never be auth-gated |
| `GET /health/liveness` | identical to `/health`, the same constant answer under the orchestrator-conventional path | Kubernetes `livenessProbe` and `startupProbe` |
| `GET /health/readiness` | `200` when the aggregate is up or degraded, `503` when a **required** component is down; JSON body with every indicator, each bounded to one second | Kubernetes `readinessProbe`, ops dashboards |
| `GET /ferroehr/rest/status` | product status document: `status`, `server_version`, `openehr_rest_api_version`, `timestamp`, `licence` (the grant in force: `state`, `use`, `licensee`, `not_after`, `configured_token`) `deployment` (the declared profile, the separations still open and the ones accepted by name) and `support` (`release_date`, `support_ends`, `status`) | version/identity checks; the URL the container's `ferroehr healthcheck` subcommand probes |
| `GET /management/*` | ops introspection; see below | operators, off by default, enable deliberately |

Every management request is itself recorded in the audit trail as a
system-domain access under its own operation id (`management_info`,
`management_env`, `management_loggers_set`, …), so a runtime filter change or a
configuration read is never an unrecorded administrative act.

There is exactly one health surface: the `/health` family above. `/health` and
`/health/liveness` are two conventional names for the same constant answer (a
load balancer wants the bare path, an orchestrator wants the `liveness`/
`readiness` pair); `/ferroehr/rest/status` is a different contract, and no health
endpoint exists under the REST root.

Not every indicator blocks readiness, and the distinction is deliberate:

| Indicator | Checks | Blocks readiness |
|---|---|---|
| `db` | a pooled connection answers | yes |
| `migrations` | this build's schema is present, re-tested on every probe | yes |
| `audit_sender` | the audit posture: `DEGRADED` with the consequence stated when auditing is off (no access log, no EHDS logging component); `UP` with `fail_mode`, the local store and its retention in the detail, and a stated caution under `fail_mode = "open"` | no: reports `DEGRADED`, never `503` |
| `audit_chain` | the latest scheduled verification of the local audit store's hash chain (present when the local store is on and `[audit.store] verify_interval_seconds` is not `0`): `UP` while intact or before the first run, `DOWN` naming the finding count and the first damaged position, `DEGRADED` when the verification could not run | no: a damaged chain reports `DOWN` in the body, never `503`, because the trail keeps recording while the damage is investigated |
| `events` | the event publisher's broker delivery (present only when eventing is enabled) | no: reports `DEGRADED`, never `503`, since the outbox buffers while the broker is down |

An instance whose event broker is unreachable therefore keeps taking traffic
and says so in the body; an instance that cannot reach its database, or whose
database lost the schema, leaves rotation. The `detail` strings are written to
carry no connection information (no DSN host, database name or role) because
this surface is unauthenticated by design.

> [!IMPORTANT]
> Liveness and readiness are deliberately different: liveness never touches a
> dependency, so a database outage takes the instance out of rotation
> (readiness `503`) instead of getting the container killed and restarted in a
> loop. Wire `livenessProbe` and `startupProbe` to `/health/liveness` and
> `readinessProbe` to `/health/readiness`. The Helm chart does exactly this out
> of the box, and only the timings are tunable (`probes.liveness`,
> `probes.readiness`, `probes.startup`); there is deliberately no option to
> point the probes at the container's `ferroehr healthcheck` subcommand
> instead, because that subcommand probes the status document rather than a
> health endpoint, which would leave readiness never touching the database.

## The management surface

The management surface is **ops introspection only**: build info, Prometheus,
the metric views, the effective configuration, and runtime log control. It is
**off by default** on the bare binary, and each endpoint is independently
opt-in with an access level (`admin_only`, `private`, or `public`). It can be
bound to its own internal port so it never appears on the public API listener.
Keeping it off costs you nothing operationally: the health probes above do not
depend on it.

| Environment variable | Default | Meaning |
|---|---|---|
| `FERROEHR__MANAGEMENT__ENABLED` | `false` | enable the management surface |
| `FERROEHR__MANAGEMENT__BASE_PATH` | `/management` | base path for the surface |
| `FERROEHR__MANAGEMENT__PORT` | unset (main listener) | serve management on its own port |
| `FERROEHR__MANAGEMENT__ENDPOINTS__<NAME>` | `off` | the access level for ONE endpoint: `off`, `private`, `admin_only` or `public`. There is no global default beside it: an endpoint you do not name is not mounted and answers `404`. |

The ops endpoints:

Every one of them ships `off`: nothing is mounted until you name the endpoint
and the level it should answer at. The right-hand column is the level to choose,
not a default you already have.

| Endpoint | Endpoint name to set | Purpose | Level to give it |
|---|---|---|---|
| `GET /management/info` | `info` | product name and version, build SHA, build date, `rustc`, the active `spec_profile`, the openEHR specification versions that profile selects, the PostgreSQL target, the `audit` posture (`enabled`, `fail_mode`, `local_store`, `retention_days`), the `manufacturer` of the release (`name`, `postal_address`, `email`, `website`), and the `support` period (`release_date`, `support_ends`, `status`) | `admin_only` |
| `GET /management/prometheus` | `prometheus` | Prometheus text exposition | `admin_only`, or `public` only when the port is not reachable outside the cluster; a `public` endpoint is served OUTSIDE authentication |
| `GET /management/metrics` | `metrics` | JSON list of the registered metric names | `admin_only` |
| `GET /management/metrics/{name}` | `metrics` | the current value(s) of one metric; `404` for a name that is not registered | `admin_only` |
| `GET /management/env` | `env` | effective configuration, with secrets redacted | `admin_only` |
| `GET`/`POST`/`DELETE` `/management/loggers` | `loggers` | read and change the log level at runtime | `admin_only` |
| `GET /management/flamegraph` | `flamegraph` | on-demand CPU flamegraph of the running server | `admin_only` |

Paths above show the default `management.base_path`; change it and every path
moves with it. The `metrics` name covers both metric routes, and the
`prometheus`, `metrics` and `loggers` routes additionally need their backing
machinery present; without it they are simply not mounted, which is the same
`404` as leaving them `off`.

> [!WARNING]
> `/management/env` and `/management/loggers` expose and change server
> internals; keep them `admin_only`, and prefer binding the surface to an
> internal-only port.

### Profiling: the on-demand CPU flamegraph

When the server is measurably slow, the metrics tell you *how slow*;
`/management/flamegraph` tells you **where the time goes**. The endpoint samples the
whole process with an in-process sampling profiler (the
[`pprof`](https://docs.rs/pprof/latest/pprof/) crate) for a bounded window and
answers with a rendered flamegraph SVG; open it in a browser and read the wide
frames.

```bash
# sample 10 s at 99 Hz (the defaults) and open the result
curl -u admin:… -o flamegraph.svg \
  "http://cdr.internal:9100/management/flamegraph?seconds=10&frequency=99"
```

- `seconds` (default 10) and `frequency` (default 99 Hz) are capped by
  `management.profiling.max_seconds` and `max_frequency` (30 and 999 out of the
  box); a request beyond a cap is refused with `400`, never silently clamped.
- **One sample window at a time**: a second request while one runs answers
  `409`; retry when the window completes.
- Sampling is low-overhead but not free; profile under the real load you are
  diagnosing, and keep the endpoint `admin_only` on an internal port like the
  rest of the surface.
- Best results come from the container images and release builds, which keep
  line tables (`debug = "line-tables-only"`) so frames resolve to `file:line`.

With the surface enabled, the viewer grows an **Operations** screen over
it (dependency health, build provenance, the metric registry, and runtime log
control) which appears only while the CDR serves `/management/info`. See
[Viewer → Operations panel](viewer/operations.md).

## The manufacturer and the deployment report

Cadasto B.V. is the manufacturer of each tagged release. The running system
names it, with its postal address and single point of contact, on every
surface an operator or a user reads:

| Surface | What it shows |
|---|---|
| startup banner | `Manufactured by Cadasto B.V. · https://www.cadasto.com/contact/`, then the postal address and `info@cadasto.com` |
| `ferroehr --version` | the version, then `Manufactured by Cadasto B.V., Comeniusstraat 2d, 1817 MS Alkmaar, The Netherlands, info@cadasto.com` and the website |
| `GET /management/info` | a `manufacturer` object with `name`, `postal_address`, `email` and `website` |
| `OPTIONS` on the API base path | `vendor` defaults to `Cadasto B.V.` ([`[server.identity]`](installation/config-server.md#serveridentity)) |
| viewer | the footer of every screen, and a Manufacturer card on `/system` |
| container images | the `eu.ferroehr.image.manufacturer` label |

`ferroehr report` writes one JSON file naming what a deployment runs: the
version, commit and build date, the enabled features, the specification pins
and `openehr-*` crate versions, the schema migration level, the deployment
posture, the licence summary, the manufacturer and the effective
configuration with every secret redacted. Attach it to a complaint or a
serious-incident report; see
[`ferroehr report`](installation/config-cli.md#ferroehr-report) for what it
holds and what it leaves out.

## Next

- [Admin & messaging APIs](operations-admin-apis.md) — the operator-facing
  HTTP surfaces, route by route.
- [Usage report](usage-report.md) — what the daily report to FerroPULSE
  carries, and how to switch it off.
- [Configuration reference](installation/configuration.md) — every setting,
  with the server, database and telemetry keys on
  [their own page](installation/config-server.md).
- The API reference at `/ferroehr/rest/swagger-ui` on your own deployment, or
  <https://sandbox.ferroehr.eu/ferroehr/rest/swagger-ui> on the hosted
  sandbox — the document the server itself generates.
