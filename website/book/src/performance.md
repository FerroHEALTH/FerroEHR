# Performance

FerroEHR's performance figures come from the same instrument as its
conformance record: the `perf` command of
[Veredictum](https://github.com/Vernum-Projecten/Veredictum), at the release
pinned in `scripts/lib/veredictum.sh`. A performance record earns no class
and enters no conformance verdict. No openEHR specification governs
performance, so the measurement method is Veredictum's own design, described
in its book under
[Measured performance](https://veredictum.eu/docs/running.html#measured-performance).
Nothing on this page is hand-typed: every number comes from the committed
record or from charts drawn from it.

<!-- toc -->

## What a run measures

`veredictum perf` offers one pinned workload to a running FerroEHR as an
open-loop schedule of Poisson arrivals. Each request's latency counts from the
instant it was due, not from when it was sent, so a server that stalls cannot
hide the queue it builds behind a few fast replies. A run has these phases:

1. **Seeding**, before the clock: the workload's corpus is committed through
   the public write path, closed-loop, and reported apart.
2. **Posture canaries**: a first bracket of probes checks that the deployment
   runs the posture the run declares (below).
3. **Settle**: `vacuumdb --all --analyze` runs in the database container, so
   no plateau pays the seed's maintenance debt.
4. **Warm-up**, discarded.
5. **Scout**: an exponential ramp, judged in short windows, whose first breach
   gives an estimate of the knee.
6. **Ladder**: plateaus at fixed fractions of that estimate, each held long
   enough to judge its tail.
7. **Closing**: the second bracket of posture canaries.

The whole run after seeding takes at most ten minutes. A plateau qualifies
when every operation keeps its judged percentile inside its latency budget,
the errors stay inside the error budget, the achieved rate keeps up with the
offered rate, and the load generator kept up. One `429` voids the record.

## How to read the record

The record is a `perf-result` document,
`docs/conformance/ferroehr/perf-result.json`:

- **The knee** is the highest offered rate whose plateau qualifies with every
  plateau below it. It is a bracket between that plateau and the first one
  that fails, a lower bound when every plateau qualifies, or below the ladder.
- **The peak** is the highest achieved throughput, whatever its latency.
- **Percentiles** p50 to p99.9 per operation come from the recorded
  histograms with their confidence intervals. A percentile with too few
  samples above it reads `n/r`, not resolvable, and each operation is judged
  at the highest percentile that resolves.
- **Population** turns the knee into the range of people a deployment of that
  capacity serves at its peak, beside the corpus scale it was measured at.
- **Server-side readings** (CPU, memory and I/O per container, database sizes,
  the top `pg_stat_statements` entries, the PostgreSQL settings) are context
  for capacity planning and never enter a judgement.
- **Between-run noise** reads `unmeasured` unless an A/A floor is attached
  (below).

`veredictum perf-judge` re-derives every derived number from the recorded
histograms and fails when one byte differs, so the record can be checked by
anyone who has it.

## The workload

The published record offers `clinical-mix`, Veredictum's reference workload:
what a clinical front end sends, with chart review as short sessions on one
EHR, ward boards, problem lists, cohort counts, AQL over ad-hoc and stored
queries, and the order, result, correction, folder, tag, definition,
demographic and bootstrap traffic around them. Every share in the mix names
its derivation, and the payloads are published openEHR CKM templates. The
pinned workloads, their expanded per-operation mix and their digests are
listed on Veredictum's
[Performance workloads](https://veredictum.eu/docs/perf-workloads.html) page.

## The measured posture

A latency figure is a claim about a configuration, so the run declares one
and its canaries check it before and after the measured window. FerroEHR is
measured as `minimal-signed-digest`: commits validated against the
operational template and every version digest-signed, which is how FerroEHR
ships. The measurement overlay `docker/sut-measurement.yml` changes two
things against the conformance deployment, both visible in the file: it
switches per-caller rate limiting off, because the scout ramp offers load past
the knee from one caller on purpose, and it switches response compression
off, because every posture profile declares it off and a compressed answer
would refuse the run.

## The durability floor: what a single write can never beat

Every committed version is durable: the transaction's WAL records are flushed
to disk before the server answers, so an acknowledged commit survives a crash.
That flush (one `fsync` on the WAL device per commit) is a physical lower
bound on single-client write latency, and no storage design takes a lone
sequential client below it. On a laptop-class Docker setup the flush alone
dominates a single write's budget; on server NVMe it is typically an order of
magnitude smaller. FerroEHR's optimization target is therefore everything *around* the
flush: the database's own per-commit work (one folded commit statement, one
merged placement read) stays far below the flush itself. That is also what
makes concurrent throughput scale: PostgreSQL group-commits, amortizing one
flush across every transaction that reaches it in the same window.

The knob behind this boundary is PostgreSQL's
[`synchronous_commit`](https://www.postgresql.org/docs/18/runtime-config-wal.html#GUC-SYNCHRONOUS-COMMIT).
Setting it `off` makes commits return before the WAL flush: single-client
write latency drops to the statement work alone, and a server crash can lose
the most recent acknowledged commits (up to three times `wal_writer_delay`,
per the PostgreSQL documentation). The database stays consistent; the tail of
acknowledged writes is what is at risk. **FerroEHR never defaults this off
and does not recommend it for clinical data.** It is an operator decision,
made per deployment on the database side, defensible only where the record of
loss is acceptable (a load-test rig, a reseedable sandbox, an analytics
replica). All published FerroEHR numbers are measured with full durability
on.

## Reproducing it

The measured lane is a stage of the conformance pipeline. It composes the
conformance deployment with the measurement overlay on fresh volumes, runs
`veredictum perf`, re-derives the record with `perf-judge`, and redraws the
charts on this page:

```bash
bash scripts/conformance.sh perf

# the same deployment measured against itself, interleaved, so the record
# carries its between-run noise floor
bash scripts/conformance.sh perf-aa
```

`CONF_PERF_WORKLOAD` picks another pinned workload (`aql-mix` offers the six
AQL classes alone), `CONF_PERF_POSTURE` another posture profile, and
`CONF_PERF_OUT` another record path. The stage runs the pinned binary on the
host, because the settle phase reaches the database container with
`docker exec`; the IXIT's `containers` block names the containers. To measure
a deployment the pipeline does not compose, drive the instrument directly:

```bash
source scripts/lib/veredictum.sh
"$(veredictum_bin)" perf --root "$(veredictum_perf)" \
  --catalogue "$(veredictum_catalogue)" \
  --ixit docs/conformance/party/ferroehr/ixit.json \
  --statement docs/conformance/party/ferroehr/statement.json \
  --workload "$(veredictum_workload clinical-mix)" \
  --posture minimal-signed-digest \
  --host-prescription reference-8cpu-8gb \
  --out perf-result.json
"$(veredictum_bin)" perf-judge --result perf-result.json
```

`scripts/render/perf-assets.sh` draws the charts and the summary from the
committed record with `veredictum perf-assets`, and the docs CI job re-renders
and diffs them, so a hand-edited or stale chart fails the build.

## Where a published figure is measured

Two hosted runs of the same code can read far apart, so an absolute figure
from a shared CI runner says little. A published record is measured on the
idle reference host and names it with a host prescription
(`reference-8cpu-8gb`, under `perf/` in Veredictum); the record reads the
prescription against the fingerprint the load generator measured, and a host
that does not match reads `unmatched` with every difference. On any other
machine, compare two builds instead: `veredictum perf-compare --ab` runs both
sides interleaved on one host from one seed, and a difference counts only
when it clears the tolerance and the A/A floor.

## The storage benchmark harness

A second instrument measures the storage layer directly, below the wire:

```bash
STORAGE_BENCH_CLASS=poc cargo bench -p ferroehr --bench storage
```

It seeds a corpus through the ordinary write path, then times the storage
layer's hot paths one at a time: a composition commit, a supersession, the
supersession that states the version it replaces, a point read by version uid
and one by versioned-object uid, the version at an instant, the revision
history, an AQL CONTAINS chain over one EHR and over the whole population,
archive and restore of an EHR, and one retention prune.

Beside each operation's wall-clock it records what the database did for it. The
per-relation tuple counters (inserted, updated, updated in place, dead, live)
and the relation sizes come from `pg_stat_user_tables`, the buffer hits and
misses from `pg_stat_database`, and the write-ahead-log bytes from the WAL
position either side of the measurement. One representative commit is probed on
its own, and the population query is explained with
`EXPLAIN (ANALYZE, BUFFERS, WAL)` inside a transaction that is rolled back, so
the plan shape and its index choices land in the record too.

Everything runs through the service and the public storage API, and the
relations are discovered from the catalogue, so no table name appears anywhere
in the harness and the same file measures a rewritten schema. That is the
point: a storage change is argued from a before-and-after pair taken with one
instrument. It earns no class and touches no conformance artifact; the record's
shape is documented in `docs/benchmarks/storage/README.md`. Records are local by
default, and the committed baseline is what this page renders.

### Comparing two builds on one runner

A dispatch-only workflow (`.github/workflows/storage-bench.yml`) runs the same
benchmark on one GitHub-hosted runner, against a PostgreSQL 18 service with its
durability left on, for two builds: a baseline and a candidate. The candidate
is the dispatched ref, and the baseline is its merge base with `main` unless
the dispatch names another revision, so a dispatch on `main` measures the same
code twice and shows the lane's own noise. The two builds run in turn, several
rounds each (eight by default), in alternating order, and each round seeds a
fresh database and runs `ANALYZE` before it measures anything. A runner that
slows down part way through then slows both builds alike.

Both builds are measured on the same machine because a latency compared with a
record from another runner says little: two hosted runs of the same bench code
read 20 of 27 operations 40 to 156 % apart. Two verdicts come out of the
rounds, and either one fails the run:

- **The plan verdict** reads pages per call, the buffer hits and reads per
  call, which do not depend on the speed of the machine. An operation fails
  when the candidate's median rose more than the page tolerance (10 % by
  default), and by at least one page, above the highest baseline round.
- **The time verdict** first divides each round by its machine factor, so a
  round where the whole runner ran slow does not read as the code. An
  operation regresses only when its median p50 or p99 rose more than the
  tolerance (15 % by default) and every candidate round is slower than every
  baseline round at that percentile. A rise whose rounds overlap the
  baseline's is reported as within the noise of the rounds.

The candidate's storage sizes do not depend on the machine, so they are judged
against the record committed for its schema generation and class: bytes per
composition version or per node row that grew more than the size budget (5 %
by default) fail the run. A generation and class with no committed record fails
the run, with the error in the job summary, so the change that moves the schema
generation commits the new generation's records with it (#4291). The run
summary lists every round with its machine factor, and each operation with its
pages per call and its raw and adjusted change.

That lane is exploration, never a conformance record. It earns no performance
class, publishes no number, and writes nothing back to the repository: the
round records leave as a run artifact, and the committed baseline under
`docs/benchmarks/storage/<generation>/` moves only in a pull request that says
which run measured it. `docs/benchmarks/storage/README.md` records the numbers
behind each bound (#4318).

{{#include ../generated/storage-bench.md}}

## The published figures

The figures below are the last ones measured with the earlier generation of
the instrument, the Veredictum 0.1 line: a proof-of-concept class run, the
step-load stress ladder and the AQL probe, against FerroEHR 4.3.0. Veredictum
0.3.1 replaced those three instruments with `perf`, so they cannot be re-run. A `perf-result` record measured on the reference host replaces them,
and these charts and their records go with it.

![The performance class ladder](perf-assets/perf-class-ladder.svg)

![Proof-of-concept class latency, re-derived from the committed histograms](perf-assets/perf-latency-class-POC.svg)

![Container CPU, memory and I/O across the measured window](perf-assets/perf-resources-class-POC.svg)

![Database volume growth across the run's four anchors](perf-assets/perf-disk-growth.svg)

![The latency-throughput curve from the committed stress run](perf-assets/perf-stress-curve.svg)

{{#include ../generated/perf-summary.md}}
