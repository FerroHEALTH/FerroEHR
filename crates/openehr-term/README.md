# openehr-term

**openEHR TERM 3.1.0 for Rust** — the terminology data model, generated
from the official openEHR machine-readable BMM meta-model, plus a hand-written
loader over the official openEHR terminology XML, which is embedded in the
package (English, Spanish, Japanese, Portuguese, Chinese).

## What it provides

- The generated TERM data classes (`TERMINOLOGY`, `CODE_SET`, concept/group
  structures), emitted under the single generation module `v3_1` (TERM 3.1.0)
  and re-exported by the crate prelude (`openehr_term::prelude`).
- `bundle` — a zero-I/O, `include_str!`-embedded bundle of the official
  openEHR terminology XML releases (`assets/`, provenance-stamped), parsed
  once and served through typed lookups: terminology groups, code sets,
  rubrics per language.
- The support-terminology queries RM invariant enforcement needs (e.g. "is
  this code in group X"), as consumed by `openehr-rm`'s validation layer.
- `measurement` — the `MEASUREMENT_SERVICE.is_valid_units_string` UCUM
  syntax validator.

## Generated code — do not edit

The types under the `v3_1` generation module each carry a `// @generated`
header and are emitted deterministically by
[`openehr-codegen`](https://github.com/FerroHEALTH/FerroEHR/tree/main/tools/openehr-codegen)
from the vendored openEHR BMM meta-model; changes to them belong in the
emitter, never in the generated output. The terminology content is a different
matter: the BMM declares only the data classes, so the embedded XML assets and
everything that reads them (`bundle`, `measurement`) are hand-written and
edited normally.

## Versioning

The package version is the crate's **own independent SemVer line** — it
tracks this implementation's code and moves freely with fixes and
improvements, never with the vendored openEHR specification.

The implemented spec version is a **per-generation** datum, carried by the
emitted `Generation` enum — the crate's only pin authority (there is no
crate-level or module-level spec-version constant). This crate has one
generation: `Generation::default()` is `V3_1` and
`Generation::V3_1.spec_version()` is `"3.1.0"` (a `const fn`);
`Display`/`FromStr` round-trip the generation-module token (`"v3_1"`).

## Minimum supported Rust version

Rust 1.97 (edition 2024).

## License

Licensed as **Apache-2.0 AND CC-BY-SA-3.0**. The `openehr-*` crates are
Apache-2.0 so any Rust project can use them; the Business Source License of the
FerroEHR application does not apply to them.

- The Rust code and hand-written parts are the project's own, under the Apache
  License 2.0 ([`LICENSE-APACHE-2.0`](LICENSE-APACHE-2.0)).
- The generated data classes and their doc text derive from the official openEHR
  machine-readable specification artifacts, which openEHR publishes under
  Apache-2.0 ([`LICENSE-APACHE-2.0`](LICENSE-APACHE-2.0)).
- The official openEHR terminology XML in `assets/` — the five language
  bundles, the external-terminology index and the property/unit data — is
  redistributed verbatim with attribution under the CC-BY-SA 3.0 of its
  upstream repository
  ([`LICENSE-CC-BY-SA-3.0`](LICENSE-CC-BY-SA-3.0);
  <https://github.com/openEHR/specifications-TERM/blob/master/LICENSE>). If you
  redistribute this crate, that data travels under CC-BY-SA 3.0.

## Part of FerroEHR

This crate is the terminology layer of [FerroEHR](https://github.com/FerroHEALTH/FerroEHR), a pure-Rust,
openEHR-spec-conformant Clinical Data Repository (ITS-REST 1.1.0 + AQL 1.1 on
PostgreSQL 18). The crates are usable standalone; FerroEHR is the reference
consumer.
