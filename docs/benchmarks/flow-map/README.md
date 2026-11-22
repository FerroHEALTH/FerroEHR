# Operation flow map (cdr-benchmark v0.19.0)

This record maps every operation the cdr-benchmark v0.19.0 mix sends through
FerroEHR, step by step, with what each step costs and whether a
specification or a binding rule needs it (#6239, under #6236). Each step is
marked **required**, with its citation, or **our own design**, with the
decision taken on it: delete, merge or keep. It was measured at the commit
named under each table; the figures change with the code, so a later reader
re-measures before relying on one.

**A benchmark record, not a conformance record.** No openEHR specification
governs how a request is executed inside the CDR: the steps marked our own
design are ours to change, and the steps marked required name the text that
binds them.

## The mix

cdr-benchmark v0.19.0 (`docs/guides/architecture.md` at the tag) sends, per
job operation, 77% reads, 15% creates, 6% updates and 2% deletes; 90% of them
on compositions, 8% on directories and 2% on `EHR_STATUS`. A composition is
sent as canonical JSON 40%, FLAT 30%, canonical XML 20% and STRUCTURED 10%.
Beside the job operations run the query workload (template reads, stored
query `PUT`, `GET` and list, EHR-scoped and population AQL), one EHR create
per new-EHR job, and one administrative EHR delete per 100 jobs.

| Operation | Share of job operations |
|---|---|
| composition read | 69.3% |
| composition create | 13.5% |
| directory read | 6.2% |
| composition update | 5.4% |
| composition delete | 1.8% |
| `EHR_STATUS` read | 1.5% |
| directory create, update, delete | 1.8% together |
| `EHR_STATUS` update | 0.5% |

## The common spine

Every request passes these steps. The code path is router, handler, service,
SQL; the file names are where to start reading.

| # | Step | Where | Cost | Verdict |
|---|---|---|---|---|
| 1 | Edge stack: request id, panic catch, timeout, body ceiling | `ferrohealth-server` | CPU only | required: the family's server contract and `reliability.md` (a panic is a clean 500) |
| 2 | One connection per pool per request, given back after the handler | `app/ferroehr-rest/src/router.rs` `one_connection_per_request` | one checkout per pool | our own design (#5879, #5986): keep, the pool rules bind it |
| 3 | Authentication: Basic against the credential cache, or a bearer JWT verified | `extensions/auth` | CPU, no statement on a cache hit | required: ITS-REST `overview.md` §Security, SMART App Launch 2.x |
| 4 | Access decision, confirmed by two indexed reads, one per store (#6048) | `extensions/access/ehr_access.rs` `admit_read` | round trips 2 and 3 of a read | required: EHDS Art 9 and the owner's access rules; the confirmation is our own design (#6048), keep: it is what lets a decision wait on no notification |
| 5 | The operation's own statements | per operation below | per operation | per operation |
| 6 | Render: raw canonical JSON served verbatim; XML, FLAT and STRUCTURED parsed and rendered | `service::composition` | the conversions column below | required: ITS-REST `Resources.md` §Simplified Formats and the canonical XML schema |
| 7 | The access-log entry, written before the response | `access_log::intake` | see §The audit step | required: EHDS Annex II 3.2 (every access), the owner's audit-completeness rule (durable before the response); how it is written is our own design |
| 8 | Background, per entry: the chainer round | `system_log/sender/chainer.rs` | see §The audit step | our own design: deleted by #6242 |

## Per path

The figures are the ceilings of the budget harness
(`app/ferroehr-rest/tests/it/statement_budget/paths.rs`): the measured end
state plus the margins its header states, per request, at `e4818c7a9f`.
Round trips are statements the request sends, nested statements those its
functions and triggers run, WAL the row records it writes as
`pg_walinspect` reads them, with the chain heads left out.

| Path | Conversions | Round trips | Nested | WAL bytes |
|---|---|---|---|---|
| read JSON | 0 | 3 | 2 | 2,677 |
| read XML | 2 | 3 | 2 | 2,677 |
| read FLAT | 2 | 3 | 2 | 2,677 |
| read at time | 0 | 3 | 2 | 2,677 |
| read from the version copy | 0 | 3 | 2 | 2,661 |
| EHR read | 1 | 3 | 2 | 1,877 |
| `EHR_STATUS` read | 0 | 3 | 2 | 2,006 |
| commit JSON | 5 | 9 | 13 | 188,764 |
| commit XML | 5 | 9 | 14 | 189,143 |
| commit FLAT | 5 | 9 | 13 | 189,469 |
| `CONTRIBUTION` commit | 6 | 13 | 14 | 190,142 |
| `EHR_STATUS` commit | 4 | 15 | 11 | 6,021 |
| single-EHR AQL | 2 | 4 | 2 | 4,231 |
| population AQL | 2 | 3 | 2 | 10,764 |
| ITI-81 page | 0 | 3 | 2 | 1,740 |

A read writes no clinical row: all of its WAL is its access-log entry. A
commit's WAL is the composition's version and node rows (#6204, #6229 cut
them) plus its entry and the held records of its reservation.

## The audit step

Before #6241 and #6242, at `e4818c7a9f`, from B1's two-worker smoke run
(#6237) and A1's attribution (#6238):

| Part | Cost |
|---|---|
| the whole audit trail | 0.73 ms of PostgreSQL time per operation, 72% of statement time |
| `audit.access_log_intake_take` | 0.607 ms per operation, 0.667 ms per call, 2,488 WAL bytes and 50 buffers per call |
| `audit.audit_event_chain_move` | 2.5 ms per call, the row digest in a PL/pgSQL trigger |
| an entry's WAL | about 1,270 bytes of heap, 714 of index, 295 of link (159 heap, 64 and 72 index), 280 of intake per statement |

What the request did per entry, and the verdict on each:

| Step | Verdict |
|---|---|
| build the `ACCESS_LOG_ENTRY` and its RFC 8785 canonical form | required: the draft access-log model (`docs/specifications/access-log`) and the content digest the seals cover |
| SHA-256 of the canonical form | keep: the content digest is what a seal covers and a tombstone keeps |
| resolve the search tokens, token sets, render policy and shared part by digest in SQL, per statement | our own design: merge into exact server maps under a generation guard (#6242) |
| render the BALP resource per entry to compute its tokens | our own design: keep for now, measured below |
| one intake row per statement naming the entries | our own design: deleted (#6242), the entry row is the record |
| `FOR KEY SHARE` on each shared part the entries reference | our own design: deleted (#6241), the `retain_until` rule keeps a referenced part |
| a link row per entry, a head row, a checkpoint per second | our own design: deleted (#6242); no text requires a chain (citations on #6239), CRA Annex I Part I(2)(f) is met by seal windows |
| the system record's row digest in a PL/pgSQL trigger | our own design: moves to a content digest computed in Rust at write, sealed the same way (#6242, second pull request) |

After #6241 (merged) and the first pull request of #6242 (#6269), measured
beside the merge base with the same shapes (release build for the times,
the budget harness for the counts):

| Part | Before | After |
|---|---|---|
| entry write per entry, N = 1 (sequential reads) | 0.75 ms (take and chain move) | 0.22 to 0.50 ms |
| entry write per entry, N = 400 (one gathered write) | 26 to 40 µs | 13 to 23 µs |
| entry write per entry, 48 requests in flight | 0.51 to 0.85 ms | 0.11 to 0.56 ms over two runs |
| WAL per entry, read / single-EHR AQL / population AQL / commit | 2,634 / 2,087 / 1,777 / 2,543 | 2,021 / 1,606 / 1,420 / 1,987 |
| nested statements of a read or a query | 2 | 0 |
| nested statements of a care-gated read | 3 | 0 |
| bytes an entry takes on disk | 1,327 to 1,391 | 1,154 to 1,193 |
| statements per request on the audit path | the take plus a share of the chainer's rounds | one insert |

The times come from a shared host that other campaigns loaded; the WAL
bytes and statement counts do not depend on the load. The system chain's
row digest in PL/pgSQL (2.5 ms per move) and the delivery outboxes are the
second pull request of #6242.

## Background work

| Task | Before | After #6242 |
|---|---|---|
| chainer round, access log | per 200 ms of load: take the head, read the intake, write links, outbox rows, delete the intake, move the head, sign a checkpoint a second | deleted; the system chain's round stays until the second pull request |
| frontier round | none | once a second: two statements, no rows |
| sealer | none | once a minute: one read of the window's ids and digests, one seal row, one signature |
| verification | walks the links in paced steps | walks the windows in paced steps |
| delivery (syslog, FHIR Feed) | outbox rows written per entry in the chainer's round | outbox rows written per frontier round, only while a sink is configured; a delivery cursor for both chains follows in the second pull request of #6242 |
