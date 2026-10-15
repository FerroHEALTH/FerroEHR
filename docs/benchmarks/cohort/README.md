# Cohort-query benchmark record

`app/ferroehr/tests/it/cohort_bench.rs` — an `#[ignore]`d test, never an
acceptance gate — writes one record at `docs/benchmarks/cohort/record.json`.
It measures the whole cohort call, predicate to result set: how the demographic
predicate and the clinical statement behave as the cohort grows from 100 EHRs to
the corpus size, with the `EXPLAIN (ANALYZE)` time of both statements beside the
wall clock.

**A benchmark, not a conformance record.** Conformance is the CNF 2.0 suite
[Veredictum](https://github.com/Vernum-Projecten/Veredictum) runs, and its artifacts
live under `docs/conformance/<sut>/`. Nothing here earns a class. No openEHR
specification governs the cohort surface: this is our own design.

## Running it

```bash
COHORT_BENCH_N=100000 cargo nextest run -p ferroehr \
    -E 'test(cohort_bench)' --run-ignored all --no-capture
```

The database comes from the shared testkit harness (`testkit::db()`). A run
replaces the record rather than appending to it, and the record names the commit
it was measured at.

## What reads it

`scripts/render/cohort-bench.sh` renders the table the book's AQL page includes
(`website/book/generated/cohort-bench.md`, render output and gitignored), so the
published numbers come from this file and are never typed by hand.
