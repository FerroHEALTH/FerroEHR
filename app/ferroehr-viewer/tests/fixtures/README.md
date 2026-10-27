# Viewer test fixtures

`minimal_evaluation.opt` is the viewer suite's shared seed template
(template id `minimal_evaluation.en.v1`), a copy of the openEHR SDK test
template also vendored at `crates/openehr-its/tests/fixtures/sdk/` — copied
rather than reached cross-crate so the owning crate can move its fixtures
without breaking this suite (#2616).

Consumers: `tests/it/e2e_browse.rs`, `tests/it/e2e_docs_shots.rs` and
`scripts/ui-e2e.sh`.

`cdr/recording.json` is a recording of the exchanges between the viewer and the
FerroEHR of the checkout it was taken in, and `cdr/manifest.json` names the ids
it holds. `tests/it/ssr_app.rs` replays it as the CDR, so every screen is drawn
through the real service in process with no external server. A screen that
reads a request the recording lacks fails there, naming the request.

`scripts/viewer-record.sh` takes the recording again: it starts the checkout's
`ferroehr` over a throwaway PostgreSQL, seeds it over ITS-REST (the seed of
`scripts/ui-e2e.sh` plus the subject, tags, directory, parties and stored query
the screens read), runs the `ssr_app` tests with the recorder of
`tests/it/cdr_record.rs` forwarding to it, and writes both files. Every id,
version signature and instant the run stamped is replaced by a stand-in, so a
second recording of the same checkout writes the same bytes. Never edit the
recording by hand: record it again.
