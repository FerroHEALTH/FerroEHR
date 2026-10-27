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
lacks fails there, naming the request: record it again against a stack. Two
exchanges carry hand edits. The dashboard trend exchange's AQL moved from the
context start time to the commit time, and its rows were kept. The
configuration exchange (`GET …/admin/config`) carries the `[audit.store]`
verify intervals of 5.5 (`verify_interval_seconds` 3600 and
`full_verify_interval_seconds` 604800) and no `usage_report.slow_aql_ms`; the
rest of that answer is still the 5.3 shape.
