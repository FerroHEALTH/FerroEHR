# `openehr-adl` — ADL 2.4.0 engine (hand-written)

The ADL2 text + semantics engine: ADL2/cADL/ODIN parser (`logos` + `chumsky`,
like `openehr-query`), the AOM2 validation engine, the `master04.5` conformance
functions, specialisation flattening, OPT2 generation, the ADL printer, and
ADL 1.4→2 conversion. Builds directly into the generated
`openehr_am::v2_4::aom2` model — never re-model AOM2.

## Module map (the whole `src/` tree, one line each)

**Substrate — read through, never re-inlined.** These sit BELOW everything
else; each helper has exactly ONE home.

| module | role |
|---|---|
| `error` | the typed `SyntaxError` + the verbatim `S*` catalogue (`ADL2/master04.6`) |
| `hrid` | `ARCHETYPE_HRID` parse / print / lookup-key (`AOM2/master07.05`) |
| `odin` | the ODIN reading bridge + literal-delimiter stripping over `openehr_lang::escape` (which owns the `master03` escape semantics for ODIN/BEL/cADL alike) + delimited-regex handling |
| `codes` | node-code math: `at`/`id`/`ac` prefixes, specialisation depth, redefinition |
| `paths` | ADL path parse + resolution over the constraint model (`crate::paths::child_path` included) |
| `aom/access` | the 13-arm `C_OBJECT` field accessors + `AomType` |
| `aom/build` | the AOM2 constructors |
| `aom/interval` | `Bounds` / multiplicity + `INTERVAL<T>` maths |
| `aom/nesting` | the engine nesting bound over a constraint tree (`check_definition_nesting`, over `openehr_lang::nesting`) — every producer of a COMPOSED tree (flattener, OPT transform) re-checks its result |
| `artefact` | `ArchetypeView`/`view` + `ArchetypeRepository`/`FlatParent` |

**Front end — source text → AOM2.**

| module | role |
|---|---|
| `source` | the outer artefact parser: sections, kind, meta, spans (`parse_source(src, dialect)`) — over `openehr_lang::lexer::lex_adl` |
| `parse/mod` | the **public `Dialect` seam**, the `Parser` state, `parse_definition_body(body, dialect)`, the re-entrant sub-parsers `rules` drives, cursor/error helpers |
| `parse/parser` | the cADL structure / attribute / tuple productions |
| `parse/refs` | archetype slots, `use_archetype` roots, `use_node` proxies |
| `parse/primitives` | the inline `C_PRIMITIVE` family |
| `parse/values` | value lists, `\|…\|` intervals, endpoints, the `CadlValue` kind trait |
| `parse/patterns` | the date/time constraint-pattern validators |
| `rules` | the `rules` section + slot assertions (BEL over `openehr_lang`) — full beom trees, plus the `slot_assertion_path`/`slot_assertion_regex` tree accessors every consumer reads through |
| `assemble` | fold the ODIN sections + `definition` + `rules` into a complete `Archetype` (`parse_artefact(src, dialect)`) |
| `meta` | the read-only artefact summary accessors (`ArtefactSummary`/`summarize`, `regression_tag`) |

**Semantics — validation, flattening, generation, serialization.**

