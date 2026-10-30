# Storage benchmark records

`app/ferroehr/benches/storage.rs` writes one record per run, at
`docs/benchmarks/storage/<schema-generation>/<class>/record.json`. It exists so a
storage change is argued from a before/after pair taken with the same
instrument, rather than from a claim: the harness drives the operations through
`FerroEhrService` and the public `ferroehr::storage` / `ferroehr::versioning`
surface and reads its database-side facts for whatever relations the catalogue
reports, so the same file measures a rewritten schema unchanged.

**A benchmark, not a conformance record.** Conformance is the CNF 2.0 suite
[Veredictum](https://github.com/Vernum-Projecten/Veredictum) runs, and its artifacts
live under `docs/conformance/<sut>/` — including the earned performance classes.
Nothing here earns a class, nothing here belongs beside `results.json`, and this
directory sits outside `docs/conformance/` on purpose. No openEHR specification
governs storage layout or benchmarking: this is our own design.

## Running it

```bash
# the default class: a small corpus, minutes
STORAGE_BENCH_CLASS=poc cargo bench -p ferroehr --bench storage

# the larger class
STORAGE_BENCH_CLASS=s cargo bench -p ferroehr --bench storage

# CPU flamegraphs instead of timings (criterion measures nothing in this mode,
# so no record is written and an existing one is left intact)
cargo bench -p ferroehr --bench storage -- --profile-time 10
```

The database comes from the shared testkit harness (`testkit::db()`), so the
run needs the same PostgreSQL 18 container the test suite uses and nothing else.

`.github/workflows/storage-bench.yml` runs the same benchmark on a
GitHub-hosted runner by dispatch and hands the fresh record to
`scripts/render/storage-bench-compare.sh`, which compares it per operation with
the record committed for the same generation and class and fails the run when
a p50 or p99 rose further than the dispatched tolerance. A generation and class
with no committed record fails the run too, with the error in the job summary:
a schema-generation change once left the lane comparing nothing and passing
(#4291), so the change that moves `ext.storage_generation()` commits the new
generation's records with it. That comparison runs locally too:

```bash
bash scripts/render/storage-bench-compare.sh --tolerance 15
```

It reads the baseline from `HEAD`, because the run just overwrote the
working-tree copy, and judges nothing when the two records carry different
`bench_class` values. `--baseline <file>` compares against any other record,
which is how a new generation is set against the one it replaces.

## The shape

```json
{
  "schema_generation": "generation-1",
  "postgres_version": "PostgreSQL 18.6 …",
  "bench_class": "poc",
  "cpu_count": 10,
  "memory_bytes": 34359738368,
  "measured_at_commit": "…",
  "measured_at": "2026-09-14T…Z",
  "timings_measured": true,
  "seed": { "ehrs": 20, "compositions": 300, "template_id": "…",
            "second_template_id": "…",
            "entry_rm_type": "OBSERVATION", "seconds": 0.0 },
  "storage": { "composition_versions": 0, "bytes_per_composition_version": 0.0,
               "node_rows_per_version": 0.0, "node_heap_bytes_per_row": 0.0,
               "node_row_bytes": 0.0, "input_bytes_per_version": 0.0,
               "stored_body_bytes_per_version": 0.0, "audit_records": 0,
               "audit_bytes_per_record": null,
               "growth": [ { "relation": "clinical.…", "bytes": 0 } ] },
  "groups": [
    { "name": "storage_commit",
      "operations": [
        { "name": "create_first_version",
          "iterations": 0, "samples": 10,
          "mean_ns": 0.0, "median_ns": 0.0,
          "p50_ns": 0.0, "p95_ns": 0.0, "p99_ns": 0.0,
          "database": {
            "tables": [ { "relation": "public.…",
                          "n_tup_ins_delta": 0, "n_tup_upd_delta": 0,
                          "n_tup_hot_upd_delta": 0, "n_tup_del_delta": 0,
                          "n_live_tup": 0, "n_dead_tup": 0,
                          "total_bytes": 0 } ],
            "blks_hit_delta": 0, "blks_read_delta": 0,
            "xact_commit_delta": 0,
            "parallel_workers_to_launch_delta": 0,
            "parallel_workers_launched_delta": 0, "wal_bytes": 0 } } ] } ],
  "commit_probe": { "operation": "create_composition", "commits": 1,
                    "database": { "…": "as above" } },
  "explain": { "statement": "SELECT …", "node_type": "…",
               "actual_total_ms": 0.0, "rows": 0,
               "shared_hit_blocks": 0, "shared_read_blocks": 0,
               "wal_bytes": 0, "wal_records": 0, "index_names": [] },
  "relations": [ { "relation": "public.…", "n_live_tup": 0, "n_dead_tup": 0,
                   "n_tup_upd": 0, "n_tup_hot_upd": 0, "total_bytes": 0 } ]
}
```

### Header

`schema_generation` is `ext.storage_generation()` where the schema declares
one, `STORAGE_BENCH_GENERATION` where a schema does not, and `generation-1`
otherwise — it is the directory the record lands in, so two generations of the
same schema never overwrite each other. `postgres_version` is the server's own
`version()`, `measured_at_commit` the working tree's HEAD, and `cpu_count` /
`memory_bytes` the machine. A record is meaningful only with all four: the same
code measures differently on different hardware, which is the same rule the
performance classes carry.

`timings_measured` is true only when every operation in the record carries
samples. A filtered run (`cargo bench … -- storage_read`) measures some of them
and says so.

### Operations

One entry per benched operation, in six groups: `storage_commit` (a first
version, a blind supersession, and the `If-Match` supersession that states the
version it replaces), `storage_read` (by version uid, by versioned-object uid,
`version_at_time`, the revision history), `storage_query` (the CONTAINS chain
over one EHR and over the population), `storage_paths` (two and three sibling
leaves under a repeated EVENT, in the flat and the streaming FROM shape),
`storage_aql` (the AQL hot paths below), and `storage_lifecycle` (archive and
restore of one EHR, and one retention prune).

The seed commits two vendored CKM templates in turn over every EHR: the vital
signs (`template_id`), which the write benches commit too, and a GP data set
(`second_template_id`), whose EVALUATIONs carry the coded and text values the
vital signs lack.

`storage_aql` holds the statements whose cost the node layout decides. They are
built from the seed payload, so each matches what was committed, and each must
return rows before it is timed:

| Operation | What it exercises |
|---|---|
| `contains_exists` | `count(c)` over a CONTAINS chain into an archetyped OBSERVATION: containment as an existence test |
| `node_predicate` | a CONTAINS step with an at-code and a name predicate, for example `ELEMENT x[at0004, 'Systolic']` |
| `where_magnitude` | a WHERE and an ORDER BY on a `DV_QUANTITY` magnitude |
| `where_coded` | a WHERE on a `DV_CODED_TEXT` code |
| `where_date_time` | a WHERE on a `DV_DATE_TIME` value |
| `where_text` | a WHERE on a `DV_TEXT` value |
| `wide_projection_one_ehr` | up to 16 leaf values of one OBSERVATION, every row of one EHR |
| `wide_projection_population` | the same over the population, `LIMIT 50` |
| `select_observation` | `SELECT o`, the whole OBSERVATION reassembled |
| `select_composition_one_ehr` | `SELECT c` over one EHR |
| `aql_version_at_time` | a `VERSION v[commit_audit/time_committed <= …]` scope; named apart from the `storage_read` operation, since the compare step keys operations by name |

Each entry carries its `statement`, the `rows` it returned and whether it ran
over one EHR (`one_ehr`).

`iterations` is how many times the routine ran across every sample;
`mean_ns` / `median_ns` are criterion's own point estimates; the percentiles are
nearest-rank over criterion's per-sample nanoseconds-per-iteration, so a
published figure does not depend on a rounding mode.

### Database facts

`database` is attributed per operation: the counters are read immediately before
and after that benchmark, and only the relations the operation actually touched
appear. The instruments are the ones the PostgreSQL documentation names —
`pg_stat_user_tables` (`n_tup_ins`, `n_tup_upd`, `n_tup_hot_upd`, `n_tup_del`,
`n_live_tup`, `n_dead_tup`), `pg_total_relation_size`, `pg_stat_database`
(`blks_hit`, `blks_read`, and `parallel_workers_to_launch` and
`parallel_workers_launched`, which say whether the operation's plans asked for
parallel workers and how many started, #4429), and the WAL position (`pg_current_wal_lsn`,
`pg_wal_lsn_diff`). A counter read waits out the statistics flush interval
first, because a backend's pending counts reach shared memory when its
transaction ends and at most once a second; a WAL figure an unprivileged role
may not read is absent rather than zero.

`storage` is what the seed's composition commits cost on disk (#4303), read as
the growth across them, so the EHRs and templates created before are not
counted: `composition_versions` committed, `bytes_per_composition_version` (the
growth of every relation in the clinical schema, `pg_total_relation_size`, over
those versions), `node_rows_per_version`, `node_heap_bytes_per_row` (the growth
of the `node` partitions' heap, `pg_relation_size`, over the rows added),
`node_row_bytes` (`pg_column_size` of a whole row, averaged),
`input_bytes_per_version` and `stored_body_bytes_per_version` (the canonical
JSON and its compressed stored form), the audit records written and
`audit_bytes_per_record` (null when the service wrote none, which is the case
when it is driven below the REST layer), and `growth`, each relation's share.

The compare step judges two of them against a budget, 5 % by default
(`--size-budget`, the lane's `size_budget_percent` input): a run whose bytes per
composition version or heap bytes per node row grow more than the budget above
the committed record fails. Sizes do not depend on the machine the way
latencies do, so the budget is tighter than the latency tolerance. The other
facts are reported beside them. A change that is meant to grow storage commits
the new record with it, which is the same procedure as a new generation.

`commit_probe` is one representative commit on its own: the WAL it writes and
the buffers it touches. It is measured around a real `create_composition` rather
than by `EXPLAIN (ANALYZE, WAL)`, because the commit runs inside the service's
own transaction and the public API hands out no statement to explain.

`explain` is `EXPLAIN (ANALYZE, BUFFERS, WAL, FORMAT JSON)` of the population
CONTAINS statement, run inside a transaction that is rolled back, summarised to
the top node's shape, timing, buffers and the index names anywhere in its tree —
a whole plan is hundreds of lines of noise a week later.

`relations` closes the record with every relation the run put rows into: its
size, its live and dead tuple estimates, and its update-versus-HOT-update
counts. That is where the version and node relations' `pg_total_relation_size`
and bloat are read.

## What is committed

Records are gitignored by default (`.gitignore` beside this file), for the same
reason the deployment records under `docs/conformance/deployment/` are: a record
measures one machine on one day, and a directory of them invites comparison
between runs that measured different systems.

The exception is deliberate: each generation's **baseline** is committed, one
per class the lane offers (`poc` and `s`), because the whole point of the
instrument is a before/after pair. A baseline comes from a run of the lane, so
every committed record measured the same kind of machine:

```bash
gh workflow run storage-bench.yml --ref <branch> -f class=poc
gh workflow run storage-bench.yml --ref <branch> -f class=s
# each run's artifact holds <generation>/<class>/record.json
git add -f docs/benchmarks/storage/<generation>/<class>/record.json
```

and say in the commit which run it came from. A tracked record stays
tracked — the ignore rule only keeps the untracked ones out of `git status`.

## Where the numbers are published

Nowhere by hand. `scripts/render/storage-bench.sh` renders the committed records
into `website/book/generated/storage-bench.md`, which
`website/book/src/performance.md` includes; the stale-numbers gate
(`scripts/checks/conformance-numbers.sh`) refuses a hand-typed latency in the
site sources. With no record committed the generated page says so, rather than
carrying a number from somewhere else.
