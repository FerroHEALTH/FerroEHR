---
paths: ["crates/openehr-its/**", "crates/openehr-sdt/**"]
---

# Serialization rules — canonical JSON, canonical XML, FLAT/STRUCTURED
(rewritten 2026-07-13)

## Spec sources (the oracle)

The vendored normative text is the authority for every wire shape
(spec-adherence.md; `/spec-lookup`):
`docs/specs/openehr/ITS-JSON/components/` (canonical-JSON schemas — the pin is
recorded in `docs/VERSIONS.md`), `docs/specs/openehr/ITS-XML/` + the vendored
XSDs at `crates/openehr-its/schemas/xml/` (canonical XML),
**`docs/specs/openehr/ITS-REST/docs/simplified_formats/` (STABLE — the
FLAT/STRUCTURED wire authority: `master04` syntax + algorithms, `master05`
per-type tables, `master06` ctx vocabulary)**, and the CNF content chapters
(`docs/specs/openehr/CNF/docs/platform_test_schedule/master15`–`master17.x`)
with the canonical fixtures under `CNF/tests/platform/robot/_resources/` as
reference material. The SM `serial_data_formats` + `simplified_im_b` docs are
DEVELOPMENT-state model documents (their terse string encodings conflict with
the STABLE suffix encoding — never implement them).

## The `_type` mechanism (generated manual serde impls — do not improvise)

Canonical-JSON `_type` self-tagging is **emitted MANUAL
`serde::Serialize`/`Deserialize` impls** — `openehr-codegen -- emit-json`
writes explicit generated code into each defining crate's `src/json_serde.rs`
(`openehr-base`/`-rm`/`-am`/`-term`/`-lang`), over the small shared
hand-written runtime `openehr_base::serde_support` (tag-anywhere enum
buffering, the class-naming `unknown_field` helper, slot-tag verification).
The spec types carry NO serde derives and NO serde attributes — per-class
field-identifier enums + visitors, the serde.rs manual long form, so every
wire decision is auditable generated code (#1702). Serde is the ONE codec
seam: never introduce a parallel `ToJson`/`FromJson` trait pair or a
hand-written JSON codec runtime beside it.
Entry points are `openehr_its::json::{to_canonical_json,
from_canonical_json, from_canonical_value}`; refusal diagnostics carry the
full JSON path via `serde_path_to_error` (error path only — the happy path
reads untracked). The contract (know it; never hand-write it):

- **Serialize** emits `"_type": "<CLASS>"` first, then fields in BMM
  declaration order; `Option` fields are omitted when `None` — no `null`s
  (container emptiness-vs-absence follows the leg-6 container adjudication).
  Integer-typed RM fields print as JSON integers; Real-typed fields carry a
  decimal point (exponent lexeme is signed, `1e+21` — no openEHR spec governs
  the REAL lexeme; RFC 8259 §6 admits both).
- **Deserialize is the STRICT reader** (parse = the 400-class shape check):
  input accepted with or without `_type` on a concretely-typed slot;
  present-but-wrong `_type` is an error; an abstract polymorphic slot
  requires `_type` and dispatches on it (tag anywhere in the object — keys
  before the tag are buffered and replayed); **undeclared keys are REFUSED**
  (named, with the class and the legal field set — never tolerate an unknown
  key); **repeated members are REFUSED** (`duplicate_field`);
  absent mandatories are `missing_field`; identifier-typed fields construct
  through the validated master05-grammar doors, so a malformed uid refuses at
  parse, path-named. Semantic invariants + terminology stay AFTER parse in
  `openehr_rm::validate` (the 422 class) — never folded into `Deserialize`.
- Closed subtype-set enums are plain Rust enums whose emitted `Deserialize`
  dispatches on each payload's `_type` (deep descendants routed to their
  direct variant).

Wrong `_type`/serialization behaviour → fix the **emitter** (`emit.rs` /
`emit_json.rs`) or `openehr_base::serde_support`, and regenerate
(`/regen-codegen`); never hand-edit `// @generated`.

## Canonical JSON (ITS-JSON)

- Attribute names snake_case; metadata keys `_`-prefixed.
- `_type` is the uppercase RM class name and is **required** whenever the
  statically declared field type is abstract (any `DATA_VALUE`-typed slot);
  a concretely-typed field may omit it.