| module | role |
|---|---|
| `validate/mod` | orchestration ONLY: `ValidationIssue`, the five public entry points, `push_issue`, and the four drivers (`run_integrity_checks` / `run_parent_conformance` / `run_flat_form_checks` / `run_filler_checks`) whose call order IS the `master08` schedule |
| `validate/catalogue` | the `Severity` + `ValidationCode` vocabulary (91 codes) |
| `validate/identification` | id / root / versions / languages + the STCNT/VOLT gate |
| `validate/structure` | the basic-integrity definition walk (`StructureScan`) |
| `validate/terminology` | term definitions, value sets, code usage |
| `validate/bindings` | binding keys + the `TerminologyResolver` seam for VETDF |
| `validate/annotations` | `annotations` + `rm_overlay` |
| `validate/source_level` | VOKU/VRRLP over the raw parsed source |
| `validate/specialisation` | the differential-vs-flat-parent walk (`ParentScan`) + the parent-dependent VACSD/VASID/VALC |
| `validate/slots` | the slot arm of that walk (a second `impl ParentScan` block) + `validate_fillers` |
| `validate/rm` | the reference-model seam (`RmModel`, `ProductionRmModel`, `validate_rm_conformance(archetype, rm, dialect)` over `RmScan`) |
| `validate/conformance` | the `master04.5` conformance functions |
| `validate/flat` | the flat-form walk (`FlatScan`: `validate_flat_form_structure` + the 1.4 VDFPT twin) + the deferred flat-form halves (`validate_flat_form`) |
| `flatten` | specialisation flattening (`flatten`, `flat_form`) |
| `opt` | OPT2 generation — raw via `create_opt`, profiled via `profile_opt`; `unresolved_constituents` names every archetype an OPT lacks |
| `print/mod` | the printer state, the artefact-kind projection, the top-level section driver, `print` + `assertion_text` |
| `print/header` | identification / `language` / `description` / `annotations` / `rm_overlay` / `component_terminologies` |
| `print/terminology` | the `terminology` section body (shared with `component_terminologies`) |
| `print/definition` | the cADL `definition` section + every primitive / interval / temporal rendering |
| `print/rules` | the `rules` section + the BEL expression printers (also serving slot assertions) |
| `print/odin` | generic ODIN rendering: keyed maps/lists, `_default`, quoted strings, `TERM_CODE_REF` |

**ADL 1.4 — everything 1.4-specific lives here and nowhere else.**

| module | role |
|---|---|
| `adl14/lower` | the 1.4-only cADL productions (qualified/listed term constraints, inline dADL domain blocks) |
| `adl14/domain` | the inline dADL domain-block lowering to `DV_QUANTITY`/`DV_ORDINAL`/… |
| `adl14/convert` | the 1.4→2 conversion core: code planning, node-id renumbering, terminology-constraint conversion, terminology rebuild |
| `adl14/walk` | the read-only definition traversals the code planning consumes + the shared `cco_data_mut` |
| `adl14/multiplicity` | the 1.4 default-occurrences materialisation + RM-default cardinality/occurrences elision |
| `adl14/metadata` | the description / standardised meta-data / HRID-version transform |
| `adl14/differ` | the differential-form reducer |
| `adl14/log` | the `ConversionLog` (code remaps, value sets, notes) |

## Crate rules

- **This crate has NO lexer.** The cADL token stream is the one workspace
  lexical layer, `openehr_lang::lexer` (`Token`/`Spanned`), read under its ADL
  reading `lex_adl`; the ODIN and BEL readings of the same superset back
  `openehr_lang::odin` and the `rules` BEL parse. Never add a token type or a
  `logos` enum here — a lexical difference is a rule in
  `openehr-lang`'s `lexer/reclassify.rs`, adjudicated against the vendored
  grammars, and pinned by that crate's `lexer_equivalence` battery.
- **Single home, no re-inlined copies.** A helper lives in exactly one module
  (the substrate table above); consumers read through it. One pair stays
  DELIBERATELY divergent, and its doc comments say why: the bounds renderers
  (`display_bounds` vs `display_bounds_always_range`, `aom/interval`), whose
  two spellings are load-bearing message text. The interval point-of reading
  is a SINGLE spec-adjudicated predicate
  (`aom/interval::point_value_{i32,f64}`): both sides bounded, both bounds
  included, both bounds equal — irrespective of point/proper tagging. The
  `master03` escape semantics likewise have exactly ONE implementation for the
  whole workspace — `openehr_lang::escape` (a CLOSED set: the six quoted forms
  plus `\uHHHH` and `\uHHHHHHHH`, everything else a typed decode error) — which
  the cADL parser and the shared lexer's
  `STRING`/`CHARACTER` callbacks both read through; the cADL side reports a
  decode defect as `SUNK` at the literal's span, the lexer refuses it at the
  lex.
  The two full-pipeline entries run the SAME schedule: `validate` and
  `validate_source` both drive basic integrity → RM conformance → parent
  conformance → flat form → fillers (VARXR, VTPL), and neither may omit a pass (there is no
  partial-validation profile in AOM2).
