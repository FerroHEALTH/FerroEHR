# Provenance

- Upstream: https://github.com/FerroHEALTH/FerroPULSE (private; Cadasto B.V., the copyright
  holder of this repository too)
- Ref: commit `095ccb5718a1b5b0adb088f4d8cb143ba3f5f101`
- Files: `schemas/report-envelope.v2.json` (JSON Schema 2020-12; the
  `status` and `metrics` objects are `$defs` inside it), and
  `schemas/examples/report-envelope.v2/` (the contract's own valid and
  invalid examples)
- Licence: upstream declares `LicenseRef-Proprietary`; the copyright holder is
  Cadasto B.V., which also holds this repository, and `REUSE.toml` declares
  this subtree under the repository's BUSL-1.1
- Fetched by: `scripts/vendor/ferropulse-schemas.sh`; never hand-edit, re-run
  the script to update (.claude/rules/vendored-corpora.md)
- Consumer: `app/ferroehr/tests/it/usage_report.rs` validates every payload
  the usage report client renders against the schema, and checks the
  validator against the examples first
