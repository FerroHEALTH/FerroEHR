---
name: sm-terminology-service-ch12-location
description: SM Platform ch.12 (Terminology service) map — master12 is 21 lines/include-only, the 7 Mixed_case class files, the fully-legible SVG as sole source of I_STATUS inheritance + Hash qualifiers, and the confirmed defect set (zero postconditions/exceptions, no versioned request path, `Term` subtype does not exist, value_set_id-vs-value_set_code)
metadata:
  type: reference
---

Reference text only: the Service Model is not an authority (owner ruling
2026-10-27); it grounds no verdict and no feature.

# SM Platform ch.12 "Terminology Service" — navigation

Sibling of [[sm-admin-service-ch15-location]] and
[[sm-subject-proxy-ch10-location]]; cross-cutting rules (master02/master03)
catalogued in [[sm-ehr-service-chapter5-location]]. The openEHR-side
terminology *content* spec is TERM, not this chapter; this chapter is the
abstract SERVICE interface only. Extension-surface silence for the wire is in
[[extension-surface-spec-silence]].

## File map
`SM/docs/openehr_platform/master12-terminology_service.adoc` = **21 lines**,
2 sections: `== Overview` L3-9 (**ONE prose sentence, L5**, + the package SVG)
and `== Class Definitions` L11-20 = **7 `include::`** and nothing else. There
is NO §Usage, NO §Persistence, NO §Bindings, NO TBD paragraph. Note the
numbering hole: platform has master10 then **master12** then master15 (no
master11/13/14 exist); `master.adoc` L63 includes master12 between
master10 and master15.

The 7 includes, in file order: `i_terminology_service`,
`terminology_description`, `terminology_extract`, `terminology_relation`,
`term_relationship`, `term_code`, `defined_term`. **9 interface calls**, 6
data classes, **1 invariant total** (`Terminology_relation.Inv_valid_definition`),
**0 postconditions, 0 exceptions, 0 `.Parameters`/`.Errors` blocks**.

`i_terminology_valueset.adoc` and `term.adoc` DO NOT EXIST (grep-verified);
the `Term` subtype named in `terminology_extract.terms`'s Meaning cell is a
dangling name — the real subtype is `Defined_term`.

## Diagram — rasterizes perfectly, and is load-bearing
`SM/docs/UML/diagrams/SM-platform.interface.terminology.svg` = 0 `<text>`,
199 `<path>`, viewBox 769x695. `rsvg-convert -w 2600` fully legible. SOLE
source of:
- **`I_TERMINOLOGY_SERVICE` inherits `I_STATUS`** (hollow triangle) — the class
  table has NO `Inherit` row (same defect class as ch.4/ch.15);
- the Hash **qualifiers**: `Terminology_extract.terms` keyed `code : String`
  (role `+terms`, `*`) and `.relations` keyed `name : String` (role
  `+relations`, `*`); `.relationships` composition `*`;
- `Defined_term.language` and `Terminology_relation.external_code` rendered as
  associations to `Terminology_code` (`0..1` each);
- the full `Terminology_code` box (BASE foundation type, drawn orange =
  imported): `terminology_id [1]`, `terminology_version [0..1]`,
  `code_string [1]`, `uri : Uri [0..1]`.
- every `List<X> [0..1]` in the tables is drawn `X [*]` (flattened) — the
  table-vs-diagram multiplicity divergence, 6 sites.

`SM-platform.definition.svg`: the component is named **`TERMINOLOGY`** (not
`TERMINOLOGY_SERVICE`), grouped under **"Knowledge Services"** with
`DEFINITIONS_SERVICE`, exposing exactly ONE interface. (That same diagram
omits `I_DEFINITION_QUERY` from `DEFINITIONS_SERVICE` — a ch.4 defect.)
`SM-platform-packages.svg` confirms `definition::TERMINOLOGY` +
`interface::terminology`.

## PLATFORM_SERVICE — the gap, precisely
`platform_service.adoc` has 8 literals (Admin, Definitions, Ehr, Ehr_index,
Demographic, Message, Query, System_log): **no `Terminology`, no
`Subject_proxy`**, against master02 L27-42's 10-row service table. But the
enum's only 4 consumers are `I_ADMIN_SERVICE.{list_contributions,
contribution_count, versioned_composition_count, composition_version_count}`,
all documented `_a_service_` = "Name of a versioned content service" — and
**no ch.12 operation takes a PLATFORM_SERVICE argument at all**, so the gap
does not block any terminology call; it falsifies the enum's own
"Enumeration of platform service names" description.

## Total silence outside SM (grep-verified)
`I_TERMINOLOGY_SERVICE` + all 6 data-class names appear ONLY in SM
(`class_index.adoc` + their own files). ZERO ITS-REST operation
(`ls ITS-REST/specifications/operations | grep -i termin` = empty), ZERO CNF
`platform_test_schedule/master*-func_tc_terminology*`, ZERO
`CNF/tests/platform/robot/I_TERMINOLOGY_SERVICE/`. The only CNF anchor is
`CNF/docs/profiles/master03-profiles.adoc` L51 "AQL & terminology" under
*Querying*, ticked **OPTIONS only** — there is no Terminology product-component
row in the profiles table. `master00-amendment_record.adoc` has **zero**
occurrences of "terminolog": the chapter was never announced.

## Naming-convention break
The 6 data classes are the ONLY Mixed_case class names in the whole SM platform
model (`Terminology_description`, `Terminology_extract`, `Terminology_relation`,
`Term_relationship`, `Term_code`, `Defined_term`); every other SM class is
UPPER_SNAKE (`CALL_STATUS`, `RESULT_SET`, `SUBJECT_PROXY`, …). Visible as the
sort break at `class_index.adoc` L129 + L233-241.

## Cross-component name adjacency to watch
AM AOM2 uses `terminology_extracts` for something ELSE entirely
(`Hash<String, ARCHETYPE_TERMINOLOGY>` on OPERATIONAL_TEMPLATE —
`AM/docs/UML/classes/org.openehr.am.aom2.operational_template.adoc` L25,
`AM/docs/AOM2/master07-terminology_package.adoc` L20). BASE also has
`Terminology_term` (`concept: Terminology_code` + `text`) which is a near-twin
of SM's `Defined_term` and is NOT reused by ch.12.
