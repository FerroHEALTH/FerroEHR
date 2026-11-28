# Viewer test fixtures

`minimal_evaluation.opt` is the viewer suite's shared seed template
(template id `minimal_evaluation.en.v1`), a copy of the openEHR SDK test
template also vendored at `crates/openehr-its/tests/fixtures/sdk/` — copied
rather than reached cross-crate so the owning crate can move its fixtures
without breaking this suite (#2616).

Consumers: `tests/it/e2e_browse.rs`, `tests/it/e2e_docs_shots.rs`,
`tests/it/e2e_paging.rs`, `tests/it/e2e_compositions_filters.rs` and the demo
seed (`scripts/sandbox/seed/manifest.json`).

`extract/foreign-ehr.json` is a synthetic EHR Extract, as an export lists
it, of an EHR another system (`hospital-b.example`) wrote: one `EHR_STATUS`
with no subject, committed by a party of that system. `tests/it/e2e_extract.rs`
imports it, since the CDR refuses an extract whose committers are its own
parties or logins.

`cdr/recording.json` is a recording of the exchanges between the viewer and the
FerroEHR of the checkout it was taken in, and `cdr/manifest.json` names the ids
it holds. `tests/it/ssr_app.rs` replays it as the CDR, so every screen is drawn
through the real service in process with no external server. A screen that
reads a request the recording lacks fails there, naming the request.

`scripts/viewer-record.sh` takes the recording again: it starts the checkout's
`ferroehr` over a throwaway PostgreSQL, seeds it with the one demo seed
(`scripts/sandbox/reseed.sh`, as `scripts/ui-e2e.sh` runs it), runs the
`ssr_app` tests with the recorder of `tests/it/cdr_record.rs` forwarding to
it one screen at a time, and writes both files. Every id, digest and instant
the run stamped, and every date the seed counted from the day it ran, is
replaced by a stand-in, so a second recording of the same checkout writes the
same bytes in all but two places the CDR's own timing decides: the windows it
seals the access log in, and the order in which its access log and system
records list the reads one screen makes at once. Never edit the recording by
hand: record it again.
