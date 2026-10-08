# Viewer test fixtures

`minimal_evaluation.opt` is the viewer suite's shared seed template
(template id `minimal_evaluation.en.v1`), a copy of the openEHR SDK test
template also vendored at `crates/openehr-its/tests/fixtures/sdk/` — copied
rather than reached cross-crate so the owning crate can move its fixtures
without breaking this suite (#2616).

Consumers: `tests/it/e2e_browse.rs`, `tests/it/e2e_docs_shots.rs`, the
`template_detail` unit tests
(`src/pages/template_detail.rs`), and `scripts/ui-e2e.sh`.
