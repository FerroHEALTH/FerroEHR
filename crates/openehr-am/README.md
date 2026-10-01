# openehr-am

**openEHR AM — the Archetype Model for Rust**, generated deterministically
from the official openEHR machine-readable BMM meta-model. Both extant AM
generations ship side by side as generation modules, exactly as the openEHR
architecture mandates: `v1_4` (AM 1.4.0, ADL/AOM 1.4) and `v2_4` (AM 2.4.0,
ADL 2 / AOM 2.4).

## What it provides

- `v1_4` — the AOM 1.4 constraint model (`C_COMPLEX_OBJECT`, `C_PRIMITIVE_*`,
  the openEHR archetype profile: `C_DV_QUANTITY`, `C_CODE_PHRASE`, …) as used
  by ADL 1.4 archetypes and OPT 1.4 operational templates.
- `v2_4` — the AOM 2 model: archetypes, templates, the persistence (`P_`)
  classes, rules/assertion expression classes, and the AOM profile. This is
  the **current** generation — the one the crate prelude
  (`openehr_am::prelude`) re-exports; `v1_4` is reached by its full module
  path (`openehr_am::v1_4::…`, or its own `v1_4::prelude`).

For an ADL 2 *engine* (parser, validation, flattener, OPT2 generation, ADL
1.4→2 conversion) over this model, see the companion crate `openehr-adl`.

## Generated code — do not edit

This crate is **entirely generated** — every file carries a `// @generated`
header — emitted deterministically by
[`openehr-codegen`](https://github.com/FerroHEALTH/FerroEHR/tree/main/tools/openehr-codegen)
from the vendored openEHR BMM meta-model. Changes belong in the emitter, never
in the generated output; hand-written spec behaviour, when a class needs it,
goes in a sibling `*_impl.rs` file the generator never rewrites. The AOM 2
*semantics* (parsing, validation, flattening) live in `openehr-adl`, not here.

## Versioning

The package version is the crate's **own independent SemVer line** — it
tracks this implementation's code and moves freely with fixes and
improvements, never with the vendored openEHR specification.

The implemented spec version is a **per-generation** datum, carried by the
emitted `Generation` enum — the crate's only pin authority (there is no
crate-level or module-level spec-version constant). `Generation::default()`
is the current generation (`V2_4`), `Generation::spec_version()` is a
`const fn` returning that generation's openEHR version (`"1.4.0"` / `"2.4.0"`),
and `Display`/`FromStr` round-trip the generation-module token (`"v2_4"`).

## Minimum supported Rust version

Rust 1.97 (edition 2024).

## License

Apache License 2.0 ([`LICENSE-APACHE-2.0`](LICENSE-APACHE-2.0)) for the whole
package. The `openehr-*` crates are Apache-2.0 so any Rust project can use
them; the Business Source License of the FerroEHR application does not apply to
them.

- The Rust code and hand-written parts are the project's own, under Apache-2.0.
- The package embeds material derived from the official openEHR
  machine-readable specification artifacts, which openEHR publishes under
  Apache-2.0 ([`LICENSE-APACHE-2.0`](LICENSE-APACHE-2.0)) — the generated types and their doc text derive from the vendored AM BMM schemas.

## Part of FerroEHR

This crate is the specification layer of [FerroEHR](https://github.com/FerroHEALTH/FerroEHR), a pure-Rust,
openEHR-spec-conformant Clinical Data Repository (ITS-REST 1.1.0 + AQL 1.1 on
PostgreSQL 18). The crates are usable standalone; FerroEHR is the reference
consumer.
