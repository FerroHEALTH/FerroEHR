# cdr-benchmark campaign records (#6237)

Each section is one quiet-host campaign of cdr-benchmark v0.19.0 and the
capacity harness against a published FerroEHR `main` image, under #6236. The
figures change with the code: compare a later campaign against these rows,
never against an estimate.

**A benchmark record, not a conformance record.** No openEHR specification
governs how fast a CDR answers.

## main `ef9de89e6a`, 2026-11-26

### Set-up

- Server `ghcr.io/ferrohealth/ferroehr@sha256:44fa8865…` (arm64, revision
  `ef9de89e6a`), PostgreSQL 19.0 `ghcr.io/ferrohealth/ferroehr-postgres@sha256:2fc22cc0…`,
  data checksums on.
- One 8-vCPU 16 GiB Podman VM (`libkrun`) on an 18-core Apple silicon host;
  the generators run on the host.
- cdr-benchmark v0.19.0 (`7a3405d`, openehr-sdk-go v0.51.0), the default
  workload. The quickstart `docker-compose.yml` of `ef9de89e6a` with the local
  subject index, `[server.caller_rate]` removed (the benchmark's "rate limiter
  off"), the database published, `database.max_connections` 20, connection
  budget 70 and the request log at `warn`. Each run is the warm-store method:
  a fresh store, an unmeasured warm-up (`--ehrs 100 --seed 1`), `ANALYZE` and
  `CHECKPOINT`, then the measured run (`--ehrs 200 --seed 17112026`, 150,636
  requests).
- The capacity harness (`scripts/perf/capacity.sh`, ferrohealth-perf 0.15.0),
  docker topology, the default `mix`, 20,000 patients under care, open loop,
  15 s warm-up and 60 s measured, two runs a step, each restored from the
  seeded snapshot.
- Each run started once the host was at least 85% CPU idle over 2 s with no
  cargo or rustc process and no other container; the host's 1-minute load at
  each start is in the tables (other agents' processes count in it).
- Per-run figures: `ef9de89e6a.jsonl` (cdr-benchmark) and `capsum-raw.md`
  (capacity harness).

### cdr-benchmark v0.19.0

Operations are 2xx job-run requests per second. Latencies in ms (p99 is
count-weighted over a group's rows). CPU from the containers' cgroups; "VM"
is the whole VM's busy CPU. Waits are PostgreSQL client backends sampled at
10 Hz, as mean backends per state.

| run | workers | ops/s | validity | GET composition p50 / p99 | POST composition p99 | CONTRIBUTION p99 | server / PG CPU-ms per op | VM cores | VM CPU-ms per op | commits / statements per op | WAL B per op | PG wire B per op | waits: CPU / WALWrite / WalSync | clinical pool acquires > 1 ms | anon MB server / PG | host load at start |
|---|---:|---:|---|---|---:|---:|---|---:|---:|---|---:|---:|---|---:|---|---:|
| step.w4 | 4 | 1,438 | ok | 1.98 / 3.82 | 7.18 | 9.33 | 0.558 / 1.075 | 3.62 | 2.52 | 2.75 / 3.03 | 8,310 | 21,520 | 1.27 / 0.11 / 0.44 | 0.0% | 406 / 807 | 3.58 |
| step.w8 | 8 | 2,091 | ok | 2.78 / 6.02 | 10.61 | 12.65 | 0.586 / 1.069 | 4.91 | 2.35 | 2.57 / 2.86 | 8,215 | 21,109 | 1.78 / 0.59 / 0.66 | 0.0% | 404 / 828 | 5.46 |
| step.w16 | 16 | 3,025 | ok | 3.82 / 9.05 | 14.57 | 19.05 | 0.560 / 0.906 | 5.95 | 1.97 | 2.34 / 2.64 | 8,149 | 20,646 | 2.51 / 1.50 / 0.76 | 0.0% | 462 / 810 | 4.92 |
| step.w32 | 32 | 3,499 | ok | 6.97 / 17.45 | 22.94 | 30.61 | 0.528 / 0.894 | 6.49 | 1.85 | 2.19 / 2.48 | 8,112 | 20,331 | 3.48 / 2.85 / 0.71 | 6.9% | 461 / 871 | 5.49 |
| ref24.r1 | 24 | 3,297 | ok | 5.43 / 12.95 | 18.43 | 24.90 | 0.567 / 0.921 | 6.39 | 1.94 | 2.23 / 2.53 | 8,130 | 20,411 | 3.24 / 2.03 / 0.75 | 0.1% | 556 / 814 | 6.23 |
| ref24.r2 | 24 | 3,508 | ok | 5.13 / 12.71 | 17.82 | 22.32 | 0.529 / 0.853 | 6.35 | 1.81 | 2.21 / 2.53 | 8,066 | 20,271 | 3.12 / 2.08 / 0.74 | 0.1% | 464 / 809 | 5.21 |
| ref24.r3 | 24 | 3,479 | ok | 5.16 / 12.35 | 17.11 | 23.12 | 0.523 / 0.872 | 6.35 | 1.83 | 2.24 / 2.53 | 8,175 | 20,406 | 3.19 / 2.20 / 0.76 | 0.1% | 497 / 834 | 4.43 |
| ref64.r1 | 64 | 3,792 | ok | 14.08 / 29.98 | 35.38 | 48.89 | 0.512 / 0.875 | 6.76 | 1.78 | 2.12 / 2.42 | 7,822 | 19,711 | 4.19 / 3.19 / 0.73 | 94.6% | 502 / 886 | 6.02 |
| ref64.r2 | 64 | 3,479 | ok | 15.30 / 34.02 | 40.71 | 56.33 | 0.578 / 0.924 | 6.49 | 1.87 | 2.14 / 2.43 | 7,993 | 19,173 | 4.03 / 2.94 / 0.75 | 95.1% | 545 / 864 | 6.33 |
| ref64.r3 | 64 | 3,793 | ok | 14.11 / 29.15 | 34.33 | 47.01 | 0.515 / 0.879 | 6.74 | 1.78 | 2.12 / 2.42 | 7,898 | 19,540 | 3.85 / 3.15 / 0.70 | 94.6% | 566 / 849 | 7.42 |
| ref128.r1 | 128 | 3,815 | ok | 16.16 / 37.40 | 41.79 | 57.76 | 0.501 / 0.858 | 6.66 | 1.75 | 2.12 / 2.41 | 7,850 | 19,618 | 4.07 / 3.34 / 0.69 | 96.7% | 503 / 865 | 5.69 |

The benchmark's own `op/s` line reads 3,344 to 3,558 at 24 workers and 3,529
to 3,870 at 64 to 128.

- **The limit:** from 32 workers the VM is 6.5 to 6.8 of 8 cores busy and 2.9
  to 3.3 backends wait in `LWLock:WALWrite`; the clinical pool of 20 queues
  behind them (95% of acquires over 1 ms at 64 workers).
- **Against 8,000 operations a second on 8 vCPU** (about 1.0 VM CPU-ms an
  operation): 1.75 to 1.94 today, so the work per operation has to fall
  about 1.8 to 1.9 times.
- `cdr-benchmark compare` against the previous campaign's `ref24.r1`
  (`32fe431bd1`): composition reads p95 −23 to −27%, template examples
  −25%, deletes −26%.

### Capacity harness, mix

Per request: CPU from the containers' cgroups, "VM" the whole VM's busy CPU.

| offered rps | answered (run 1 / run 2) | failed (run 1 / run 2) | server / PG CPU-ms per req | VM CPU-ms per req | transactions per req | host load at start |
|---:|---|---|---|---:|---:|---|
| 1,000 | 1,000 / 1,000 | 0 / 0 | 0.93 / 1.96 | 4.51 | 2.97 | 4.61 / 7.99 |
| 2,000 | 1,998 / 2,000 | 82 / 0 | 0.79 / 1.94 | 3.74 | 2.41 | 6.92 / 6.76 |
| 3,000 | 1,819 / 1,944 | 84,954 / 76,910 | 0.84 / 1.93 | 4.10 | 2.30 | 9.23 / 9.35 |
| 4,000 | 1,901 / 1,781 | 155,378 / 163,551 | 0.89 / 1.92 | 4.34 | 2.29 | 8.53 / 11.51 |
| 5,000 | 1,719 / 1,694 | 244,760 / 246,354 | 0.98 / 1.96 | 4.74 | 2.28 | 10.93 / 9.54 |

- **#6325's criterion 4:** at 4,000 rps the harness answers 97.8% of what it
  answers at 3,000 (1,841 against 1,882); at 5,000, 90.7% (1,707).
- **PostgreSQL CPU per request is 1.92 to 1.96 ms**, where the campaign at
  `3d619d49bf` measured 1.12 to 1.37. The statements behind it are on #6635:
  the facts check `party.facts_behind` takes 171 to 202 buffers a call and
  0.53 to 0.93 ms, on every request of the care posture.