- **`validate/` is grouped by TOPIC, never by phase number, and NOTHING is
  named after a phase number.** A new rule goes in the topic module that owns
  its subject, and its driver call keeps the existing order — issue emission
  order is behaviour. Functions, structs, tests and files are named for what
  they DO (`validate_integrity`, `run_parent_conformance`, `FlatScan`,
  `corpus_validity_integrity.rs`); master08's "phase 1/2/3" vocabulary survives
  only as a doc-comment cross-reference into the spec.
- **The cADL parser is `parse/`, one module per production family**, and the
  ADL 1.4-only productions do NOT live there: they are `adl14/lower.rs` +
  `adl14/domain.rs`, and the three dialect-gated dispatch points in
  `parse::parser` are the only coupling — keep it that way.
- **The printer is `print/`, one module per artefact section.** Section
  modules are private; `print::print` (a whole artefact) and
  `print::assertion_text` (one assertion) are the serializer seams, both
  fallible over `print::PrintError` — a modelled node no released grammar
  spells (EXTERNAL_QUERY) is REFUSED, never rendered as invented or empty
  text. Printed output is pinned byte-for-byte by the round-trip corpus — a
  layout change is a behaviour change.
- **An assertion's EXPRESSION TREE is the authority, never its string form.**
  `ASSERTION.expression` is the "Root of expression tree" and
  `string_expression` only its "String form of expression"
  (`LANG/docs/BEL/master04-expression_object_model.adoc` §Core Package): the
  printer renders from the tree, `rules::parse_slot_assertions` fills each
  assertion's string form FROM that rendering (which is what makes
  parse → print → parse a fixed point), and validators read the tree through
  the `rules::slot_assertion_*` accessors. Never scan the string form for a
  path, a regex or an operator.
- **No file over ~1,000 non-test LOC.** When a module crosses it, split by
  subject (production family, artefact section, validation topic), never by
  phase number or arbitrary line count.
- **Zero re-exports; the public seam is deliberate.** Every import names its
  defining module; anything with no out-of-crate consumer is `pub(crate)` /
  `pub(super)`, not `pub`.
- **Spec oracle:** `docs/specs/openehr/AM/docs/{ADL2,AOM2,OPT2}/` +
  `LANG/docs/odin/` (`/spec-lookup`). Every validation rule cites its code +
  spec file/section; master08-only codes and all other spec-silences carry the
  explicit `// NOTE:` flag.
- **No ANTLR runtime, ever.** The normative `.g4` grammars from
  `openEHR/adl-antlr` are vendored under `vendor/grammar/` (version-scoped by
  AM generation — `v1_4/`/`v2_4/` — with PROVENANCE.md; refresh via
  `scripts/vendor/adl-grammars.sh`) as reference input only.
- **The dialect is a PUBLIC PARAMETER, never a twin function.**
  `parse::Dialect` is the seam: `parse_source`, `parse_definition_body`,
  `parse_artefact`, `assemble`, `validate::validate_source_integrity` and
  `validate::rm::validate_rm_conformance` each take it and dispatch internally
  — there are no `*_adl14` twins. The one deliberately separate 1.4 entry is
  `validate::validate_adl14_source(src, rm)` (the FULL 1.4 catalogue), because
  the 1.4 pipeline takes neither an `ArchetypeRepository` nor a
  `TerminologyResolver` (no differential lineage to flatten, no AOM2
  external-binding rule) — folding it into `validate_source` would add two
  parameters it silently ignores.
