# Care caseload record

`app/ferroehr/tests/it/care_caseload_plans.rs` holds the plan shapes of the
care projection at 100, 1,600 and 50,000 patients per caseload and prints the
time of the check of one EHR, a caseload page of 50 and a `COUNT` of the
caseload's compositions, with relationships scoped to no episode and then to
one. `record.json` holds one run of it, with the commit and the host it was
measured on.

**A local measurement, not a load test.** Throughput and tail latency under
load come from the capacity test and the benchmark campaign. No openEHR
specification governs these access paths: our own design.

## Running it

```bash
cargo nextest run -p ferroehr --no-capture -j 1 \
    -E 'test(the_caseload_plans_hold_at_practice_scale)'
```

Copy the printed times into `record.json` with the commit you measured.

## What reads it

`scripts/render/care-caseload-bench.sh` renders the table the book's care
relationships page includes (`website/book/generated/care-caseload-bench.md`,
render output and gitignored), so the published numbers come from this file
and are never typed by hand.
