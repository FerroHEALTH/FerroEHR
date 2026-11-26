# Storage sizing

How much disk a FerroEHR database takes, how the figures were measured, and how to measure
your own.

<!-- toc -->

The database grows with two things: the composition versions you commit and
the requests the access log records. Size a deployment from the figures below,
then check it against your own data with the query at the end of this section.
No openEHR specification governs storage layout or sizing: these are
measurements of our own design.

## What the figures were measured on

The run of 2026-11-26, on the quickstart (`docker-compose.yml` with
`FERROEHR_EHR_INDEX=local` and the usage report off), with the images pinned by
digest:

| Image | Digest |
|---|---|
| `ghcr.io/ferrohealth/ferroehr` (built from commit `31f0f293cb`) | `sha256:3ec481157f904e622403d23404656211187f7d60586f639411ffa089f582ef06` |
| `ghcr.io/ferrohealth/ferroehr-postgres` (PostgreSQL 19.0) | `sha256:2fc22cc02891a84b82eaebdc858fd3eeaf8e4915e6f7e41a0361f69f5cc70099` |

PostgreSQL ran with `shared_buffers=1GB`, `max_wal_size=4GB`,
`wal_compression=lz4` and default autovacuum, in a Podman VM with 8 CPUs and
16 gibibytes of memory. `scripts/perf/storage-sizing.sh` drove each workload
over ITS-REST with 8 workers, on a fresh database each time, and read the size
of every relation and the WAL position after each phase:

| Phase | Vital signs | International Patient Summary |
|---|---|---|
| EHRs created | 300 | 100 |
| Compositions committed | 3 000 (10 per EHR) | 500 (5 per EHR) |
| Updates (`PUT` with `If-Match`) | 750 | 125 |
| Deletes | 300 | 50 |
| Composition reads | 6 000 | 1 000 |

The compositions are the vendored CKM examples: vital signs is 10 769 bytes of
canonical JSON without whitespace and 43 RM nodes, the International Patient
Summary 87 133 bytes and 395 RM nodes.

## Measuring your own figures

`scripts/perf/storage-sizing.sh` in the repository runs the same phases
against a running FerroEHR and reports the bytes per create, update, delete
and read, per relation, and the node rows with their heap and index bytes per
row. Start the server on a fresh database, so the run is all the database
holds, then:

```sh
scripts/perf/storage-sizing.sh run sizing-out
```

It reads the sizes through `psql` as the database superuser, by default in the
quickstart's `ferroehr-postgres` container (`SIZING_PSQL` names another
command), and writes `sizing-out/report.json`.
`SIZING_TEMPLATE=international-patient-summary` measures the larger
composition, and `SIZING_EHRS`, `SIZING_PER_EHR`, `SIZING_UPDATES`,
`SIZING_DELETES` and `SIZING_READS` set the counts; the script's header lists
every setting. With the defaults (300 EHRs, 3 000 vital signs creates, 750
updates, 300 deletes and 6 000 reads) it reproduces the figures below within
a few percent. Its WAL figures include the full-page images each phase's
`CHECKPOINT` forces, so they are higher than a steady state.

## Bytes per composition version

| Per committed version | Vital signs (10.8 KB of JSON) | International Patient Summary (87 KB of JSON) |
|---|---|---|
| Node rows | 43 rows of 190 B heap and 65 B of index, 11.0 KB | 395 rows of 174 B heap and 66 B of index, 94.6 KB |
| `version` row, with the body compressed (lz4) | 2.4 KB, in the row | 11.4 KB, in TOAST |
| `contribution`, `vo_head`, `commit_audit_facts` and `audit_reservation` | 0.6 KB | 1.2 KB |
| **Clinical storage per create** | **14.0 KB (1.3 × the JSON)** | **107 KB (1.2 × the JSON)** |
| An update that commits the same content again | 2.8 KB | 13.2 KB |
| A delete (a version with no body and no nodes) | 0.9 KB | 3.1 KB |
| WAL written per create | 25 KB | 177 KB |

The node rows hold 79 % of a vital signs version's clinical bytes and 88 % of
an International Patient Summary's, so the size of a version follows the
number of RM nodes in the composition more closely than the length of its
JSON. The driver's updates commit the example body again: such an update adds
the version row and its change-control rows and no node rows. A new EHR takes
about 10 KB, its `EHR_STATUS` version and its access-log entry included.

## Bytes per audit record

Every audited request writes an entry of the openEHR access log
(`audit.access_log_entry`, with its search index `audit.access_log_index`),
reads included. An entry takes about 1.5 KB for a composition read or create
of vital signs, and 1.3 to 1.9 KB for an International Patient Summary. For a
read-heavy workload the access log can outgrow the clinical data: 6 000
composition reads added about as much as 630 new vital signs versions.

## A sizing estimate

Add up, for the period you size for:

- the composition versions you expect, times the bytes per version of your
  own templates (14 KB for one like vital signs, about 0.3 KB per RM node as
  a first estimate);
