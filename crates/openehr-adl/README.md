# openehr-adl

**A hand-written openEHR ADL 2.4 engine for Rust**: ADL2/cADL/ODIN parsing,
AOM 2 validation, specialisation flattening, operational-template (OPT 2)
generation, and ADL 1.4 → ADL 2 conversion — over the generated
`openehr_am::v2_4::aom2` model.

## What it provides

- **Parser**: ADL 2 archetype text (cADL definition sections, ODIN metadata /
  terminology sections) into typed `openehr_am::v2_4::aom2` values, with
  precise, typed syntax errors.
- **Validation**: the AOM 2 validity catalogue (VxxXX codes) over parsed
  archetypes.
- **Flattener**: specialisation flattening and template expansion.
- **OPT 2**: operational template generation from a flattened archetype set.
- **ADL 1.4 → 2 conversion** for legacy archetype libraries.

The grammar authority is the vendored openEHR ADL 2.4 specification text and
its normative grammars; the parser is a hand-written native-Rust
recursive-descent reader over the shared openEHR token stream
(`openehr_lang::v1_1::lexer`), with no ANTLR runtime.

## Versioning

The package version is the crate's **own independent SemVer line** — it
tracks this implementation's code and moves freely with fixes and
improvements, never with the vendored openEHR specification. The implemented
spec version is always available at runtime as the crate-level constant
`openehr_adl::SPEC_VERSION` (`"2.4.0"` — ADL/AOM), independent of the package
version.

## Minimum supported Rust version

Rust 1.97 (edition 2024).

## License

Business Source License 1.1 ([`LICENSE`](LICENSE), SPDX `BUSL-1.1`), the
licence of the FerroEHR application: all non-production use is free, production
use is free for Non-Commercial Purposes, and any other production use, hosting
for third parties or distribution for a fee needs a commercial licence from the
Licensor, Cadasto B.V.: write to [info@cadasto.com](mailto:info@cadasto.com)
or use <https://www.cadasto.com/contact/>. Each version becomes Apache License
2.0 four years after it is published. The crate is hand-written; the openEHR
specification text it implements is the authority but is not embedded in the
package. Versions up to 0.0.59 were published under Apache-2.0 and keep it.

## Part of FerroEHR

This crate is the archetype-tooling layer of [FerroEHR](https://github.com/FerroHEALTH/FerroEHR), a pure-Rust,
openEHR-spec-conformant Clinical Data Repository (ITS-REST 1.1.0 + AQL 1.1 on
PostgreSQL 18). The crates are usable standalone; FerroEHR is the reference
consumer.
