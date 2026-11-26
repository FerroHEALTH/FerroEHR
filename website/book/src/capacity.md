# Capacity

The performance gate drives a fixed 20 requests per second to compare a
change against its base. It says nothing about how far one deployment
scales. The capacity test answers that: it steps the arrival rate up until
FerroEHR degrades, and records what each part of the system spent at every
step, so the step where it degrades names what saturated first. No openEHR
specification governs capacity: the method and the budgets on this page are
FerroEHR's own design.

<!-- toc -->

## What it measures

`scripts/perf/capacity.sh` runs one or more FerroEHR instances on one
PostgreSQL 19, with Caddy in front when there is more than one (round robin,
no sticky sessions, as [Running several
instances](installation/several-instances.md) deploys them). It seeds them
with the performance gate's access seed (`scripts/perf/access-seed.sh`):

- an `ACCESS_POLICY` that requires a care relationship and holds a controller
  rule letting the clinician's role read HIV data;
- a care team with the clinician as a member, and 20 000 patients under its
  care, each EHR carrying five directives;
- an Ed25519 OpenPGP key generated for the run, which signs the access log.

The access check and the access log stay on, as in production. On top of
the gate's seed, the first 500 population EHRs get one composition each, so
the single-EHR groups read across 500 EHRs, and the commits go to the next
500, so a query's answer does not grow while the sweep runs. The single-EHR
AQL reads EHRs of its own, filled as a chart is: the next 500 hold 20 lab
results each, a typical answer, and the 50 after them 100 each, a whole
chart.

