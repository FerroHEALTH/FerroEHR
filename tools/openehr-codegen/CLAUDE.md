# `openehr-codegen` — the BMM/XSD/OAS → Rust generator (hand-written tooling)

The single deterministic generator behind the whole spec layer. Lives under
`tools/*` (dev tooling; nothing ships it). Subcommands (`src/cli.rs`):

- `emit` — BMM → `openehr-base/rm/am/term/lang` (incl. the `openehr-rm` model).
- `emit-json` — BMM → the canonical-JSON `serde::Serialize`/`Deserialize`
  impls, as MANUAL long-form impls (a field-identifier enum + a visitor,
  <https://serde.rs/deserialize-struct.html>), one `src/json_serde.rs` per SPEC
  CRATE — they must live where the types are defined (orphan rule) — plus the
  `_type` dispatch + declared-key table in `openehr-its`. Never a serde derive:
  none of serde's four enum representations expresses the canonical `_type`
  discriminator. Shared runtime: hand-written `openehr_base::serde_support`.
- `emit-xml` — XSD + BMM → `ToXml`/`FromXml` in `openehr-its`.
- `emit-rest` — the vendored OAS → the ITS-REST contract in `openehr-its`.
- `emit-opt` — the OPT 1.4 model + XML codec (`openehr-its` `opt14`).
- `emit-aom2` — BOTH AOM2 archetype XML serializations: the persistent form
  (`P_Archetype.xsd` → `aom2`) and the AOM model form (`Archetype.xsd` →
  `aom2_model`), each from its own curated XSD closure.
- `emit-rm-model` — the static RM attribute/type model (refreshes the subtree
  `emit` already writes).
- `emit-validate` — the machine-classified RM invariant cores.
- `model-query` — read-only report: what the vendored BMM states about every
  class attribute (declared type, existence, container + cardinality, class
  abstractness) beside the field shape the emitter currently emits for it;
  `[--class X] [--attribute Y] [--component KEY] [--flattened]
  [--format table|tsv|json]`. **`--flattened` is the inheritance dimension**:
  one row per class × CARRIED attribute (inherited ones included) with the
  declaring class in `declared_on`, so a per-descendant divergence is
  queryable instead of assumed — the default view reports DECLARED attributes
  only, which cannot express it. Use the flattened view for any question of
  the form "does every class that carries X emit the same shape".
- `check` / `check-xsd` — input validation.

**Generation-twin templates** (`templates/<crate>/…`, stamped by `emit` —
`src/render/emit_templates.rs`, #1964): a hand-written spec-behaviour file
that is identical across a crate's generations modulo generation paths keeps
ONE source here, written against the CURRENT generation; `emit` stamps the
per-generation copies (own module + paired dependency-generation tokens from
the composition table) under an `@generated-from-template` header, so the
purge/drift machinery owns them and divergence is impossible. A genuinely
generation-specific difference is a per-generation override
(`templates/<crate>/overrides/<module>/…`, taken verbatim, carrying its
adjudication — the live case: RM v1_1 `item_tag_impl`, the 1.1.0 field
order). The `hand_written_twins_are_templates` emitter invariant fails on
any remaining identical hand-written twin pair.

## Pipeline structure (four stages + CLI)

Four stages, one directory each — every stage consumes the previous stage's
output, never the raw files:

- `src/load/` — parse the vendored inputs verbatim (`bmm`, `xsd`, `oas`); no
  analysis, no decisions.
- `src/analyze/` — model analysis over the loaded BMM: merged include closures,
  descendant/variant sets, the ownership graph + back-reference cycle breaking,
  the constructibility proof, the cross-schema re-emission closure, and the
  invariant classification (`invariants.rs`). Plain analysis results, no text.
- `src/plan/` — the emission-decision layer: `mod.rs`/`composition.rs` decide
  the Rust shape each class emits as (+ the XML-shape classification), and
  `overrides.rs` holds the declarative decision maps (`type_override`,
  `class_binding`, `back_reference`, `field_default`, `primitive`,
  `is_mapped_class`, `xml_bmm_only_allowed`), each carrying its spec citation.
- `src/render/` — the only stage that produces text: `emit`, `emit_json`,
  `emit_xml`, `emit_rest`, `emit_rm_model`, `emit_opt`, `emit_validate`, and the
  shared `naming` helper.
- `src/cli.rs` + a thin `src/main.rs` — argument dispatch. EVERY emit target's
  text production is a `render_*_files()` seam consumed by both its `cmd_*`
  write-shell and `testsupport::emit_*_to_memory()` (#2686/#2687), so the CLI
  and the invariant tests can never diverge by code path; the `cmd_*` handlers
  only write files.

## Discipline

- **Every emitter change must be followed by regeneration + diff review**
  (`/regen-codegen` runs the emits + the drift check). Never commit an emitter
  change without its regenerated output in the same change — the
  `codegen-drift` CI job fails otherwise.
- The emitter owns generated-code quality: generated crates must stay idiomatic
  and lib-clippy-clean **by construction**. A clippy warning in generated output
  is an emitter bug — fix it here, never in the output.
- Emission conventions are settled decisions (flattened concrete structs,
  untagged enums for closed subtype sets, `Box` for recursion, bound-fill +
  monomorphization for generics, `// @generated` headers) — do not re-litigate
  per class; extend the decision maps in `plan/overrides.rs` instead.
- Vendored inputs live at `vendor/bmm/` (with provenance) — never edit a
  vendored file; a spec bump moves the pin in `scripts/vendor/codegen-inputs.sh`
  (BMM, ITS-JSON, ITS-XML, ITS-REST OAS, plus the component-repo BMM overlays),
  re-runs it, and regenerates; `--check` proves the committed inputs.
- The generator writes ONLY generated files/subtrees; it must never touch
  `*_impl.rs` siblings or hand-written runtimes.
- Gates: `cargo clippy -p openehr-codegen --all-targets` +
  `cargo nextest run -p openehr-codegen` + a clean drift check.

## Container shapes (the emission adjudication)

A container property's Rust shape follows its BMM **existence** and
**cardinality**, decided at one point in `render/emit.rs`:

| BMM | Rust | why |
|---|---|---|
| mandatory (`1..1`), cardinality `0..*` | `Vec<T>` | genuinely admits zero members |
| mandatory, cardinality `1..*` | `NonEmptyVec<T>` | the bound is a structural statement about the model, so the type carries it |
| optional (`0..1`), no non-empty rule | `Option<Vec<T>>` | "absent" and "present-but-empty" are two legitimate model states |
| optional, with a non-empty invariant | `Option<NonEmptyVec<T>>` | no valid instance can carry present-but-empty, so it stops being representable |

The RM **does** distinguish Void from empty on a `0..1 List<T>`. Where a class
invariant of the `x /= Void implies not x.is_empty` family forbids the
present-but-empty state (`LOCATABLE.Links_valid`,
`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.locatable.adoc`
§Invariants), the invariant then holds by construction and the strict readers
refuse `[]` at parse — parse-don't-validate, the same door the `1..*` mandatory
containers take. Where no such rule exists (`FOLDER.items`/`folders`,
`…org.openehr.rm.common.folder.adoc` §Attributes types both `0..1` with no
non-empty rule), both states are real and the type must carry both.

The `1..*` cardinality source is the BMM, minus the `LowerBound` entries of
`plan/overrides.rs` `SPEC_CONTRADICTIONS` (`cardinality_contradicted`) — an
attribute whose bound the release's own syntax chapter contradicts. The map's
`Existence` entries (`existence_contradicted`) do the same for a mandatory
`Hash`: it stays a `BTreeMap`, absent reads as empty, and empty is omitted on
write (`P_BMM_PACKAGE_CONTAINER.packages`).

**The wire is unaffected in the write direction**: the canonical-JSON writer
omits an empty list whether it is `None` or `Some(vec![])`, per
`docs/specs/openehr/ITS-REST/specifications/docs/overview/Resources.md` §JSON
Format ("The RM attributes (even required ones) that are `Null` or an empty
list (array) SHOULD be absent when serialized as JSON"). The reader is the
direction that gains: absent → `None`, `[]` → `Some(vec![])` (or a refusal
where the type is `NonEmptyVec`).
