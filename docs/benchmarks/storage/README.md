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

`.github/workflows/storage-bench.yml` runs the benchmark by dispatch on one
GitHub-hosted runner for two builds, a baseline and a candidate, and judges
the candidate against the baseline measured there (#4318):

1. It builds the bench of both revisions. The candidate is the dispatched ref
   (or the `candidate` input), and the baseline is the candidate's merge base
   with `main` (or the `baseline` input). Dispatched on `main`, both are the
   same commit and the run measures the lane's own noise.
2. It runs the two builds in turn, `rounds` times each (8 by default, 4 to 8),
   alternating the order (baseline then candidate, candidate then baseline),
   so a runner that slows down part way through slows both alike. Each round
   seeds a fresh database and runs `ANALYZE` before the first group, so every
   round plans its statements over the same statistics. A clone starts with
   none, and whether autovacuum's first `ANALYZE` landed before or during a
   group used to decide that group's plans: the same build read `supersede`
   at 9 ms in one round and 14 ms in the next, with ten times the buffer hits
   (#4340). Eight rounds of the `poc` class take about four hours; dispatch
   the `s` class with fewer, so the job stays inside its 360 minutes.
3. `scripts/render/storage-bench-compare.sh --ab <dir>` reads two verdicts
   from the rounds, and either one fails the run.

**The plan verdict** reads pages per call: the operation's
`database.blks_hit_delta` plus `blks_read_delta`, over its `iterations`. A
plan reads the same pages on a fast runner and a slow one, so this verdict
does not depend on the machine. An operation fails when the candidate's median
pages per call is more than the page tolerance (10 % by default) and at least
one page above the **highest** baseline round, whatever its time. The bound is
the highest round because the counters are the whole database's, so a round
can carry work beside the operation, and a plan the baseline itself reached
in one round is no new plan. The two A/A runs of 2026-10-30 (41105458534 and
41105463959, both directions) set the bound:

- Read against the other build's median, the candidate's median moved up to
  +360 % (`wide_projection_one_ehr`, where three of eight rounds read about
  10 000 pages per call, one 4 700 and four 1 200 to 1 900, in both builds) and
  +58 % (`retention_prune`, 1.3 against 2.1 pages).
- Read against the other build's highest round, no median was more than
  4.0 % above it (`wide_projection_population`), and none of the operations
  that read about two pages moved by a whole page.
- A plan change that this lane has met reads many times the pages: the node
  foreign-key check read 2 000 or 17 000 to 19 000 hits per supersession
  (#4340), and `revision_history` 16 or about 440 (#4438).

10 % sits above the noise of both A/A runs by a factor of two and a half and
far below any plan change seen so far.

**The time verdict** reads the p50 and p99 of each round, divided first by
the round's machine factor: the median, over the operations, of the round's
p50 against that operation's median p50 across every round of both builds. A
factor of 1.22 says the whole runner ran 22 % slow in that round; the
summary lists every round with its factor and flags one beyond the tolerance
as slower or faster across the board. The tolerance (15 % by default) then
means two things:

- An operation regresses when the median of its adjusted p50 or p99 rose
  more than the tolerance above the baseline's AND every candidate round is
  slower than every baseline round at that percentile. A rise beyond the
  tolerance whose rounds overlap the baseline's reads as within the noise of
  the rounds: the run cannot separate it, so dispatch it again.
- The candidate build regresses as a whole when the median of its rounds'
  factors is more than the tolerance above the baseline's AND every
  candidate round's factor is above every baseline round's. Dividing by the
  factor cannot tell a slow machine from code that slows every operation, and
  interleaving can: a slow spell of the runner falls on rounds of both builds,
  while slower code falls on every candidate round and no baseline round.

The summary shows the raw change beside the adjusted one. With eight rounds a
build, an unchanged operation's rounds separate by chance once in
C(16, 8) = 12 870 orders, against once in 70 with four. Run 41105458534 shows
why the factor is there: with four rounds a build, one candidate round ran
22 % slow across the board (23 of 29 operations above their median), and the
raw medians read 18 operations 15 to 27 % slower, two of them with the rounds
separated, so unchanged code failed. Divided by the factors, one operation
rose beyond 15 % (`latest_by_versioned_object_uid`, +20.7 % at p50) with its
rounds overlapping the baseline's, and the candidate build
ran 6.4 % slower across the board, inside the spread of the rounds' factors.
The pages per call of the same rounds stayed within the bound above.

The candidate's storage sizes are judged against the record committed for its
generation and class, as before, because they do not depend on the machine.

A latency is never compared across two runners: two hosted runs of the same
bench code on 2026-10-28 read 20 of 27 operations 40 to 156 % apart, so a
committed record is the source of the size facts and of the book's numbers,
never of a latency verdict. A generation and class with no committed record
fails the run, with the error in the job summary: a schema-generation change
once left the lane comparing nothing and passing (#4291), so the change that
moves `ext.storage_generation()` commits the new generation's records with it.

The single-record comparison runs locally too, against the record committed at
`HEAD`:

```bash
bash scripts/render/storage-bench-compare.sh --tolerance 15
```

It reads the baseline from `HEAD`, because the run just overwrote the
working-tree copy, and judges nothing when the two records carry different
`bench_class` values. `--baseline <file>` compares against any other record,
which is how a new generation is set against the one it replaces. Like the
lane's own judgement, a local verdict is sound only for two records measured
on the same machine.

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
| `person_identifier_lookup` | a lookup of one person by an identifier value, `WHERE id/items[at0001]/value/id = $v` over the party store's `person_identifier` CLUSTERs (#4437) |
| `ehr_by_subject` | a lookup of one EHR by the subject of its EHR_STATUS, `WHERE e/ehr_status/subject/external_ref/id/value = $u AND e/ehr_status/subject/external_ref/namespace = $n` (#4442) |
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
