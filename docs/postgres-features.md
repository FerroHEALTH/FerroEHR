# PostgreSQL 17, 18 and 19 features: what FerroEHR uses, and what it does not

**Pin: PostgreSQL 19, target 19.0+** (`docs/VERSIONS.md`; CI runs `postgres:19.0`;
the boot path refuses an older server, `db::server_version`).
Upstream EHRbase targets **PG 15/16**; we target **19**. This file is the
register of the PG 17, PG 18 and PG 19 features that matter to a JSONB-heavy
openEHR CDR, and for each one it says whether the code USES it, where, or why
it is deliberately NOT used. A feature listed here is not a claim that the code
uses it; the "status" column is. The PG 17 and PG 18 reasons cite the
PostgreSQL 18 documentation (`https://www.postgresql.org/docs/18/`), where they
still hold on 19 (#6224 re-cites the ones 19 changed); the PG 19 rows cite the
19 release notes (`https://www.postgresql.org/docs/19/release-19.html`).

## Versioning note (why the feature list is 17.0, 18.0 and 19.0)

PostgreSQL adds features **only in major releases**. Every minor release is a
cumulative **bugfix + security** rollup with **no new SQL features**, and a
same-major upgrade needs no dump/restore. So the feature delta over EHRbase's
PG 16 is exactly the **PG 17.0**, **PG 18.0** and **PG 19.0** feature sets
below; we run the latest patch (19.0, 2026-11-19) for the fixes. PostgreSQL 18
is no longer supported, and a major upgrade is a dump and restore.

## PG 17.0 — SQL/JSON + query performance

| Feature | Status | Where, or why not |
|---|---|---|
| **SQL/JSON path functions** — `jsonb_path_query_first`, `jsonb_path_query` (PG 12+, extended in 17) | **used** | the AQL emitter extracts leaves with `jsonb_path_query_first` and unnests arrays with `jsonb_path_query` as a lateral set-returning function (`app/ferroehr/src/aql/sql/`). |
| **`JSON_TABLE()`** | not used | the lateral `jsonb_path_query` is the function form of the same operation; the emitter emits no `JSON_TABLE`. A switch would be a measured emitter change, not a documentation fact. |
| **SQL/JSON query fns** — `JSON_EXISTS`, `JSON_QUERY`, `JSON_VALUE` | not used | `jsonb_path_query_first` and `#>> '{}'` serve extraction; `JSON_VALUE(... RETURNING type)` would give typed extraction with `ON ERROR` control and is a candidate for the emitter, nothing more. |
| **SQL/JSON constructors** — `JSON()`, `JSON_SCALAR()`, `JSON_SERIALIZE()` | not used | nothing builds JSON in-query; the canonical body is stored verbatim as `text`. |
| **`jsonpath` item methods** — `.integer()/.boolean()/.date()/.datetime()` | not used, deliberately | the date/time methods depend on the session TimeZone, so they exist only in the `_tz` function variants, which are STABLE and cannot back an index (`functions-json.html`); comparison goes through `ext.openehr_timestamp` (STABLE, floors partial precision) and `ext.openehr_magnitude` (IMMUTABLE). |
| **`MERGE … RETURNING` + `merge_action()`**, **`MERGE … WHEN NOT MATCHED BY SOURCE`** | not used, deliberately | `INSERT … ON CONFLICT` is the concurrency-safe upsert the commit path uses for `contribution` and `ehr`; the docs call MERGE and `ON CONFLICT` "not interchangeable" under concurrency (`sql-merge.html`). |
| **Optimizer: `IN`/`NOT IN`, correlated subqueries, B-tree `IN` batches** | planner-side | applies to every generated statement without code. |
| **Incremental backup** (`pg_basebackup --incremental`) | operator's | not app code. |

## PG 18.0 — async I/O, temporal, identifiers, generated columns, auth

| Feature | Status | Where, or why not |
|---|---|---|
| **`uuidv7()` (native)** | **used** for database-minted ids | `commit_audit.id`, `contribution.id` (fallback), `vo_attestation.id`, `item_tag.id`. Versioned-object and EHR ids are minted in Rust as v7 with the licence stamp, so `(vo_id, sys_version)` keys are NOT append-ordered. |
| **Temporal `PRIMARY KEY`/`UNIQUE` `WITHOUT OVERLAPS`** | not used | the one relation that carried it, `linkage.subject_ehr`, is gone, and with it the need for `btree_gist`. NOT used on `version`, which carries no interval at all: the store is append-only and validity is derived from `committed_at`. |
| **Temporal `FOREIGN KEY`** | not used | NO ACTION only, and the pseudonymisation boundary refuses cross-domain foreign keys anyway. |
| **`RETURNING OLD/NEW`** | not used | the commit path is one CTE chain with plain `RETURNING id`; nothing reads `old.`/`new.`. |
| **Virtual generated columns** | not usable here | a virtual column "must not reference user-defined functions or types" (`ddl-generated-columns.html`), so it cannot call `ext.*`; a STORED one may call IMMUTABLE `openehr_magnitude` but never the STABLE timestamp parser. The promoted `node` columns are populated by the decomposer at write time. |
| **B-tree skip scan** | planner-side, relied on once | `item_tag` lookups by key within one EHR over the identity index; planner-conditional on few distinct leading values (`indexes-multicolumn.html`), never a guarantee. |
| **Asynchronous I/O (AIO)** (`io_method`; on 19 `io_min_workers` and `io_max_workers` scale the workers, `io_workers` is gone) | server default | nothing in compose or Helm tunes it; the default `worker` method applies. Helps sequential and bitmap scans and VACUUM, not the index nested loops AQL emits. |
| **OAuth authentication** (`pg_hba.conf`) | not used | app-level OAuth2/OIDC is the auth layer. |
| **Self-join elimination** | planner-side | applies to the emitter's `node` self-joins without code. |
| **`OR` → `= ANY(array)`** | emitter-side | the emitter writes `= ANY($1)` itself for `MATCHES` lists; the planner transformation is not relied on. |
| **`jsonb` null → SQL `NULL` cast** | not relied on | no emitter site depends on it. |
| **Partition planner improvements** | **used** | `version`, `node` and `vo_attestation` are `PARTITION BY LIST (tier)` in both change-control domains. AQL writes `tier = 'hot'` as a literal so the cold partition is pruned at plan time; an `UPDATE` of the partition key is how archival moves rows; `vo_attestation` follows through `ON UPDATE CASCADE` (verified first-hand on 18.6 — the documentation neither permits nor forbids it), and `node`, which has no foreign key (#5969), is moved by name in the same statement. |

## PG 19.0 — WAL, TOAST, vacuum, planner

| Feature | Status | Where, or why not |
|---|---|---|
| **`wal_compression = on` picks the best method** (lz4 or zstd, was pglz) | recipe, measured first | `docker-compose.yml` and `deploy/hosted/cloud-init-postgres.yaml` set `lz4`; the Helm chart sets none. Compared on the quiet host under #6206 before a recipe changes. |
| **Full-page-image bytes in `pg_stat_wal`, `pg_stat_get_backend_wal()` and `EXPLAIN (ANALYZE, WAL)`** | instrument, planned | a full-page-image ceiling beside the WAL ceilings of the statement budget harness, and the capacity report's WAL split (#6206). |
| **`default_toast_compression` lz4** (was pglz) | server default | applies to every compressible column the migrations create, the `STORAGE MAIN` audit columns included; lz4 against pglz is measured in #6219. |
| **Table scans mark pages all-visible**; **parallel autovacuum of indexes**; **autovacuum scoring** | server default, measured first | helps index-only scans over the append-only `version`, `node` and audit tables before vacuum reaches them; `autovacuum_max_parallel_workers` is left at its default until #6220 measures it. |
| **Optimizer: `NOT IN` to anti join without NULLs, more left joins to anti joins, eager aggregation, semijoin planning; faster foreign-key checks; `NOTIFY` wakes only listeners** | planner-side | applies without code: the access filter's `NOT IN` (`aql/sql/access.rs`), the commit chain's foreign keys, the cache channels. Measured against 18.6 in #6218. |
| **JIT off by default** | relied on | the emitter already keeps estimates from switching JIT on (#4430, #4431); on 19 no OLTP statement compiles unless a deployment enables JIT. |
| **WAL write and flush wait events, `commit_delay` wait event** | instrument, planned | breaks down the commit flush, the measured limit, and makes group commit measurable (#6221). |
| **`INSERT ... ON CONFLICT DO SELECT ... RETURNING`** | **used** | the settings class of the `EHR_ACCESS` projection (`storage/access_projection.rs`), the audit search tokens and shared sets (`system_log/search`) and the usage report's installation id (`usage_report/store.rs`) are found or stored in one statement that also returns a row a concurrent writer committed (#6222). Not used where the conflict answer needs no stored row: the EHR create, the template upload and the stored query, each a `409` or a `false`. |
| **`WAIT FOR LSN` on standbys** | not used | read-your-writes replica reads are a design question under the exact-reads rule (#6223). |
| **`REPACK (CONCURRENTLY)`** | operator's | rebuilds a bloated table without an `ACCESS EXCLUSIVE` lock; not app code. |
| **`max_locks_per_transaction` default 128, with halved per-unit capacity** | neutral | no recipe sets it, so the partitioned relations keep the same lock capacity. |

## Feature → subsystem mapping (what the code actually does)

- **Persistence / service layer:** `uuidv7()` for database-minted ids; one
  CTE chain per Contribution with `INSERT … ON CONFLICT` for the idempotent
  rows and the head-row upsert; heap-only updates on `vo_head`; list
  partitioning by tier on the change-control relations.
- **AQL engine:** `jsonb_path_query_first`, lateral `jsonb_path_query`,
  `ext.openehr_magnitude`, `ext.openehr_timestamp`, integer nested-set joins,
  promoted btree columns, `= ANY` lists (`.claude/rules/aql-engine.md`), and
  four partial btree indexes a filter enters through: the search text of an
  ELEMENT value, its code, the time of an event or the origin of a history,
  and the magnitude of an ELEMENT value, all but the time over an `IMMUTABLE`
  SQL function (`indexes-partial.html`, `indexes-expressional.html`). Each
  helper is in the language and shape that measured fastest over the values it
  sees; five reach every cast through a guard rather than an `EXCEPTION` block,
  which the docs call "significantly more expensive to enter and exit than a
  block without one" (`plpgsql-control-structures.html`), and the two date/time
  parsers keep one because the trapped cast measured cheaper than any guard
  replacing it. A read either shape refuses is NULL, so a stored value can never
  make a query error.
- **Auth:** app-level OAuth2/OIDC (crates) is primary; DB `oauth` is unused.
- **Optimization:** a feature that is *only* a performance win is adopted on a
  committed measurement, never on this list; the candidates and their
  instruments are in the storage redesign plan (#3337).

**Discipline:** use PG 19 features where they simplify or speed the SQL, but a
feature that is *only* a perf win (not needed for correctness/conformance) is a
`// TODO(#NNNN):` on its optimization issue — never trade away REST/AQL
conformance for it.
