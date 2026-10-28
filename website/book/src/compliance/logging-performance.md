# The logging component's intended performance

EHDS Annex II 1.1 asks that the harmonised software components "achieve the
performance intended by its manufacturer" and be suitable for their intended
purpose in normal conditions of use (Regulation (EU) 2025/327,
[`docs/law/eu/ehds/text.html`](https://github.com/FerroHEALTH/FerroEHR/blob/main/docs/law/eu/ehds/text.html)).
Annex III point 2 asks the technical documentation for the system in place to
evaluate that performance. This page states the performance Cadasto B.V.
intends for FerroEHR's European logging software component, as claims a test
can check, and the test that checks them. The Regulation names no method and
no figure, so both are FerroEHR's own design.

<!-- toc -->

> [!WARNING]
> A met claim shows what the software did on the run that measured it. It
> does not show that the component meets Annex II, which no conformity
> assessment has examined.

## The claims

The component is the access log the [hazard log](hazard-log.md) assesses:
every access through the API is recorded, by `ehds-logging` and `ihe-atna`
through `ferroehr::system_log`, in the local store, and retrieved through IHE
ITI-81. The claims hold under the deployment posture the
[instructions for use](https://ferrohealth.eu/docs/ehds/instructions-for-use.html)
set for real data: the access log on, the local store on, and
`[audit] fail_mode = "closed"`.

| Claim | Stated performance | How it is checked |
|---|---|---|
| Completeness | Every composition create and read has exactly one access record, and every AQL execution at least one (one per EHR it served). | Measured on every run: the access records of the measured window are counted through ITI-81 per operation id and compared with the operations that ran. |
| Loss under outage | No access succeeds without its record. A record the queue or the local store cannot take refuses the request with `503`, and a record owed to a FHIR Audit Record Repository that is down stays owed and is delivered when it returns. | Deterministic tests, not a load measurement: `app/ferroehr-rest/tests/it/audit_e2e.rs` (`fail_closed_returns_503_when_channel_full`, `fail_closed_503_carries_openehr_error_body_and_retry_after`), `app/ferroehr/tests/it/access_event_fail_closed.rs`, and `app/ferroehr/tests/it/audit_feed.rs` (`arr_outage_leaves_rows_pending_then_delivers_on_recovery`). |
| Latency added | With the access log on, the median p50 and p95 request latency are at most 125 % of those with it off. | Measured: the same workload with the log off and on, on one machine. |
| Throughput kept | With the access log on, the median throughput is at least 80 % of that with it off. | Measured, as above. |
| Retrieval | A page of 50 access records retrieved through ITI-81 from the measured window takes at most 250 ms at p95. | Measured: 50 paged retrievals after each run. |

"Normal conditions of use" for these figures are the load of the release
performance guard: a store seeded with 20 EHRs of 10 compositions each, then
2,000 operations over 8 concurrent workers, of every 20 six composition
creates, eight composition reads and six AQL queries
([`tools/loadtest`](https://github.com/FerroHEALTH/FerroEHR/tree/main/tools/loadtest)).
The intended-purpose statement names no load, so this one is chosen to be the
one every release is already measured under. The ratios compare two runs on
one machine, so they hold across machines in a way an absolute latency would
not; the retrieval figure is absolute and generous for that reason.

## The test

[`scripts/perf/audit.sh`](https://github.com/FerroHEALTH/FerroEHR/blob/main/scripts/perf/audit.sh)
starts the quickstart on fresh volumes, three times with the access log off
and three times with it on and failing closed, alternating the order. The
load test runs the workload against each; with the log on it waits for the
asynchronous log to settle after the seed, opens the measured window, and
after the measured phase counts the window's records per operation id (the
ITI-81 `subtype` and `_count=0`) until two readings a second apart agree, then
times the paged retrievals.
[`scripts/perf/audit-verdict.sh`](https://github.com/FerroHEALTH/FerroEHR/blob/main/scripts/perf/audit-verdict.sh)
compares the medians with the claims above and exits non-zero when one is not
met; its `--self-test` holds each budget to its edge.

The release pipeline runs it on the release candidate in its performance leg.
A claim not met refuses the release, and the summary is attached to the
release as `ferroehr-<tag>-logging-performance.md`, which is the latest result
for that release. The weekly performance guard runs it on `main`.

To run it yourself against an image:

```shell
cargo build --release -p loadtest
FERROEHR_PORT=8080 bash scripts/perf/audit.sh ghcr.io/ferrohealth/ferroehr:<tag> \
  ghcr.io/ferrohealth/ferroehr-postgres:<tag> perf-audit
```

## What it does not measure

- Records over syslog or the FHIR feed are not timed: both send from the
  stored record, after the request, so they add no latency to it.
- The store's size over years is not loaded: the retrieval figure holds for
  the store a run leaves. ITI-81 answers through a token index, so a search
  reads an index rather than every record, but no run measures a store of
  millions of records.
- Access made straight against the database, outside the server, is not
  recorded at all (hazard H1).
