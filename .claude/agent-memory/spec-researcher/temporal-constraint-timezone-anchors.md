---
name: temporal-constraint-timezone-anchors
description: Where the cADL date/time/timezone CONSTRAINT-PATTERN rules live across all SIX layers (ADL2 ch.04.5, the verbatim ADL1.4 master05 twin, the 3 grammars, the AOM C_TEMPORAL_DEFINITIONS model lists, the AOM BMMs, the ITS-XML AM XSDs + the rasterizable UML diagrams) plus the VERIFIED-FALSE traps and the confirmed defect set
metadata:
  type: reference
---

# Temporal constraint patterns + timezone — the SIX layers

A timezone/date-time-pattern question has SIX authorities; check all six
before calling anything a defect, and before calling anything *settled*
(verified first-hand 2026-08-22 re-verifying #2563 — layers 5 and 6 were
added then, and layer 5 nearly refuted a claim that layers 1-4 supported).

| layer | file | anchors |
|---|---|---|
| 1 ADL2 prose | `AM/docs/ADL2/master04.5-cadl_primitive_types.adoc` | intro L370 · exhaustiveness sentence L381 · pattern table L383-410 · "(but not date)" L414 · timezone table L416-424 · assumed-value L430 · §Assumed Values L140-146 (NO examples) |
| 2 ADL1.4 prose (VERBATIM twin, same defects) | `AM/docs/ADL1.4/master05-cadl.adoc` | §Constraints on Dates… L843-932: intro L852 · exhaustiveness sentence L863 · pattern table L865-892 · "(but not date)" L896 · timezone table L898-906 · assumed-value L912 · intervals L914-932 · §Assumed Values L1007-1019 (**the `some_date` example is L1018, ADL1.4-ONLY**) · §Symbols lexical spec L1283-1498 |
| 3 grammar (3 rival artifacts) | vendored `.g4` + the 1.4 chapter's OWN lexical spec | see the grammar table below |
| 4 AOM object model (docs text) | `AM/docs/UML/classes/org.openehr.am.aom2.c_temporal_definitions.adoc` | 3 pattern lists L16-26 / L55-64 / L89-99 · 3 replacement hashes L29-50 / L67-84 / L102-125 · `valid_iso8601_*_constraint_pattern` fns L131-160 |
| 5 **the AOM BMMs + the ITS-XML AM XSDs** | `tools/openehr-codegen/vendor/bmm/components/AM/json/openehr_am_{1.4.0,2.4.0}.bmm.json` · `crates/openehr-its/schemas/xml/its-xml-2.0.0-nsv2/AM/{Release-1.4,latest}/*.xsd` | see §Layer 5 below — **this layer disagrees with layer 4** |
| 6 **the published UML diagrams** (0-text, rasterizable) | `AM/docs/UML/diagrams/AM-aom{14.archetype,2}.constraint_model.primitive.svg` | `rsvg-convert -z 2 <svg> -o out.png` renders fully legibly; **also disagrees with layer 4** |

`C_TEMPORAL.pattern_constraint` is the AOM2 carrier (`…aom2.c_temporal.adoc`
L19; named explicitly at `AOM2/master04.2` L225; formal Eiffel at
`AOM2/master04.5` L636-638+L645; AOM2 diagram; AOM2 XSD) — five witnesses,
all spelling it in FULL, so "`pattern` is shorthand for it" is refuted.

## Layer 5/6: where the AOM class TABLES are contradicted

- **AOM 1.4 has NO `pattern` attribute** in the class tables, the BMM, or the
  diagram — but the **ITS-XML `AM/Release-1.4/Archetype.xsd` declares an element
  literally named `pattern`** on C_STRING (L248), C_DATE (L282), C_DATE_TIME
  (L300), C_TIME (L320), C_DURATION (L337). So the AOM1.4 `Pattern_validity`
  invariants' bare `pattern` is a MISSING-ATTRIBUTE defect (fix = declare it),
  not a wrong-name defect. `AOM1.4/master00` L66 (SPEC-251) even promises
  "Add pattern attribute to `C_DURATION`" — never added to the table.
  Our `openehr_its::opt14::types` HAS `pattern` (XSD-derived); our
  `openehr_am::v1_4` does NOT (BMM-derived). Both are correct to their source.
