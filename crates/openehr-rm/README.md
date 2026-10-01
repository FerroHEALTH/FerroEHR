# openehr-rm

**openEHR RM — the Reference Model for Rust**, generated
deterministically from the official openEHR machine-readable BMM meta-model.
This is the domain model of an openEHR system: `COMPOSITION`, `EHR_STATUS`,
`OBSERVATION`, the data structures and data types, and the change-control
(versioning) classes. Two RM component versions ship side by side as
generation modules: `v1_1` (RM 1.1.0, the latest released) and `v1_2`
(RM 1.2.0, the development line).

## What it provides

- Two generation modules, each emitting its component version completely.
  `v1_2` is the **current** generation — the one the crate prelude
  (`openehr_rm::prelude`) re-exports; `v1_1` is reached by its full module
  path (`openehr_rm::v1_1::…`, or its own `v1_1::prelude`). Each generation
  resolves against its own released BASE pairing: `v1_2` against
  `openehr_base::v1_3`, `v1_1` against `openehr_base::v1_2` (the RM 1.1.0
  BMM's own `includes` names BASE 1.2.0).
- The complete class set per generation, package-mirrored (`ehr`,
  `composition`, `data_structures`, `data_types`, `common` incl. change
  control, `support`, `demographic`, `integration`, `ehr_extract`), with
  closed subtype sets as Rust enums and recursion boxed.
- Emitted **manual** canonical-JSON `serde` implementations (`json_serde.rs`)
  — `_type`-first self-tagging in BMM declaration order, strict reader — no
  derives, no serde attributes on spec types.
- `validate` (per generation, e.g. `v1_2::validate`) — machine-classified RM
  invariant cores generated from the BMM invariant expressions, plus
  terminology-backed invariant enforcement via `openehr-term`.
- `model` (per generation, e.g. `v1_2::model`) — the static RM attribute/type
  model (attribute → types, multiplicity, abstract → concrete descendant
  sets), the oracle an AQL planner or path validator needs, generated rather
  than reflected.

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
is the current generation (`V1_2`), `Generation::spec_version()` is a
`const fn` returning that generation's openEHR version (`"1.1.0"` / `"1.2.0"`),
and `Display`/`FromStr` round-trip the generation-module token (`"v1_2"`).

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
  Apache-2.0 ([`LICENSE-APACHE-2.0`](LICENSE-APACHE-2.0)) — the generated types and their doc text derive from the vendored RM BMM schema.

## Part of FerroEHR

This crate is the specification layer of [FerroEHR](https://github.com/FerroHEALTH/FerroEHR), a pure-Rust,
openEHR-spec-conformant Clinical Data Repository (ITS-REST 1.1.0 + AQL 1.1 on
PostgreSQL 18). The crates are usable standalone; FerroEHR is the reference
consumer.
