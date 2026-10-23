# Viewer test fixtures

`minimal_evaluation.opt` is the viewer suite's shared seed template
(template id `minimal_evaluation.en.v1`), a copy of the openEHR SDK test
template also vendored at `crates/openehr-its/tests/fixtures/sdk/` — copied
rather than reached cross-crate so the owning crate can move its fixtures
without breaking this suite (#2616).

Consumers: `tests/it/e2e_browse.rs`, `tests/it/e2e_docs_shots.rs` and
`scripts/ui-e2e.sh`.

`cdr/recording.json` is a recording of the exchanges between the viewer and a
running FerroEHR 5.3 (the quickstart stack seeded as `scripts/ui-e2e.sh` seeds
it), and `cdr/manifest.json` names the ids it holds. `tests/it/ssr_app.rs`
replays it as the CDR, so every screen is drawn through the real service in
process with no external server. A screen that reads a request the recording
lacks fails there, naming the request: record it again against a stack. The
one hand edit is the dashboard trend exchange, whose AQL moved from the
context start time to the commit time; its rows were kept.