- **`C_DATE.timezone_validity`**: absent from BOTH class tables and BOTH BMMs,
  but PRESENT in the aom14 **diagram** and in `AM/Release-1.4/Archetype.xsd`
  L283. (Supersedes the older flat claim "C_DATE has no timezone_validity in
  either generation" — true of tables+BMM only.)
- **AOM2 `Archetype.xsd` omits `pattern_constraint` from `C_DATE`** (nsv2 L215-219
  / nsv1 L215-ish are bare extensions) while declaring it on C_DATE_TIME/C_TIME/
  C_DURATION; `DateConstraintPattern` is referenced only from `P_Archetype.xsd`
  (L240). A date pattern is unserializable in the AOM2 model-form XML.

## The three rival grammars for ONE token name

- `crates/openehr-adl/vendor/grammar/v2_4/base_lexer.g4` **L35-37 + L45** —
  `DATE_CONSTRAINT_PATTERN` (NO tz) · `TIME_CONSTRAINT_PATTERN … TZ_PATTERN?` ·
  `DATE_TIME_CONSTRAINT_PATTERN : DATE… 'T' TIME…` · `fragment TZ_PATTERN : '±' ('hh'|'HH') (':'? ('mm'|'MM'))? | 'Z' ;`
- `crates/openehr-adl/vendor/grammar/v1_4/cadl14_primitives.g4` **L74-77** —
  re-defines all four LOCALLY; TIME (L75) has **no** tz; DATE_TIME (L76) also `'T'` only.
- the ADL 1.4 chapter's OWN §Symbols spec — `master05-cadl.adoc` **L1415-1426**,
  no tz on any, and DATE_TIME's separator is **`[ T]`** (L1422) — the ONLY artifact
  admitting a space. The XSD facet (`ArchetypeCommon.xsd` DateTimeConstraintPattern)
  says `[T]`.

Both `masterAppC` (ADL1.4, L4 "The normative specification … is expressed in
Antlr4") and `masterAppB` (ADL2) declare the .g4 set normative.

## VERIFIED-FALSE traps (do NOT report these)

1. "TZ_PATTERN admits only `±`, the table's `Z` has no grammar." FALSE — second alternative IS `'Z'`.
2. "ADL2 masterAppB L57 has the `{grammar_dir}` mismatch too." FALSE — `{grammar_dir}` occurs EXACTLY ONCE tree-wide: `ADL1.4/masterAppC` L66.
3. "the tz in `yyyy-mm-dd±hh; 1970-01-01+02` might sit on the assumed value." FALSE twice.
4. The 4 sentences citing `*_CONSTRAINT_PATTERN` are NOT dangling.
5. "`pattern` might be an established shorthand for `pattern_constraint`." FALSE — see the five full-spelling witnesses above.
6. "AOM 1.4 might declare `pattern` somewhere I haven't looked." Checked exhaustively: tables, chapters, BMM, diagram → no. Only the XSD (layer 5) has it.

## Confirmed defect set (re-verified 2026-08-22, upstream-report #2563)

- 6 `Pattern_validity` invariants use bare `pattern` (aom2 c_date L66 / c_time L74 / c_date_time L82; aom14 c_date L36 / c_time L52 / c_date_time L64) — identical text in both BMMs.
- `valid_time_constraint_replacements` keys `"HH-??-??"` L78 / `"HH-??-XX"` L83 — `HH-` occurs EXACTLY twice tree-wide, so both are unmatchable.
- aom14 c_time L32 + c_date_time L44 "Validity of timezone in constrained **date**" (aom2 twins say "time"/"date/time" — L55/L79).
- aom14 c_time L64 merges `Validity_is_range` into the `Second_validity_disallowed` cell — merged in the BMM too, so not a rendering artifact.
- The chapter pattern tables list **14 of the model's 17** patterns; missing: `HH:MM:??`, `YYYY-MM-DDTHH:??:??`, `YYYY-XX-XX`.
- `AOM1.4/UML/…c_date_time.adoc` L67 invariant key `Month_validity_optional:` (stray colon, in the BMM too).
- `valid_iso8601_*_constraint_pattern` is defined ONLY on AOM2 `C_TEMPORAL_DEFINITIONS` — AOM 1.4 has no such class, so its 3 invariants call an undefined function.
- `as_upper` (used 6× in aom2 c_date/c_time/c_date_time) is not a declared BASE `String` feature.

See [[adl2-cadl-primitive-types-location]], [[aom14-domain-constraint-classes-location]],
[[unvendored-material-and-diagram-extraction]].
