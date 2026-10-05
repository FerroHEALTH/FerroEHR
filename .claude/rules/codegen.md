---
paths: ["tools/openehr-codegen/**", "crates/openehr-base/**", "crates/openehr-rm/**", "crates/openehr-am/**", "crates/openehr-lang/**", "crates/openehr-term/**", "crates/openehr-its/src/**/generated/**"]
---

# Code generation

The openEHR **spec + serialization + REST-contract layer is generated**, not
hand-written. `openehr-codegen` reads the vendored specs and emits Rust:

- `emit` → the spec crates from BMM: `openehr-base`, `openehr-rm`, `openehr-am`
  (`v1_4`+`v2_4`), `openehr-term`, `openehr-lang`.
- `emit-xml` → canonical-XML `ToXml`/`FromXml` for the RM/BASE types into
  `openehr-its/src/xml/generated/` (from the vendored XSDs + the BMM field model).
- `emit-rest` → the ITS-REST contract (DTOs + `#[async_trait]` server traits +
  per-group clients + routes) into `openehr-its/src/rest/generated/` (from the vendored `-codegen` OAS).
- `emit-rm-model` → the static RM attribute/type model (attributes+types,
  multiplicity, descendant/ancestor sets, structure classification) into
  `openehr-rm/src/model/` — the AQL planner's oracle. `emit` already
  emits it as part of `openehr-rm`; this target refreshes just that subtree.
- `emit-opt` → the OPT 1.4 model + XML codec (`opt14`) into `openehr-its`.
- `emit-aom2` → both AOM2 archetype XML codecs into `openehr-its`: `aom2` (the
  persistent `P_AOM` form) + `aom2_model` (the AOM model form).

## The three hard rules

**The generator emits the COMPLETE model — never minimize (owner hard
rule, 2026-07-19).** Everything the vendored inputs (and any legitimate
emission closure over them) define gets emitted in full, mirrored to its
source package path — including classes nothing consumes yet; future need
is the point. Forbidden moves: narrowing a schema merge to shrink a
closure, pruning "unrelated" classes out of an emission, suppressing
generated files to quiet a diff, or restoring-around a generation defect
instead of fixing it. If an emission change pulls in a large new class
set, that is the CORRECT outcome — emit it all and let the diff be big.

**A generated-model gap is fixed in the GENERATOR, never worked around in a
consumer (owner hard rule, 2026-07-19).** When a consumer (`ferroehr-*`,
`openehr-adl`, `openehr-its` runtime, …) hits a generated shape that is
wrong or insufficient versus the vendored spec/BMM — a missing subtype
seam, a too-narrow field, a closed enum a downstream component's BMM
extends — the fix is an emitter/override change + regeneration. Never a
shadow type, duplicate model, adapter layer, placeholder value, or
"temporary" local representation: that silently forks the spec model the
generated crates exist to guarantee. Cross-component subtype extension
(e.g. AM classes extending LANG's expression classes) is re-opened by the
emitter at the DOWNSTREAM crate boundary (an extender-level enum composing
the upstream variants + the downstream leaves) — upstream crates never
gain downstream variants (dependency arrows point one way). If the emitter
fix is large, register a tracker issue; the workaround is still forbidden.
Existing workarounds get a removal issue on discovery.

**Never hand-edit a `// @generated` file.** To change generated output, edit the
emitter (`tools/openehr-codegen/src/`, the `load/`→`analyze/`→`plan/`→`render/`
pipeline stages; decision maps live in `plan/mod.rs`, text producers in
`render/`) or a hand-written `*_impl.rs` sibling (spec behaviour), then regenerate.
A hand edit is silently overwritten on the next `emit` and fails the CI
`codegen-drift` job.

## Workflow

1. Change the emitter/override, not the output.
2. Regenerate: `cargo run -p openehr-codegen -- emit && … emit-xml && … emit-rest`
   (or use the `/regen-codegen` skill).
3. `cargo build`/`clippy`/`nextest` the affected crates + `openehr-its` gates.
4. Commit the emitter change **and** the regenerated output together (the
   drift-check requires them in sync).

## Notes

- The generated crates are idiomatic + clippy-clean *by construction*; a
  lint exception inherent to verbatim spec docs is declared once in the generated
  `lib.rs`/file header, never per-hand-edit.
- Vendored inputs: BMM at `openehr-codegen/vendor/bmm/`, XSD/OAS/JSON schemas at
  `openehr-its/schemas/` + `openehr-its/vendor/rest-oas/` (each with `PROVENANCE.md`).
  The spec *text* at `docs/specs/openehr/` is read-only reference for humans/agents
  (spec-adherence.md) — never a codegen input, never hand-edited.
- Hand-written spec crates (NOT generated): `openehr-term` bundle/assets,
  `openehr-its` runtime (`xml/runtime.rs`, `rest/runtime.rs`), `openehr-sdt`
  (Simplified Formats + RM-instance validation), `openehr-query` (AQL parser). These follow
  `rust-style.md`.

## Generation modules

**Generation modules (owner design 2026-08-05, #1936/#1941):** every
generated BMM crate carries its generations as version-named top modules
(`openehr_base::v1_2`/`v1_3`, `openehr_rm::v1_1`/`v1_2` — the released
generations emitted beside the development pins, #1942 —,
`openehr_lang::v1_0`/`v1_1` — the released 1.0.0 generation (emitted
faithfully from the released BMM, defects verbatim, #1946) beside the
development generation whose SPECIFICATION UNITS `bmm`+`bmm3` sit side by
side, prelude = the stable units —, `openehr_am::v1_4`/`v2_4`,
`openehr_term::v3_1`), driven by the codegen composition table
(`tools/openehr-codegen/src/plan/composition.rs` — the single authority for
which generations exist and their paired dependency generations), each crate
with an emitted `Generation` enum as the ONLY pin authority (derived
`Default` — the `#[default]` variant IS the current generation —, per-variant
`const fn spec_version()`/`as_str()`, `FromStr`/`Display` on the module
token; no version constants exist in the generated crates). The crate prelude
re-exports the CURRENT generation only; older generations are reached by full
module path; generated cross-crate references bind the PAIRED dependency
generation by full defining-module path (never a prelude). Hand-written
`*_impl.rs` siblings live inside their generation module — and a sibling that
is IDENTICAL across generations modulo generation paths is a generation-twin
TEMPLATE (`tools/openehr-codegen/templates/<crate>/…`, #1964): one
hand-written source, per-generation copies stamped by `emit` under
`@generated-from-template` (per-generation overrides at
`templates/<crate>/overrides/<module>/…` carry their adjudication; the
`hand_written_twins_are_templates` invariant refuses new unconverted twins);
cross-generation runtime
(`openehr_base::{serde_support,containers,validate}`) stays top-level.
