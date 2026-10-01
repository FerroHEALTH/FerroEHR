# openehr-lang

**openEHR LANG for Rust** — the BMM (Basic Meta-Model) object model,
generated from the official machine-readable schemas, plus hand-written
readers for the component's notations. Two LANG component versions ship side
by side as generation modules: `v1_0` (LANG 1.0.0, the latest released) and
`v1_1` (the 1.1.0 development line).

## What it provides

- `v1_1` — the **current** generation, the one the crate prelude
  (`openehr_lang::prelude`) re-exports. LANG 1.1.0 publishes two
  machine-readable specification **units**, and both are emitted completely,
  side by side inside this one generation:
  - the STABLE, tool-implemented BMM v2.x model — `bmm` (incl. the
    `rm_access` schema-repository facade), its `bmm_persistence` P_BMM form,
    and the `beom` expression object model — which is what the generation's
    prelude carries;
  - the PAUSED BMM3 model — `bmm3` (entities, features, literal values,
    expressions, statements) — reachable by its full module path
    (`openehr_lang::v1_1::bmm3::…`) only, because 18 class names occur in
    both units with materially different shapes.
- `v1_0` — the released LANG 1.0.0 generation, emitted faithfully from that
  release's own BMM (`bmm`, `bmm_persistence`, `obsolete_elom`) and reached
  by its full module path (`openehr_lang::v1_0::…`, or its own
  `v1_0::prelude`).
- `odin` — a hand-written ODIN instance parser (`logos` + `chumsky`, from the
  official `odin.g4`/`odin_values.g4` grammars), producing insertion-ordered
  typed values; present in both generations, each reading exactly the syntax
  its own release defines.
- `bel` and `el` — hand-written readers for the BEL expression syntax and the
  Expression Language, over the one shared `lexer` token superset. `v1_0`
  carries the ODIN reading alone: LANG 1.0.0 publishes no expression-language
  grammar.
- `v1_1::bmm_persistence::loader::load_model` — the P_BMM pipeline: ODIN
  schema text → inclusion closure → a resolved `BMM_MODEL`. This is how a
  BMM-consuming tool reads openEHR reference-model schemas.

## Generated code — do not edit

Every type file carries a `// @generated` header. The crate is emitted
deterministically by [`openehr-codegen`](https://github.com/FerroHEALTH/FerroEHR/tree/main/tools/openehr-codegen)
from the vendored openEHR BMM meta-model; hand-written spec behaviour
(invariants, spec functions) lives in sibling `*_impl.rs` files, and the
notation readers (`odin`, `bel`, `el`, `lexer`, the P_BMM schema reader) are
hand-written modules — the generator never rewrites either. Changes belong in
the emitter, never in the generated output.

## Versioning

The package version is the crate's **own independent SemVer line** — it
tracks this implementation's code and moves freely with fixes and
improvements, never with the vendored openEHR specification.

The implemented spec version is a **per-generation** datum, carried by the
emitted `Generation` enum — the crate's only pin authority (there is no
crate-level or module-level spec-version constant). `Generation::default()`
is the current generation (`V1_1`), `Generation::spec_version()` is a
`const fn` returning that generation's openEHR version (`"1.0.0"` / `"1.1.0"`),
and `Display`/`FromStr` round-trip the generation-module token (`"v1_1"`).

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
  Apache-2.0 ([`LICENSE-APACHE-2.0`](LICENSE-APACHE-2.0)) — the generated types and their doc text derive from the vendored LANG BMM schemas.

## Part of FerroEHR

This crate is the meta-model layer of [FerroEHR](https://github.com/FerroHEALTH/FerroEHR), a pure-Rust,
openEHR-spec-conformant Clinical Data Repository (ITS-REST 1.1.0 + AQL 1.1 on
PostgreSQL 18). The crates are usable standalone; FerroEHR is the reference
consumer.
