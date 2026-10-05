# Provenance

- Upstream: https://github.com/FerroHEALTH/FerroPULSE (private; Cadasto B.V., the copyright
  holder of this repository too)
- Ref: commit `6e136a96abcda45d6bceb1c75ba4543a5119254c`
- Files: `schemas/report-envelope.v1.json` and
  `schemas/ferroehr-metrics.v1.json` (JSON Schema 2020-12), and
  `schemas/examples/` (the contract's own valid and invalid examples)
- Licence: upstream declares `LicenseRef-Proprietary`; the copyright holder is
  Cadasto B.V., which also holds this repository, and `REUSE.toml` declares
  this subtree under the repository's BUSL-1.1
- Fetched by: `scripts/vendor/ferropulse-schemas.sh`; never hand-edit, re-run
  the script to update (.claude/rules/vendored-corpora.md)
- Consumer: `app/ferroehr/tests/it/usage_report.rs` validates every payload
  the usage report client renders against the two schemas, and checks the
  validator against the examples first
