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

FerroEHR 5.6.0-rc5 from the quickstart (`docker-compose.yml` with
`FERROEHR_EHR_INDEX=local` and the usage report off), PostgreSQL 18.6 with
`shared_buffers=1GB`, `max_wal_size=4GB`, `wal_compression=lz4` and default
autovacuum, in a Podman VM with 8 CPUs and 16 gibibytes of memory. A client with 8 workers ran
each workload over ITS-REST in five phases, and the size of every relation
(`pg_relation_size`, the TOAST relation's `pg_total_relation_size` and
`pg_indexes_size`) and the WAL position (`pg_current_wal_lsn()`) were read
after each phase:

1. create 1 000 EHRs;
2. commit 20 000 compositions, 20 per EHR;
3. update 5 000 of them (`PUT` with `If-Match`);
4. delete 2 000 versions;
5. read 40 000 compositions.

The compositions are the vendored CKM examples for the vital signs template
(10 769 bytes of canonical JSON without whitespace, 43 RM nodes) and, in a
smaller run of 200 EHRs and 2 000 compositions, the International Patient
Summary (87 133 bytes, 395 RM nodes). The vital signs workload ran twice: once
on fresh volumes and once appending to the same database. The two runs grew
the database by 608.0 MiB and 607.8 MiB, so the growth is linear in the work
and does not depend on how much the database already holds.

The node rows, the clinical totals and the WAL below were measured again after
a node row below the root started to keep its archetype header, language and
encoding in columns: the same driver, PostgreSQL image and settings, against a
server built from the source, with 500 EHRs and 5 000 vital signs
compositions and 100 EHRs and 500 International Patient Summaries. That change
took a vital signs version from 21.6 KB to 17.1 KB and an International
Patient Summary from 159 KB to 131 KB. They were measured once more, with
the same driver and settings (300 EHRs, 3 000 vital signs compositions, 750
updates, 300 deletes and 6 000 reads; 100 EHRs and 500 International Patient
Summaries), after storage generation 5 let a node row store a plain
`DV_DATE_TIME` origin or time as its value string, leave out a bare
`PARTY_SELF` subject, and leave the root's uid and archetype header with the
version row: a vital signs version went from 17.1 KB to 15.5 KB and an
International Patient Summary from 131 KB to 128 KB.

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
| Node rows | 43 rows of 201 B heap and 83 B of index, 12.2 KB | 395 rows of 199 B heap and 92 B of index, 115 KB |
| `version` row, with the body compressed (lz4) | 2.5 KB, in the row | 10.9 KB, in TOAST |
| `commit_audit`, `contribution` and `vo_head` | 0.7 KB | 0.7 KB |
| **Clinical storage per create or update** | **14.6 to 15.5 KB (1.4 × the JSON)** | **128 to 138 KB (1.5 to 1.6 × the JSON)** |
| A delete (a version with no body and no nodes) | 1.4 KB | 2.5 KB |
| WAL written per create | 30 KB | 242 KB |

The node rows hold about 80 % of the clinical bytes, so the size of a version
follows the number of RM nodes in the composition more closely than the
length of its JSON. An update writes a complete version again (its body and
all its node rows), so it costs the same as a create. A new EHR takes about
8 KB, its `EHR_STATUS` version and the audit record included.

## Bytes per audit record

With `[audit.store]` on (the default) every audited request writes one record,
reads included. A record of a composition read takes about 1.0 KB in
`audit.audit_event` (heap and indexes, no TOAST), and a record of a
composition create about 1.05 KB. The request id and the version uid are
searched through the record's own columns, so a record adds no row to
`audit.audit_search_token` beyond the first record about a patient or a
resource kind. For a read-heavy workload the access log can outgrow the
clinical data: 40 000 composition reads add about 38 MiB, as much as about
1 800 new vital signs versions.

## A sizing estimate

Add up, for the period you size for:

- the composition versions you expect, times the bytes per version of your
  own templates (15 KB for one like vital signs, 0.3 to 0.4 KB per RM node
  as a first estimate);
- the audited requests, times about 1.0 KB, for as long as the
  [audit retention](ehds-logging.md#retention-and-who-chooses-it) keeps them;
- `pg_wal`, up to `max_wal_size` (below);
- your backups and WAL archive, which this database does not hold.

One million vital-signs-sized versions take about 15 GB, and ten million
audited requests about 10 GB.

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
writes about twice its stored size to the WAL (30 KB for a 15.5 KB vital signs
version), which is what a WAL archive for point-in-time recovery receives.

Set `wal_compression = lz4`. The first change to a page after a checkpoint
writes the whole page to the WAL, and lz4 compresses those images at a CPU
cost far below the flush it saves: a composition commit wrote about 52 KB of
WAL without it and about 24 KB with it. Since PostgreSQL 19, `on` picks
the first of zstd, lz4 and pglz the server was built with, where it always
meant pglz before (PostgreSQL 19 docs,
[WAL Configuration](https://www.postgresql.org/docs/19/runtime-config-wal.html#GUC-WAL-COMPRESSION));
naming `lz4` keeps the measured method. The quickstart and the capacity
harness both run with it.

## Vacuum and fillfactor

The clinical write path is insert-only: a composition update or delete adds a
version and never updates or deletes an existing `version` or node row. After
the measured runs `pgstattuple_approx` found no dead tuples in
`clinical.node_hot`, `clinical.version_hot` or `audit.audit_event`, and under
1 % free space in `node_hot`. Changing autovacuum or fillfactor settings does
not change the volume. Autovacuum still has to run, for the planner
statistics and to freeze old rows before transaction ID wraparound.

## Index maintenance

The node table's primary key, `clinical.node_hot_pkey`, takes inserts from
concurrent commits at many places in the index, so its pages split and stay
partly empty. After the measured runs its leaves were 72 % full, and a rebuild
took it from 186 MiB to 149 MiB (20 % smaller) with 90 % full leaves. Rebuild
it after heavy write periods, without blocking reads or writes:

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
The audit indexes on `recorded_at` are ascending and fill their pages without
a rebuild.

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
