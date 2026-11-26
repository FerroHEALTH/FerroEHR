# Database roles, migrations and backup

The PostgreSQL side of running FerroEHR: the least-privileged roles, applying the
migrations, and backing up and restoring each domain.

<!-- toc -->

## Database roles and least privilege

FerroEHR connects to an **external** PostgreSQL 19: a managed service or an
operator-run cluster, never a chart-side sidecar, because a database holding
PHI must be independently backed up and recoverable. The server carries only a
connection string, ideally sourced from a secret.

The database never runs as a superuser at runtime. Two roles cover
provisioning and migration:

| Role | Purpose | Used by |
|---|---|---|
| owner | owns the database | provisioning only |
| `ferroehr_migrator` | runs the schema migrations; owns the helper functions | the migration step |

and four cover serving, split by **pseudonymisation domain**, the clinical
record and the identity of its subject:

| Role | Reads and writes | Barred from |
|---|---|---|
| `ferroehr_clinical` | `clinical`, archival tier included | `party` |
| `ferroehr_party` | `party`, archival tier included | `clinical` |
| `ferroehr_clinical_reader` | read-only over `clinical` | `party` |
| `ferroehr_party_reader` | read-only over `party` | `clinical` |

The migrations create these roles idempotently, apply the per-schema grants,
**and revoke every other domain explicitly in both directions**, and revoke the
ability to create objects in the public schema. All four are `NOINHERIT` and
none is a member of another, so a boundary cannot be crossed by picking up a
membership. GDPR Art. 4(5) defines pseudonymisation as processing where
attribution to a person needs additional information "kept separately and
subject to technical and organisational measures", and Art. 32(1)(a) names it a
security measure for health data
(<https://eur-lex.europa.eu/eli/reg/2016/679/oj>); EDPB Guidelines 01/2025
require that separation to hold against internal actors, operators with
database access included.

Each serving role holds `SELECT`/`INSERT`/`UPDATE`/`DELETE` on its own domain's
tables, `EXECUTE` on the `ext` helper functions, and `MAINTAIN` on its store's
partitioned tables (and `ehr`, for `ferroehr_clinical`), and nothing else: it is not
a superuser, does not bypass row-level security, and cannot create, alter or
drop a table, index, schema or role. On the audit trail it is narrower still:
it may record an event and stamp it forwarded, and it holds no privilege that
can rewrite or remove one (see [Audit](audit.md)).

`MAINTAIN` is there so the server can keep those tables' planner statistics
(see [Planner statistics of the partitioned tables](#planner-statistics-of-the-partitioned-tables)).
It also allows `VACUUM`, `CLUSTER`, `REINDEX` and `LOCK TABLE` on exactly
those tables. The role can already write every row of them, so what it adds
is the locks and index rebuilds those commands take. The reader roles do not
hold it.

### Turning the schema split into a role split

The **schema** separation is unconditional: the server always reads and writes
parties in `party`, whatever it authenticates as, and each of the three pools
carries only its own schema on its `search_path`. The **role** separation is a
deployment choice, and it is one configuration table per domain:

```toml
[database.party]
url = "postgres://ferroehr_party:***@pg:5432/ferroehr"
```

(or `url_file`, for a mounted secret). With a domain's `url` set the server
opens that pool on that credential, and a flaw that reaches one of them reaches
one domain. Left unset, the domain uses `[database].url` and the separation is
schema-only. Set both domains and no credential the server uses can join a
party to its EHR in SQL; the crossing happens in the application, over
two connections, and is recorded as an access event (see [Security → Resolving
across the boundary](pseudonymisation.md#resolving-across-the-boundary)).

A DSN that names another host moves the domain to a **database or cluster of its
own**, which is the separation a schema cannot make: a base backup, WAL
archiving and physical replication carry every schema of a database together
([PostgreSQL 18, Backup and
Restore](https://www.postgresql.org/docs/18/backup-dump.html)).

Two boot checks make the posture honest. Two domains configured on **different**
DSNs that authenticate as the **same** database role are refused — a separation
that exists only in the configuration reads as one that holds, which is worse
than none. And a domain role that does not exist is a warning under
`deployment.profile = "sandbox"` and a refusal under `production`: absent roles
mean absent grants, and the boundary check would otherwise pass by having
nothing to measure.

### Which credential prepares the schema

**None of the domain credentials.** Preparing a database spans every schema in
it at once: the DDL of each resident migration set under `[database] migrations = "apply"`,
and their `_sqlx_migrations` bookkeeping tables under `"verify"` — a read, but a
read across the whole database. Each runtime role holds exactly one domain, so
none can do it, and `verify` is not the exception: a role that is a member of
`ferroehr_clinical` and nothing else is refused on the very first set: it
cannot read the `ext`, `party` or `audit` bookkeeping.

So the credential that prepares the schema is named separately, and the server
uses it for that one boot step:

```toml
[database]
url = "postgres://ferroehr_clinical:***@pg:5432/ferroehr"
migrate_url = "postgres://ferroehr_migrator:***@pg:5432/ferroehr"

[database.party]
url = "postgres://ferroehr_party:***@pg:5432/ferroehr"
```

`migrate_url` prepares every domain that reaches the same DATABASE it does,
which is all of them in the posture above: three credentials, one database. A
domain whose DSN reaches a DIFFERENT database is prepared there on that
database's own migrator, `[database.<domain>]` migrate_url`, because
`migrate_url` names one database and a relocated domain is not in it; unset, it
falls back to the domain's own DSN, which a DML-only role cannot use. Which case a domain is in is read from the server itself
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
`ext`: … point `[database] migrate_url` (or `migrate_url_file`) at the credential
that prepares the schema — unset, it falls back to `[database] url` — and, for a
domain in a database of its own, `[database.<domain>]` migrate_url` at that
database's migration credential
```

Either way the server **refuses to boot** when the grants themselves are wrong:
a self-check enumerates every table, view, sequence and function in each
domain and fails, naming the role and the object, if either runtime role can
read across the boundary. `ferroehr db verify` runs the same check. When the
four roles do not exist at all (the development, compose and test-harness case,
where the migrator holds no `CREATEROLE`) the check warns rather than inventing
a failure — under `deployment.profile = "production"` it refuses, because absent
roles mean absent grants and there is then nothing to measure.

On Kubernetes the same choice is chart values, mounted as files the same way
the clinical DSN is, so no credential enters the pod's environment:

```yaml
database:
  existingSecret: ferroehr-db                 # postgres://ferroehr_clinical:…
  party:
    existingSecret: ferroehr-db-party         # postgres://ferroehr_party:…
  audit:
    existingSecret: ferroehr-db-audit         # the audit repository's own DSN
  migrateExistingSecret: ferroehr-db-migrator # postgres://ferroehr_migrator:…
```

Leave a domain's block unset and that pool uses the shared DSN, which is the
schema-only posture.

Provisioning the four roles is yours in both cases. The migrations create them
only when the migrator holds `CREATEROLE` and skip them with a `NOTICE`
otherwise, so on a managed database — where the migrator usually does not —
create them before the first deploy:

```sql
CREATE ROLE ferroehr_clinical NOLOGIN NOINHERIT;
CREATE ROLE ferroehr_party NOLOGIN NOINHERIT;
CREATE ROLE ferroehr_clinical_reader NOLOGIN NOINHERIT;
CREATE ROLE ferroehr_party_reader NOLOGIN NOINHERIT;
```

then give each login role membership of exactly one of them.
`ferroehr db verify` tells you whether the boundary holds afterwards.

The local Audit Record Repository is written on its own pool
(`[database.audit]`), and the `audit` schema is granted to `ferroehr_clinical`
(record an event, stamp it forwarded, run the retention reaper, verify the
chain) and to `ferroehr_clinical_reader` (read it and verify the chain). A
clinical login role that is a member of `ferroehr_clinical` alone writes its own
access log; no extra membership is needed. The audit trail is not a
pseudonymisation domain, so this grant adds no reach into `party`.

The compose stacks create the four domain roles and grant the clinical and party domains to
the single dev login role, which owns the database. That
demonstrates the schema separation and exercises the boot self-check; it is
deliberately **not** the credential separation, because one container with one
DSN cannot show that half honestly.

### Which of these postures is exercised, and which is not

A recommendation nothing runs is a guess. This is what the suites cover.

Exercised against a real PostgreSQL 19:

- Two runtime credentials serving both domains through the assembled router,
  each of the server's own pools refused every relation of the other domain
  (`app/ferroehr-rest/tests/it/credential_separation.rs`), and the single-DSN
  fallback still serving both.
- All four domain roles refused every relation in the domain they do not own,
  enumerated from `information_schema` rather than from a written list
  (`app/ferroehr/tests/it/pseudonymisation_boundary.rs`).
- The boot sequence on separated credentials with `[database] migrate_url`, under
  both `verify` and `apply`, and the single-DSN fallback under both
  (`app/ferroehr/tests/it/schema_preparation_credential.rs`).
- The party domain relocated to a second database, prepared and verified
  there on `[database.party]`, and the
  refusal when the domain's two DSNs reach different databases
  (`app/ferroehr/tests/it/relocated_domain_preparation.rs`).
- The chart's production overlay on three PostgreSQL clusters, one per domain,
  prepared by the migration Job and booting under `production`
  (`scripts/deploy-probe-k8s.sh --production-only`, probes P-K8S-PROD-BOOT and
  P-K8S-PROD-SHARED).
- The compose stack's grant boundary read back from the running database by
  `scripts/deploy-probe.sh`, which also records that the stack is
  single-credential by design.

Not exercised by anything, and stated rather than left to inference:

- **A real deployment on separated DSNs**: a container booting with the DSN
  files mounted, and the chart wiring them through
  `database.party.existingSecret` and `database.migrateExistingSecret`. What is covered is the in-process half.
- **Role provisioning on a managed database** where the migrator holds no
  `CREATEROLE` and the roles are created by the manual step above.

Which posture you actually get depends on `[database] migrations`, because the server's
embedded migrations are DDL: a self-migrating deployment necessarily runs as a
role that can execute DDL. The single-container quickstart takes that path: its
DSN authenticates as a non-superuser role that owns the database and is a member
of `ferroehr_migrator`, `ferroehr_clinical` and `ferroehr_party`.

## Applying migrations

`[database] migrations` decides who runs the schema:

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
  `[database] migrate_url` — see [which credential prepares the
  schema](#which-credential-prepares-the-schema), and set that key whenever
  `database.url` is a role that holds one domain.

Migrations are **append-only** within a major release line: a released
migration is never edited, because sqlx records a checksum of each applied file
and refuses a database whose recorded checksum no longer matches, so a
corrected schema arrives as a new migration rather than as a change to an old
one. A CI guard fails any pull request that modifies, renames or deletes a
migration a release carries. v6.0.0 starts the line again from one baseline per
schema, so a v5 database is not upgraded in place; see
[Upgrading from v5 to v6](upgrades.md#upgrading-from-v5-to-v6).

With `verify`, something else has to run the migrations first. Use the binary's
own subcommand under the migrator DSN: a CI/CD stage, a one-shot job, or the
Helm chart's `migrations.job.enabled` pre-install/pre-upgrade hook Job, which
Helm waits on so a failed migration fails the release (it takes its own
`migrations.job.existingSecret` holding the migrator DSN, deliberately a
different credential from the runtime one, and rendering fails without it):

```shell
FERROEHR__DATABASE__URL='postgres://ferroehr_migrator:***@pg:5432/ferroehr' \
  ferroehr db migrate     # applies; exits when done
FERROEHR__DATABASE__URL='postgres://ferroehr_clinical:***@pg:5432/ferroehr' \
FERROEHR__DATABASE__MIGRATE_URL='postgres://ferroehr_migrator:***@pg:5432/ferroehr' \
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
> {"state":"down","phase":"serving","indicators":[{"name":"db","state":"up"},
>  {"name":"migrations","state":"down"}]}
> ```
>
> on `/health/readiness` (`503`), with the detail `core schema tables missing
> (migrations not applied)` on `/management/health`, leaves the load balancer's rotation, and keeps
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

### Planner statistics of the partitioned tables

`node`, `version` and `vo_attestation` are partitioned by storage tier in both
stores, and PostgreSQL estimates a join over them from the statistics of the
partitioned table itself. Autovacuum analyzes the partitions and never the
partitioned table (PostgreSQL 18, §24.1.3 Updating Planner Statistics), so
FerroEHR runs `ANALYZE ONLY` on the partitioned tables itself, and on `ehr`
while autovacuum has never analyzed it:

- after it applies the migrations, as the migration credential;
- after `load_ehrs` loads an archive;
- at boot and then every hour, as each store's serving role.

Without these statistics an AQL `CONTAINS` can read every COMPOSITION's
nodes. `ANALYZE` needs the `MAINTAIN` privilege on the table, and the
migrations grant it to `ferroehr_clinical` on `clinical.node`,
`clinical.version`, `clinical.vo_attestation` and `clinical.ehr`, and to
`ferroehr_party` on the three partitioned tables of `party`. When a serving
role still lacks it, for example because the roles were absent when the
migrations ran, the hourly refresh logs a warning naming the tables it could
not analyze. Grant the role `MAINTAIN` on those tables, or run the same
`ANALYZE ONLY` from a scheduled job as the migration role.

### Recovering a partially wiped database

A wipe that removes *some* of the server's schemas is not a fresh start, and the
server cannot tell it from one.

The server owns four schemas: `ext`, `clinical`, `party` and `audit`.
A fresh start drops all four. Each domain's cold archival tier is a **partition**
of the relation it archives, inside that domain's own schema, so a
`DROP SCHEMA clinical CASCADE` takes the archived rows with the live ones. There
is no separate mirror schema left to survive a wipe of the tier it mirrors.

Dropping some of the schemas is not a supported reset, because they are not
independent. `ext` holds the helper functions the `clinical` and `party`
relations depend on, so `DROP SCHEMA ext CASCADE` drops them with it, and those
sets, still recorded as applied, never recreate them. The next boot rebuilds
the dropped schemas, and its
migration check reads each set's bookkeeping only, so it passes over a database
that has lost those guarantees.

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
> four schemas of a database in one transaction, for taking an instance out of
> service ([Decommissioning](operations-decommissioning.md)).

## Backup and point-in-time recovery

Enable WAL archiving and point-in-time recovery from day one (pgBackRest or a
managed PITR), because a CDR's data is not reconstructible. Clinical and audit
tables are never `UNLOGGED`. Test your restore, not just your backup.

### Dump each domain separately

A logical backup is where the two pseudonymisation domains are easiest to
re-join by accident. One `pg_dump` of the whole database produces a single file
holding the pseudonymised clinical record beside the identities of its
subjects, and whoever can read that file can re-identify every record in it,
which is the separation the schema split and the database roles exist to
maintain. **Two domains, two dumps**, into targets with different access
control.

**How many dumps, by DSN layout.** The number of dumps follows the domains, not
the databases: co-located, the three domains share one database and are dumped
`--schema` by `--schema` as below; relocated, each database is dumped for the
domains that live in it. What never changes is that the clinical record and the
identities land in two different artefacts with two different audiences. `ferroehr config check` prints the resolved layout, which is the
list a backup runbook is written from. The audit repository travels with the
clinical dump while it shares that database (`--schema=audit` below) and needs a
dump of its own once `[database.audit]` names another one. On Kubernetes that
third dump is not yours to schedule: the chart renders a `backup.audit`
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

# The audit trail, ONLY when [database.audit] names a database of its own —
# co-located it is in the clinical dump above, and dumping it twice is a second
# copy of who read what
pg_dump --dbname="$AUDIT_DSN" --format=custom --no-owner \
  --schema=audit \
  --file=/backups/audit/audit-$(date -u +%Y%m%dT%H%M%SZ).dump
```

Which EHR belongs to which subject is the EHR's own `EHR_STATUS.subject`, so
it travels in the clinical dump.

The clinical and party examples ship. In Compose they are the opt-in `backup`
profile:

```bash
docker compose --profile backup run --rm ferroehr-backup-clinical
docker compose --profile backup run --rm ferroehr-backup-party
```

Set `FERROEHR_BACKUP_CLINICAL_DIR` and `FERROEHR_BACKUP_PARTY_DIR` to the two
targets; they default to `./backups/clinical` and `./backups/party`.
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
enforce them: the two targets carry **different** access control, and the
credential each job uses reaches **one** domain. Give the demographic job the demographic
DSN once you run two ([Deploying](installation/kubernetes.md)); with a single
credential you have separated the artefacts but not the authority to produce
them.

> [!NOTE]
> Point-in-time recovery stays instance-wide. All the domains live in one
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
# then, before anything serves traffic:
ferroehr db verify
```

Restore the clinical dump first. It is the one carrying the `ext` schema, whose
helper functions the other domains' relations depend on.

`ferroehr db verify` issues no DDL. It checks that the database carries exactly
this build's migrations, **all four sets**, so a restore that skipped a domain
is refused by name, and that no runtime role can read across the domain
boundary, exiting non-zero when either is untrue. It is the same check the
server runs at boot, which is why a server pointed at a badly restored database
refuses to start rather than serving from it.

What it does **not** read is a constraint. `pg_restore` ignores a failed
statement unless you pass `--exit-on-error`, so a table can come back with its
rows and without a key while every check above stays green. Read the restore's
own output.

Restoring only one domain is a supported outcome, not a mistake: a demographic
dump restored on its own gives a database with identities and no clinical
record, which is what a rehearsal of the identity domain's recovery looks like.
Such a database is not one the server will serve from — `ferroehr db verify`
refuses it for the domains that are missing, which is the correct answer for a
rehearsal target.
