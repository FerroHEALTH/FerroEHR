# PostgreSQL 19 against 18.6 on the request path

This record measures what moving from PostgreSQL 18.6 to 19.0 changes under
FerroEHR's requests, apart from any change in FerroEHR's own code (#6218,
under #6237). Main refuses a server older than 19 since #6225, so the server
is compared on the last build that ran on 18.6, `37c5dd3279` (the parent of
#6225's merge), and main is then measured on 19.0 to show what the code
changed since.

**A benchmark record, not a conformance record.** No openEHR specification
governs how a request is executed inside the CDR.

## Set-up

- Two containers on the 8-vCPU Podman VM, `postgres:18.6` and `postgres:19.0`
  (Debian builds `18.6-1.pgdg13+2` and `19.0-2.pgdg13+1`), each started with
  the testkit's settings (`tools/testkit/src/server_settings.rs`) and reached
  through `FERROEHR_TEST_PG_URL`.
- The budget harness `app/ferroehr-rest/tests/it/statement_budget/` at
  `37c5dd3279` on both servers, and at main `ef9de89e6a` on 19.0. Its
  figures are counts (round trips, nested statements, WAL row records,
  allocations), so the host load does not move them.
- The plans of every statement that carries the access filter's unknown-class
  subquery (`aql/sql/access.rs` `class_known`), logged by `auto_explain` with
  `log_analyze` and `log_buffers` while `access_eval_aql`, `care_caseload_plans`
  (without the practice-scale seed) and `aql_planner` ran: 355 plans of 48
  statements on each server.

The load figures on 19.0 (p99 per group, CPU per request, waits) are on #6218
and #6237; they compare across code changes and are not repeated here.

## The same build on both servers

Per request, `37c5dd3279`, 10 measured requests a path:

| path | round trips | nested, 18.6 | nested, 19.0 | WAL B, 18.6 | WAL B, 19.0 |
|---|---:|---:|---:|---:|---:|
| commit JSON | 9 | 11 | 3 | 228,979 | 228,651 |
| commit XML | 9 | 11 | 3 | 228,973 | 228,559 |
| commit FLAT | 9 | 11 | 3 | 228,222 | 228,726 |
| `CONTRIBUTION` commit | 13 | 11 | 3 | 229,634 | 229,562 |
| `EHR_STATUS` commit | 15 | 9 | 3 | 5,944 | 5,944 |
| party commit | 29 | 18 | 12 | 7,586 | 7,586 |
| party read | 8 | 6 | 4 | 1,954 | 1,954 |
| demographic AQL | 8 | 6 | 4 | 1,934 | 1,934 |
| read JSON, XML, FLAT, at time | 3 | 2 | 2 | 2,634 | 2,634 |
| single-EHR AQL | 4 | 2 | 2 | 4,180 | 4,180 |
| population AQL | 3 | 2 | 2 | 10,588 | 10,676 |

- **Round trips, conversions and allocations do not move.** They are the
  code's.
- **WAL is the same within 0.8%.** The IPS commit writes 226,096 B of row
  records on both servers and the lab commit 9,694 B.
- **Foreign-key checks are no longer nested statements on 19.0.** On 18.6 a
  composition commit runs eight `SELECT 1 FROM ONLY … FOR KEY SHARE` lookups
  through SPI (seven on `clinical.ehr`, one on `clinical.template_ref`); on
  19.0 it runs none, and the WAL ceiling test counts 0 nested statements per
  commit where 18.6 counted 8. This is the release note's faster foreign-key
  check ("General Performance").

## The access filter's plans

- **The `NOT IN` stays a hashed subplan on 19.0.** Every one of the 355 plans
  on both servers filters with `NOT (ANY (ehr_id = (hashed SubPlan …)))`. The
  19 anti-join rewrite of `NOT IN` applies to a top-level `WHERE` condition,
  and `class_known` puts the subquery inside a `CASE`, so it does not apply.
- **With fresh statistics 19.0 reads fewer buffers.** With an `ANALYZE` after
  each care projection write (a scratch change to the test, not committed),
  the `access_eval_aql` file reads 8,276 buffers on 19.0 against 9,058 on 18.6.
- **On stale statistics two statements read 5.6 and 6.9 times the buffers.**
  In `a_large_care_census_reads_exactly_the_ehrs_in_care` the test writes
  5,000 `access_care` rows and queries before any `ANALYZE`; both servers
  estimate the caseload scan at 5 rows. 18.6 runs the caseload join once; 19.0
  puts it inside a loop over the candidate versions, or unique-ifies the
  semijoin and builds the whole caseload:

| statement (6 executions) | buffers, 18.6 | buffers, 19.0 | ms, 18.6 | ms, 19.0 |
|---|---:|---:|---:|---:|
| the population `COUNT` | 6,037 | 33,891 | 8.9 | 41.4 |
| the population row read | 970 | 6,655 | 0.9 | 7.2 |

Main on 19.0 shows the same two shapes (33,947 and 6,804 buffers). The
regression is #6630, whose change analyzes the care projection after a
write of 1,000 rows or more.

## Main on 19.0

Main `ef9de89e6a` on 19.0, per request, against `37c5dd3279` on 19.0. Every
difference here is code; the server is the same.

| path | round trips | nested | WAL B | changed by |
|---|---|---|---|---|
| commit JSON | 9 to 4 | 3 to 0 | 228,651 to 176,577 | one guarded statement (#6256), node fragments stored once (#6229), write-once audit (#6242) |
| `CONTRIBUTION` commit | 13 to 4 | 3 to 0 | 229,562 to 176,976 | the same |
| `EHR_STATUS` commit | 15 to 4 | 3 to 0 | 5,944 to 4,163 | #6257 |
| party commit | 29 to 9 | 12 to 5 | 7,586 to 5,971 | #6257, #6271, #6284 |
| read JSON | 3 to 1 | 2 to 1 | 2,634 to 775 | confirm reads on the store read (#6251, #6320), write-once audit (#6242, #6319) |
| single-EHR AQL | 4 to 2 | 2 to 1 | 4,180 to 1,758 | the same |
| population AQL | 3 to 1 | 2 to 1 | 10,676 to 4,762 | the same |
| party read | 8 to 6 | 4 to 1 | 1,954 to 1,035 | the same |

The access-log entry is 775 B of WAL for a composition read on main, against
2,634 B at `37c5dd3279`, on either server.
