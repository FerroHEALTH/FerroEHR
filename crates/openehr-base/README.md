# openehr-base

**openEHR BASE — foundation and base types for Rust**, generated
deterministically from the official openEHR machine-readable BMM meta-model.
Two BASE component versions ship side by side as generation modules: `v1_2`
(BASE 1.2.0, the latest released) and `v1_3` (BASE 1.3.0, the development
line).

## What it provides

- Two generation modules, each emitting its component version completely.
  `v1_3` is the **current** generation — the one the crate prelude
  (`openehr_base::prelude`) re-exports; `v1_2` is reached by its full module
  path (`openehr_base::v1_2::…`, or its own `v1_2::prelude`).
- The complete BASE component type set: foundation types (intervals,
  ISO-8601 date/time types, `Uri`, terminology-neutral primitives) and base
  types (identification — `OBJECT_VERSION_ID`, `ARCHETYPE_ID`, `HIER_OBJECT_ID`
  and peers — plus `DV_`-supporting base classes and definitions).
- `serde_support` — the small hand-written runtime shared by the emitted
  canonical-JSON `serde` implementations of every openEHR spec crate
  (`_type`-first self-tagging, strict reading: undeclared and duplicate keys
  are refused).
- `containers` — the structural container bounds the RM relies on, notably
  `NonEmptyVec<T>`, which makes a spec-mandated `1..*` list's emptiness
  unrepresentable rather than merely rejected.
- Validated constructors: types with BMM invariants construct through
  `new() -> Result`, so invalid spec values are unrepresentable.

`serde_support`, `containers` and `validate` are cross-generation runtime and
live at the crate root, outside the generation modules.

Every other `openehr-*` crate builds on this one.

## Generated code — do not edit

Every type file carries a `// @generated` header. The crate is emitted
deterministically by [`openehr-codegen`](https://github.com/FerroHEALTH/FerroEHR/tree/main/tools/openehr-codegen)
from the vendored openEHR BMM meta-model; hand-written spec behaviour
(invariants, spec functions) lives in sibling `*_impl.rs` files the generator
never rewrites. Changes belong in the emitter, never in the generated output.

## Versioning

The package version is the crate's **own independent SemVer line** — it
tracks this implementation's code and moves freely with fixes and
improvements, never with the vendored openEHR specification.

The implemented spec version is a **per-generation** datum, carried by the
emitted `Generation` enum — the crate's only pin authority (there is no
crate-level or module-level spec-version constant). `Generation::default()`
is the current generation (`V1_3`), `Generation::spec_version()` is a
`const fn` returning that generation's openEHR version (`"1.2.0"` / `"1.3.0"`),
and `Display`/`FromStr` round-trip the generation-module token (`"v1_3"`).

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
  Apache-2.0 ([`LICENSE-APACHE-2.0`](LICENSE-APACHE-2.0)) — the generated types and their doc text derive from the vendored BASE BMM schema.

## Part of FerroEHR

This crate is the specification foundation layer of [FerroEHR](https://github.com/FerroHEALTH/FerroEHR), a pure-Rust,
openEHR-spec-conformant Clinical Data Repository (ITS-REST 1.1.0 + AQL 1.1 on
PostgreSQL 18). The crates are usable standalone; FerroEHR is the reference
consumer.