- Abstract classes flatten into the concrete shape; nulls are omitted
  entirely; UIDs serialize as `{"_type": …, "value": …}`, never a bare
  string; `DV_MULTIMEDIA.data` is inline base64.
- Validation oracle: the vendored `openehr_rm_1.1.0_all.json` schema
  (`crates/openehr-its/schemas/json/`, exact commit pinned in
  `docs/VERSIONS.md` — ITS-JSON has no numbered release). Note the schema's
  RM-1.1.0 ceiling vs our RM 1.2.0 where relevant.

## Canonical XML (ITS-XML) — GENERATED

`ToXml`/`FromXml` impls are emitted by `emit-xml`, for every RM/BASE
generation, into `openehr-its/src/xml/generated/` from the vendored XSDs (wire shape: element
order, attribute/element split, `xsi:type` slots) + the BMM model (Rust
fields). Hand-written parts: the runtime (`xml/runtime.rs`, `quick-xml`) and
entry points.

- Namespace `http://schemas.openehr.org/v1`; **one impl set serves both the
  1.0.2 (STABLE, the target) and 2.0.0 (TRIAL) bundles** — they differ only
  by root `xmlns`, selected at serialize time. Both XSD bundles stay
  vendored.
- `xsi:type` is emitted iff the concrete type differs from the declared slot
  type; inbound dispatch routes deep types through the descendant→variant
  map.
- C14N uses the `xmllint --c14n` shell fallback — never hand-roll C14N.

## FLAT / STRUCTURED / Web Template (`openehr_sdt::flat`, hand-written)

The Simplified Formats live in the `openehr-sdt` crate, over the
`openehr-its` wire layer.

- **The oracle is the STABLE ITS-REST Simplified Formats spec**
  (`docs/specs/openehr/ITS-REST/docs/simplified_formats/`): `master04`
  (field-identifier syntax, node-id generation, level removal, `|raw`,
  `|other`, the FLAT⇄STRUCTURED algorithms), `master05` (per-RM-type
  suffix tables — e.g. `DV_QUANTITY` `|magnitude`/`|unit`), `master06`
  (the `ctx/` vocabulary + defaults). Vendor implementations are prior
  art only; there is no quirks feature flag — spec behaviour is the only
  behaviour.
- Architecture: one internal tree (`sim::SimNode`); FLAT and STRUCTURED
  are pure codecs; RM conversion is written once (`flatten.rs`/`build.rs`,
  entry points in `convert.rs`). Spec-example JSON blocks are the primary
  test vectors; the OPT corpus is regression.
- MIME types (`master02 §MIME Types` + the docs-text
  `specifications/docs/overview/Resources.md` §Simplified Formats):
  `application/openehr.wt+json`, `…wt.flat+json`, `…wt.structured+json` —
  the release's exhaustive MUST-use set. The deprecated `.schema+json`
  names (§Simplified Formats NOTE: "now deprecated and will be removed")
  and the legacy `…nc.flat+json`/`…tds2+xml` (§Alternative data formats:
  "might not be supported") are NOT implemented — spec-legal because the
  same section makes simplified-format support optional and pins the
  refusal codes (415 on `Content-Type`, 406 on `Accept`). Adjudicated on
  issue #1872; asserted by `overview::negotiate`'s
  `deprecated_and_legacy_types_unrecognized` unit test and the
  `flat_http` 415/406 refusal pair over every banned name.

## Acceptance instruments

- **JSON:** the vendored canonical-JSON corpus round-trip gates in
  `openehr-its/tests/` (deserialize → re-serialize → normalized value
  equality + ITS-JSON schema validation) — real corpus data, not hand-built
  fixtures.
- **XML:** the round-trip + C14N gates; **FLAT/STRUCTURED:** the
  spec-example vectors (every JSON block in `simplified_formats`
  `master04`/`master05`/`master06`) + `insta` golden vectors + the OPT
  corpus round-trips (regression).
- Redact volatile fields (timestamps, generated UUIDs) before committing
  any snapshot. Never weaken a gate to get green (testing.md).
- At the wire, the CNF pipeline (Veredictum) is the end-to-end
  acceptance instrument.

Hand-written files here follow `rust-style.md` and `comments.md` (the
sanctioned comment-annotation forms only). Generated files
carry `// @generated` and are never hand-edited.
