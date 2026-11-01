# Provenance

- Upstream: https://github.com/Vernum-Projecten/Veredictum
- Ref: tag `v0.3.3`, commit `bfcb62d0514eb065396f3786a8476de306fd13b3`
- Files: `catalogue/cases/_setup/content_templates_adl2.arazzo.yaml` (the ADL 2
  operational templates, one upload workflow each) and the 26
  `catalogue/cases/content/adl2_*.arazzo.yaml` documents (each commit step's
  request body is a composition against one of those templates), with the
  upstream `LICENSE` and `NOTICE`
- Licence: Apache-2.0 (the upstream `LICENSE`, with its `NOTICE` carried per
  its section 4(d)); `REUSE.toml` declares this subtree under it
- Fetched by: `scripts/vendor/veredictum-adl2-content.sh`; never hand-edit,
  re-run the script to update (.claude/rules/vendored-corpora.md)
- Consumers: `crates/openehr-sdt/tests/it/adl2_content_constraints.rs` (the
  archetype-conformance pass over every workflow) and
  `app/ferroehr/tests/it/adl2_content_commit.rs` (the canonical, FLAT and
  STRUCTURED commit paths)