The load is `ferrohealth-perf`, the version `docs/VERSIONS.md` pins for the
gate, run open loop at a constant arrival rate. Each latency counts from the
instant the request was due, so a stalled server raises the latency of every
request it held back (no coordinated omission), and a step whose generator
fell behind its schedule is refused. FerroEHR's published figures come from
[Veredictum's `perf`](performance.md), which offers Veredictum's own
`clinical-mix` workload; the capacity test exists beside it because it runs
FerroEHR's access path at scale and sweeps each request group on its own.

## The groups and the mix

| group | request | caller | weight in the mix |
|---|---|---|---:|
| `ehr_read` | `GET /ehr/{id}` over 500 EHRs | clinician with a care relationship | 3 |
| `composition_read` | `GET /ehr/{id}/composition/{uid}` over 500 EHRs | clinician | 5 |
| `composition_commit` | `POST /ehr/{id}/composition` over 500 EHRs | clinician | 3 |
| `aql_ehr` | the lab-results AQL with `ORDER BY`, one EHR of 20 results, over 500 EHRs | clinician | 3 |
| `aql_chart` | the same AQL, one EHR of 100 results, over 50 EHRs | clinician | 1 |
| `aql_population` | `SELECT e/ehr_id/value FROM EHR e LIMIT 20` | clinician | 1 |
| `aql_caseload` | the newest 20 compositions of the clinician's whole caseload | clinician | 0 |
| `access_log_read` | ITI-81 `AuditEvent?date=ge2026-01-01&_count=20` | administrator | 1 |
| `party_read` | `GET /demographic/person/{id}` | administrator | 2 |
| `care_commit` | `POST /demographic/party_relationship`: a new care relationship for each of 500 patients per round, as an ADT feed writes one per encounter | administrator | `CAPACITY_CARE_WEIGHT` (0) |
| `export` | the EHR export, over 500 EHRs | clinician | 0 |
| `health` | `GET /health/liveness`, the harness's own ceiling | none | 0 |

The mix is 19 requests per round: 15 single-EHR clinical requests (3 of
them writes), one population query, one access-log read and two
demographic reads. `capacity.sh groups` prints the table the script reads.

The callers are the clinician, judged through a care relationship, the
controller rule and each EHR's directives, and the administrator. A patient
reading their own record needs a SMART token whose login binds their own
`PERSON` (a Basic caller never is), and a system client needs a
client-credentials token; the harness mints neither yet, so neither caller
is measured.

## The knee

A step passes when all of these hold:

- every request was answered with the expected status;
- the generator kept its schedule (lag p99 at most 10 milliseconds) and never waited
  for a free slot;
- the answered rate is at least 98 % of the offered rate;
- every group's p99 is within its SLO: 100 milliseconds for the single-EHR groups,
  500 milliseconds for the population query, the access-log read and the export.
  Both the generator's p99 and the server's own, from its request log, must
  be within it.

A step run several times (`CAPACITY_REPEAT`) passes when every run kept its
schedule and answered as expected, and each group's median p99 is within its
SLO. The knee is the highest rate that passes with every rate below it
passing. A sweep stops at the first step that fails; with
`CAPACITY_CONTINUE=1` it runs every rate, records each step's outcome
(pass, fail, or error when the harness could not measure it) in
`sweeps.jsonl`, and the summary lists them. 100 milliseconds is the response time a person reads as immediate, the
budget a chart view built from several reads has to fit in. A population
query and an access-log review are reporting work, so they get five times
that.

## Running it

On a Linux workstation, with PostgreSQL 19, Caddy and a release build of the
server:

```sh
cargo build --locked --release -p ferroehr-server
bash scripts/perf/gate.sh install
export CAPACITY_FERROEHR=target/release/ferroehr
export CAPACITY_PG_BIN=/usr/lib/postgresql/19/bin CAPACITY_CADDY=$(command -v caddy)
bash scripts/perf/capacity.sh up 1          # or 2, behind Caddy
bash scripts/perf/capacity.sh seed          # about 30 minutes at 20 000 patients
bash scripts/perf/capacity.sh snapshot      # keep the seeded store for `restore`
bash scripts/perf/capacity.sh sweep health 500 1000   # the floor of the path to the server
bash scripts/perf/capacity.sh sweep mix     # 50 100 200 500 1000 1500 2000 3000
bash scripts/perf/capacity.sh sweep ehr_read 200 500 1000
bash scripts/perf/capacity.sh profile mix 500 20   # an external CPU profile at one rate
bash scripts/perf/capacity.sh summary
bash scripts/perf/capacity.sh down
```

On macOS, run the containers instead (`CAPACITY_TOPOLOGY=docker`, with
Podman answering as `docker`): PostgreSQL in the Podman VM, the server
beside it, and the generator on the host. Do not run PostgreSQL natively on a
macOS host with endpoint protection: plain PostgreSQL inserts there stall for
seconds while the scanner reads the data files, and every write path inherits
the stall. Do not run the server on the host against PostgreSQL in the VM
either: each statement then crosses the VM's port forwarder, about a millisecond per
round trip.

In GitHub Actions, dispatch **Capacity test** (`perf-capacity.yml`) with the
image, the instance count, the groups and the rates. It runs the containers
on one runner with the generator pinned to the first core and the
containers to the others, and keeps every step as an artifact. A hosted
runner shares its host with other jobs, so read its knee as a floor.

Run `composition_commit` and the mix last when you sweep several groups on
one store: their commits grow the store the other groups read, or set
`CAPACITY_FRESH=1` so every run starts from the seeded store. A variant,
such as a longer `[auth.basic] cache_ttl`, goes in
`CAPACITY_SERVER_ENV` and belongs in the label of every figure it produced.

## Care relationships at practice scale

The seed puts `CAPACITY_POPULATION` patients in the care of the clinician's
team, so the population groups read a caseload of that size. Every
`care_commit` request is a new relationship, which stamps the patient's
party; every replica's copy of the caller facts and the care projection read
the stamped parties' facts. A sweep of `care_commit` on its own measures how
many relationship commits per second the party store takes; a mix with
`CAPACITY_CARE_WEIGHT` above 0 measures the reads while an ADT feed writes.

```sh
export CAPACITY_POPULATION=50000          # a GP practice's list; 1600 for a ward
bash scripts/perf/capacity.sh up 1
bash scripts/perf/capacity.sh seed
bash scripts/perf/capacity.sh snapshot
CAPACITY_FRESH=1 CAPACITY_REPEAT=3 CAPACITY_SLO_MS=30 \
  bash scripts/perf/capacity.sh sweep care_commit 100 200 500 1000 2000 3000
CAPACITY_FRESH=1 CAPACITY_REPEAT=3 CAPACITY_SLO_SLOW_MS=100 \
  bash scripts/perf/capacity.sh sweep aql_caseload 100 500 1000
CAPACITY_FRESH=1 CAPACITY_REPEAT=3 CAPACITY_CARE_WEIGHT=1 CAPACITY_SLO_MS=30 \
  CAPACITY_SLO_SLOW_MS=100 bash scripts/perf/capacity.sh sweep mix 1000 2000 3000
bash scripts/perf/capacity.sh summary
```

The plans these queries take at 100, 1 600 and 50 000 patients per caseload
are held by the integration test `care_caseload_plans`; its figures are on
[Care relationships and episodes of care](care-relationships.md#what-it-costs).

## Trusting a number

A latency from a shared or virtualised host can carry more of the host than
of FerroEHR. Every step records what it ran on, so a reader can tell, and
the harness has a way around each distortion.

**The path to the server.** The Podman VM on macOS forwards every published
port, and the forwarder adds about 1.6 milliseconds to each request the
generator times. Every step therefore reports the server's own p50, p95 and
p99 per group, read from its request log (`latency_ms`) over the measured
window, beside the generator's. In the mix, the three AQL groups share one
route, so the server reports them as one group, `aql`. A `health` step
measures the floor: the summary names the client p50 of
`/health/liveness` against the server's as the forwarder floor of the run.
With `CAPACITY_GENERATOR=network` (docker topology) the generator runs in a
container on the servers' network and reaches them by name, with no
forwarder between; `capacity.sh generator` builds it once, from the Rust
image `docker/Dockerfile` pins.

**The clock.** The Podman VM runs its guest on the physical counter, so
every clock read traps to the hypervisor: at 2000 requests per second about
one of its 8 cores goes to reading the clock, split between FerroEHR and
PostgreSQL (`track_io_timing` and `pg_stat_statements` timing are on). CPU
per request and the knee from that host overstate FerroEHR's cost. The
latency targets are judged on the **Capacity test** workflow's runner, a
GitHub-hosted `ubuntu-26.04` x86_64 machine whose clock is read through the
vDSO, with the generator on the servers' network and each step run three
times. The laptop VM serves relative comparisons only. Each step's
`host.json` records the clock source of the kernel the containers ran on.

**Run-to-run spread.** On a shared host, one p99 can move by more than the
change under test. `CAPACITY_REPEAT=3` runs each step three times; the
summary gives each group's median p99 and the spread of the three, and
`capacity.sh compare <arm> <arm>` calls a difference only when the two
arms' spreads do not overlap and each ran at least three times.

**A store that grows.** An arm that runs second reads a store the first one
grew. `capacity.sh snapshot` keeps the seeded store as a template database,
and `capacity.sh restore` copies it back and restarts the servers, so no
in-memory copy outlives the store it was read from. To compare
configurations on one image, interleave them:

```sh
bash scripts/perf/capacity.sh snapshot
CAPACITY_REPEAT=3 bash scripts/perf/capacity.sh arms ehr_read 3000 \
  base= cache=FERROEHR__AUTH__BASIC__CACHE_TTL=1h
bash scripts/perf/capacity.sh compare base cache
```

Each round restores the snapshot before each arm and reverses the arms'
order every other round, so a host that drifts over the rounds weighs on
both alike. Two images are compared each on a store of its own, seeded
fresh, since their migrations differ.

**Other tenants.** Each step's `host.json` holds the host's load averages
and cores at both ends, the same for the kernel the containers run on, and
the CPU every container outside the harness spent during the step. The
summary's host table shows the worst run of each step: a load above the
cores, or other containers using a core or more, marks a step whose latency
is an upper bound.

**What the release needs besides latency.** Each step records the peak
resident set of every instance and the peak memory of each container
(`memory.json`), and the store's bytes per composition version and per
access-log entry, both over the whole store and for what the step itself
added (`storage.json`). The server's own report of its resident set and
of each in-memory copy's entries and bytes (`process_resident_memory_bytes`,
`access_copy_entries`, `access_copy_size_bytes`) goes into each step's
`step.json` as `server_metrics`. The summary's memory and storage table
carries them at every rate.

## Soak: memory that does not grow

A peak at each rate says nothing of a slow leak, which a step of a minute
cannot show. A soak holds one rate for a long time and samples each
instance's resident set and copies, as the server reports them, every
`CAPACITY_SOAK_INTERVAL` seconds (default 10):

```sh
CAPACITY_SOAK_SETTLE=900 bash scripts/perf/capacity.sh soak mix 1000 7200
```

The soak fits a straight line to each instance's resident set over the
samples taken after the warm-up and `CAPACITY_SOAK_SETTLE` seconds more
(default 600), in which the bounded caches fill. It fails when an instance's
slope exceeds `CAPACITY_SOAK_MAX_MIB_PER_HOUR` (default 16 MiB per hour)
and its last sample is above its first in that window: a resident set that
keeps rising after warm-up. 16 MiB per hour is about 0.4 GB a day; no
specification governs the figure, it is FerroEHR's own. The soak
also fails as a step fails (refusals, a lagging generator, a p99 over its
SLO). Its step goes under `steps/soak/` as any step does; the samples
(`soak.tsv`) and the fit and verdict per instance (`soak.json`) go under
`soaks/<group>-<rate>/`, and a line in `soaks.jsonl` feeds the summary's
soak table.

The script's header lists every setting: the pool size
(`CAPACITY_POOL`, the shipped default of 20 connections per instance), the
concurrent-request bound (`CAPACITY_MAX_CONCURRENT`, 256 as in the
quickstart, so a step past the knee fails on `503` refusals instead of the
server running out of memory), the
population, the spread, the results in each EHR the AQL groups read, the
warm-up and measured window per step, and the SLOs.

## Reading a step

Each step leaves a directory, `target/capacity/steps/[<arm>/]<group>/<rate>/`,
with one `run-<n>/` directory per run, and one line in `steps.jsonl`:

- `step.json`: the verdict over the runs, the answered rate, p50, p95 and
  p99 per group as the generator and the server measured them (each the
  median over the runs, a p99 with every run's value and their spread), the
  failures by kind, and the CPU seconds of the servers, PostgreSQL, the
  balancer and the generator over the step, with the cores each used and the
  CPU milliseconds per request. `transactions_per_request` counts
  PostgreSQL's commits and rollbacks over the step. `host_load_1m` against
  `host_cores` shows a host that was busy with something else.

Each `run-<n>/` holds that run's `step.json`, the files below, and the
`statements.json`, `activity.json` and `prometheus-<n>.txt` this list
describes after them:

- `host.json`: the host's and the containers' kernel's load averages and
  cores at the start and the end, the kernel's clock source, and every other
  container with the CPU seconds and cores it used during the run.
- `server_latency.json`: the server's own p50, p95 and p99 per group, from
  its request log over the measured window.
- `memory.json`: the peak resident set (`VmHWM`) of each instance, and the
  peak, current and anonymous bytes of each container's cgroup.
- `storage.json`: the database's bytes, each domain schema's bytes, the
  composition versions and access-log entries it holds, the bytes per
  version and per entry, and what the run added.
- `statements.json`: the 15 statements of `pg_stat_statements` that took the
  most time in the step, planning included: PostgreSQL counts each
  statement's planning beside its execution (`track_planning`), and an AQL
  statement is planned for its values on every call, so at a small answer
  its execution alone leaves out about half of what it costs.
- `activity.json`: the database's client backends by state and wait event,
  sampled four times a second, as a mean count; `peak_active` is the most
  backends active at once. A `Lock:advisory` or `LWLock` wait there is
  contention; `ClientRead` means PostgreSQL waits on FerroEHR.
- `prometheus-<n>.txt`: each instance's metrics after the step, among them
  `db_pool_connections`, `db_pool_acquire_duration_seconds` and
  `db_pool_hold_duration_seconds` (each by `pool`), `audit_chained_total`
  and the access log's write and sign timings.

Read the first failing step against the last passing one. When the server's
cores approach what the host gives it while PostgreSQL has room, the server
saturated; profile it with `capacity.sh profile`, which samples the server
with `sample` on macOS or `perf record` on Linux. The server's own
`/management/flamegraph` is not used while its unwinder can stop the server
inside an Ed25519 signature. When the pool's
acquire time rises and the active backends sit at the pool size, the pool
saturated. When one statement's total time grows faster than the rate, or
backends wait on one lock, that statement or lock is the limit.

## Where the measurements are

The tracker issue of each run holds its result, with the hardware it ran on.
A knee is a property of one deployment on one host, so
the book publishes the method and never a figure without its host.