- the audited requests, times about 1.5 KB, for as long as the
  [audit retention](ehds-logging.md#retention-and-who-chooses-it) keeps them;
- `pg_wal`, up to `max_wal_size` (below);
- your backups and WAL archive, which this database does not hold.

One million vital-signs-sized versions take about 14 GB, and ten million
audited requests about 15 GB.

## Audit retention

An access policy with no `log_retention`, the default, keeps every audit
record forever, so the access log grows with every request for the life of
the database. A period (`ACCESS_POLICY.log_retention.period`, in years or
days) makes the reaper delete each record on or after its end date, hourly,
and the `audit_event` table then levels off at roughly the period times the
request rate. The space of a deleted row is not returned to the operating system:
`VACUUM` marks it free and later records reuse it (PostgreSQL 19 docs,
[Routine Vacuuming, Recovering Disk Space][pg-vacuum]).
The reaper also deletes the search tokens no remaining record names. The
period is the controller's legal choice, within the floors and ceilings it
declares, set out on the audit page.
The openEHR access log levels off too: each entry is kept for its own period,
its tombstone as long again, and then a run of tombstones folds into one gap
row ([steady-state size](ehds-logging.md#the-access-log-at-steady-state)).

## `pg_wal`

PostgreSQL keeps the write-ahead log of the last checkpoints in `pg_wal`, up
to about `max_wal_size` (4 GB in the quickstart), on the same volume as the
data unless you move it. `max_wal_size` is a soft limit: a failing
`archive_command`, `wal_keep_size` or a replication slot whose standby falls
behind holds more (PostgreSQL 19 docs,
[WAL Configuration](https://www.postgresql.org/docs/19/wal-configuration.html)).
Budget it as a fixed amount beside the database, not as growth. A commit
writes about 1.8 times its stored size to the WAL (25 KB for a 14.0 KB vital
signs version), which is what a WAL archive for point-in-time recovery receives.

Set `wal_compression = lz4`. The first change to a page after a checkpoint
writes the whole page to the WAL, and lz4 compresses those images at a CPU
cost far below the flush it saves. Since PostgreSQL 19, `on` picks
the first of zstd, lz4 and pglz the server was built with, where it always
meant pglz before (PostgreSQL 19 docs,
[WAL Configuration](https://www.postgresql.org/docs/19/runtime-config-wal.html#GUC-WAL-COMPRESSION));
naming `lz4` keeps the measured method. The quickstart and the capacity
harness both run with it.

## Vacuum and fillfactor

The clinical write path is insert-only: a composition update or delete adds a
version and never updates or deletes an existing `version` or node row, so the
clinical tables accumulate no dead tuples. Changing autovacuum or fillfactor settings does
not change the volume. Autovacuum still has to run, for the planner
statistics and to freeze old rows before transaction ID wraparound.

## Index maintenance

The node table's primary key, `clinical.node_hot_pkey`, takes inserts from
concurrent commits at many places in the index, so its pages split and stay
partly empty, and a rebuild packs its leaves again. Rebuild it after heavy
write periods, without blocking reads or writes:

```sql
-- leaf density before; pgstattuple ships with PostgreSQL
CREATE EXTENSION IF NOT EXISTS pgstattuple;
SELECT avg_leaf_density FROM pgstatindex('clinical.node_hot_pkey');

REINDEX INDEX CONCURRENTLY clinical.node_hot_pkey;
```

Run it as the database owner or a role holding `MAINTAIN` on
`clinical.node_hot`. `REINDEX CONCURRENTLY` needs room for a second copy of
the index while it runs, and it waits for every transaction older than
itself to finish, so one long-running query or an idle open transaction holds
it up (PostgreSQL 19 docs,
[REINDEX, Rebuilding Indexes Concurrently][pg-reindex]).


## Reading the size per relation

```sql
SELECT n.nspname || '.' || c.relname                           AS relation,
       pg_size_pretty(pg_relation_size(c.oid))                  AS heap,
       pg_size_pretty(COALESCE(pg_total_relation_size(NULLIF(c.reltoastrelid, 0)), 0)) AS toast,
       pg_size_pretty(pg_indexes_size(c.oid))                   AS indexes,
       pg_size_pretty(pg_total_relation_size(c.oid))            AS total
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE c.relkind = 'r'
  AND n.nspname IN ('clinical', 'party', 'audit', 'ext')
ORDER BY pg_total_relation_size(c.oid) DESC
LIMIT 15;
```

Divide the growth of `clinical.node_hot` and `clinical.version_hot` between
two readings by the versions committed in between
(`SELECT count(*) FROM clinical.version_hot`) to get your own bytes per
version. The functions are described in PostgreSQL 19 docs,
[Database Object Management Functions][pg-dbobj].

[pg-vacuum]: https://www.postgresql.org/docs/19/routine-vacuuming.html#VACUUM-FOR-SPACE-RECOVERY
[pg-reindex]: https://www.postgresql.org/docs/19/sql-reindex.html#SQL-REINDEX-CONCURRENTLY
[pg-dbobj]: https://www.postgresql.org/docs/19/functions-admin.html#FUNCTIONS-ADMIN-DBOBJECT
