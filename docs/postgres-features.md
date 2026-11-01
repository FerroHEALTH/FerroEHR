# PostgreSQL 17 + 18 features: what FerroEHR uses, and what it does not

**Pin: PostgreSQL 18, target 18.6+** (`docs/VERSIONS.md`; CI runs `postgres:18.6`).
Upstream EHRbase targets **PG 15/16**; we target **18**. This file is the
register of the PG 17 and PG 18 features that matter to a JSONB-heavy openEHR
CDR, and for each one it says whether the code USES it, where, or why it is
deliberately NOT used. A feature listed here is not a claim that the code
uses it; the "status" column is. Every reason cites the PostgreSQL 18
documentation (`https://www.postgresql.org/docs/18/`).

## Versioning note (why the feature list is only 17.0 + 18.0)

PostgreSQL adds features **only in major releases**. Every minor release —
**18.1 through 18.6** (18.5 was never released) and all of **17.x** — is a
cumulative **bugfix + security** rollup with **no new SQL features** (18.6,
2026-08-13, fixes 28 CVEs; same-major upgrades need no dump/restore). So the
feature delta over EHRbase's PG 16 is exactly the **PG 17.0** and **PG 18.0**
feature sets below; we run the latest patch (18.6) for the fixes.

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
| **Asynchronous I/O (AIO)** (`io_method`, `io_workers`) | server default | nothing in compose or Helm tunes it; the default `worker` method applies. Helps sequential and bitmap scans and VACUUM, not the index nested loops AQL emits. |
| **OAuth authentication** (`pg_hba.conf`) | not used | app-level OAuth2/OIDC is the auth layer. |
| **Self-join elimination** | planner-side | applies to the emitter's `node` self-joins without code. |
| **`OR` → `= ANY(array)`** | emitter-side | the emitter writes `= ANY($1)` itself for `MATCHES` lists; the planner transformation is not relied on. |
| **`jsonb` null → SQL `NULL` cast** | not relied on | no emitter site depends on it. |
| **Partition planner improvements** | **used** | `version`, `node` and `vo_attestation` are `PARTITION BY LIST (tier)` in both change-control domains. AQL writes `tier = 'hot'` as a literal so the cold partition is pruned at plan time; an `UPDATE` of the partition key is how archival moves rows, and the referencing rows follow through `ON UPDATE CASCADE` (verified first-hand on 18.6 — the documentation neither permits nor forbids it). |

## Feature → subsystem mapping (what the code actually does)

- **Persistence / service layer:** `uuidv7()` for database-minted ids; one
  CTE chain per Contribution with `INSERT … ON CONFLICT` for the idempotent
  rows and the head-row upsert; heap-only updates on `vo_head`; list
  partitioning by tier on the change-control relations.
- **AQL engine:** `jsonb_path_query_first`, lateral `jsonb_path_query`,
  `ext.openehr_magnitude`, `ext.openehr_timestamp`, integer nested-set joins,
  promoted btree columns, `= ANY` lists (`.claude/rules/aql-engine.md`). Each
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

**Discipline:** use PG 18 features where they simplify or speed the SQL, but a
feature that is *only* a perf win (not needed for correctness/conformance) is a
`// TODO(#NNNN):` on its optimization issue — never trade away REST/AQL
conformance for it.