- **A 1.4 upload is judged AS 1.4, never as an ADL2 superset.**
  `parse::Dialect::Adl14` both ADDS the 1.4-only forms and REMOVES the
  constructs ADL 2 introduced — the cADL 1.4 keyword set is closed
  (`ADL1.4/master05-cadl.adoc` §Keywords). The dialect reaches the OUTER
  structure too (`source::parse_source` in `Dialect::Adl14`, the reading every
  1.4 caller gets): 1.4 section keywords are case-insensitive (`master08-adl.adoc`
  §Symbols), an old-form archetype with `primary_language`/
  `languages_available` in its ontology and no `language` section is accepted
  and upgraded on parse (§Language Section + §Ontology Header Statements), and
  the `concept` section is mandatory (§Syntax Specification `arch_concept`;
  VARCN). ADL2 outer parsing is unchanged — exact lowercase keywords,
  unconditional `SALAN`, no concept clause. The 1.4-only validity rules (VCOC;
  VATDF/VACDF over the qualified/listed spelling) run on the
  `Dialect::Adl14` basic-integrity path only. The `S*` error space is a verbatim 1:1
  mirror of the openEHR catalogue (`ADL2/master04.6`): never invent a code —
  reuse the catalogue code for the parse position and name the construct in
  the message. The `V*`/`W*` VALIDITY space is the AOM2 `master08` catalogue
  plus a CLOSED set of three flagged local extensions (`VRDLA`, `WOUC`,
  `W14DEP` — each carrying the explicit "no openEHR spec defines this code"
  flag at its variant) plus the RM resource-meta family (`Rm*` variants whose
  mnemonics ARE the RM class-table invariant names — spec-grounded in RM
  common ch.8, adjudicated on issue #2447; the empty-prose rows
  `Purpose_valid`/`Use_valid`/`misuse_valid` are Warning-severity on a 1.4
  SOURCE because `<"">` is the 1.4 ecosystem's spelling of absence, pinned by
  the `ckm_archetype_packs` sweep); extending either set is an adjudicated
  decision, since a new code is a new accepted/refused surface, never a
  convenience.
- **1.4 defaults are EFFECTIVE-value accessors, never mutations** —
  `validate::conformance::effective_{existence,occurrences}_adl14` apply
  master05's `{1..1}` defaults (and the `use_node` inheritance rule) on read;
  an absent value stays absent in the parsed AOM. Only the 1.4→2 CONVERTER
  writes a default out, because ADL 2 infers a different one.
- **ADL 1.4→2 conversion (`adl14/`) has NO spec basis** — the whole module is
  our own design (archie is prior art only), flagged as such; every heuristic
  is pinned by the paired `upgrade_from_14` corpus fixtures, not a spec clause.
  `ConvertConfig::collapse_specialised_codes` collapses dotted specialised
  codes to a depth-0 flat code space for a standalone flattened-OPT root (the
  differential lineage is unresolvable there); reused 1.4 node codes are
  re-minted archetype-wide-unique (VCOSU) with their terminology bindings
  cloned, and all remaps are recorded in
  `RESOURCE_DESCRIPTION.conversion_details`.
- **Conformance corpus:** `tests/corpus/` (the openEHR ADL2 regression library;
  file names encode the expected rule code) plus two HAND-WRITTEN ADL 1.4 trees
  the vendored ADL2 library cannot cover — `adl14-dadl/` (master04 dADL breadth)
  and `adl14-cadl/` (master05 cADL: dialect gates, domain lowering,
  VATDF/VACDF/STCDC/STCAC, VCOC, the operators the chapter names but no grammar
  defines — `~matches`/`~is_in`/`∉`, `=~`/`!~` — and the
  `cadl_breadth_{structure,primitives,datetime}` trio covering every construct
  of the chapter). `app/ferroehr/tests/it/adl14_knowledge_archetypes.rs` is the
  DB-free parse gate over the app's real-world CKM 1.4 knowledge resources.
  Corpus cases are the regression net — never delete/weaken one to get green;
  a defect goes through adjudication, not case edits, and every refusal keeps
  its accepting twin.
- Dependencies point downward only: `openehr-am`, `openehr-base`, `openehr-rm`
  (the generated RM model oracle), `openehr-lang` (ODIN/BEL), `openehr-term`
  (the languages code set for the resource-meta rows). No app-crate
  knowledge, no SQL, no REST.
- Spec pin: ADL/AOM 2.4.0 via `SPEC_VERSION`; the package version is the
  crate's own SemVer line (`.claude/rules/crates-publishing.md`).
- Gates: `cargo clippy -p openehr-adl --all-targets` +
  `cargo nextest run -p openehr-adl`.
