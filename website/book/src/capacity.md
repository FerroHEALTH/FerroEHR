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
PostgreSQL 18, with Caddy in front when there is more than one (round robin,
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
the single-EHR groups read and write across 500 EHRs.

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
| `aql_ehr` | the lab-results AQL with `ORDER BY`, one EHR, over 500 EHRs | clinician | 3 |
| `aql_population` | `SELECT e/ehr_id/value FROM EHR e LIMIT 20` | clinician | 1 |
| `access_log_read` | ITI-81 `AuditEvent?date=ge2026-01-01&_count=20` | administrator | 1 |
| `party_read` | `GET /demographic/person/{id}` | administrator | 2 |
| `export` | the EHR export, over 500 EHRs | clinician | 0 |
| `health` | `GET /health/liveness`, the harness's own ceiling | none | 0 |

The mix is 18 requests per round: 14 single-EHR clinical requests (3 of
them writes), one population query, one access-log read and two
demographic reads. `capacity.sh groups` prints the table the script reads.

The callers are the clinician, judged through a care relationship, the
controller rule and each EHR's directives, and the administrator. A patient
reading their own record needs a SMART token from a `persons` issuer whose
caller is the person (a Basic caller never is), and a system client needs a
client-credentials token; the harness mints neither yet, so neither caller
is measured.

## The knee

A step passes when all of these hold:

- every request was answered with the expected status;
- the generator kept its schedule (lag p99 at most 10 ms) and never waited
  for a free slot;
- the answered rate is at least 98 % of the offered rate;
- every group's p99 is within its SLO: 100 ms for the single-EHR groups,
  500 ms for the population query, the access-log read and the export.

The knee is the highest rate that passes; a sweep stops at the first step
that fails. 100 ms is the response time a person reads as immediate, the
budget a chart view built from several reads has to fit in. A population
query and an access-log review are reporting work, so they get five times
that.

## Running it

On a Linux workstation, with PostgreSQL 18, Caddy and a release build of the
server:

```sh
cargo build --locked --release -p ferroehr-server
bash scripts/perf/gate.sh install
export CAPACITY_FERROEHR=target/release/ferroehr
export CAPACITY_PG_BIN=/usr/lib/postgresql/18/bin CAPACITY_CADDY=$(command -v caddy)
bash scripts/perf/capacity.sh up 1          # or 2, behind Caddy
bash scripts/perf/capacity.sh seed          # about 30 minutes at 20 000 patients
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
either: each statement then crosses the VM's port forwarder, about 1 ms per
round trip.

In GitHub Actions, dispatch **Capacity test** (`perf-capacity.yml`) with the
image, the instance count, the groups and the rates. It runs the containers
on one runner with the generator pinned to the first core and the
containers to the others, and keeps every step as an artifact. A hosted
runner shares its host with other jobs, so read its knee as a floor.

The script's header lists every setting: the pool size
(`CAPACITY_POOL`, the shipped default of 20 connections per instance), the
population, the spread, the warm-up and measured window per step, and the
SLOs.

## Reading a step

Each step leaves a directory, `target/capacity/steps/<group>/<rate>/`, and
one line in `steps.jsonl`:

- `step.json`: the verdict, the answered rate, p50, p95 and p99 per group,
  the failures by kind, and the CPU seconds of the servers, PostgreSQL, the
  balancer and the generator over the step, with the cores each used and the
  CPU milliseconds per request. `transactions_per_request` counts
  PostgreSQL's commits and rollbacks over the step. `host_load_1m` against
  `host_cores` shows a host that was busy with something else.
- `statements.json`: the 15 statements of `pg_stat_statements` that took the
  most time in the step.
- `activity.json`: the database's client backends by state and wait event,
  sampled four times a second, as a mean count; `peak_active` is the most
  backends active at once. A `Lock:advisory` or `LWLock` wait there is
  contention; `ClientRead` means PostgreSQL waits on FerroEHR.
- `prometheus-<n>.txt`: each instance's metrics after the step, among them
  `db_pool_connections`, `db_pool_acquire_duration_seconds`,
  `audit_drain_queue_depth` and the access log's append and sign timings.

Read the first failing step against the last passing one. When the server's
cores approach what the host gives it while PostgreSQL has room, the server
saturated; profile it with `capacity.sh profile`, which samples the server
with `sample` on macOS or `perf record` on Linux. The server's own
`/management/flamegraph` is not used while its unwinder can stop the server
inside an Ed25519 signature (#5723). When the pool's
acquire time rises and the active backends sit at the pool size, the pool
saturated. When one statement's total time grows faster than the rate, or
backends wait on one lock, that statement or lock is the limit.

## Where the measurements are

The tracker issue of each run holds its result, with the hardware it ran on:
the first is #5722. A knee is a property of one deployment on one host, so
the book publishes the method and never a figure without its host.
