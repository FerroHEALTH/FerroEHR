---
name: sm-message-service-ch9-location
description: SM Platform ch.9 (Message service) map — master09 is include-only, I_MESSAGE_SERVICE has ZERO features, the component diagram omits the two working interfaces, zero pre/post/errors anywhere, and CNF master13 tests operations that do not exist
metadata:
  type: reference
---

Reference text only: the Service Model is not an authority (owner ruling
2026-10-27); it grounds no verdict and no feature.

# SM Platform ch.9 "Message Service" — navigation

Sibling of [[sm-ehr-service-chapter5-location]] (owns the master02/master03
cross-cutting conventions every SM chapter inherits),
[[sm-admin-service-ch15-location]] (the other include-only skeleton chapter).

## File map (tiny and total)
`SM/docs/openehr_platform/master09-message_service.adoc` = **18 lines,
INCLUDE-ONLY**: §Overview = ONE sentence + the package SVG; §Class Definitions
= 3 `include::` pulls, ZERO own prose. Class files (`SM/docs/UML/classes/`):
`i_message_service` (**0 features** — Description row only, 12 lines),
`i_ehr_extract_service` (4 calls), `i_tdd_service` (2 calls).
Referenced types live in **RM** (STABLE): `RM/docs/UML/classes/
org.openehr.rm.ehr_extract.{extract,extract_spec}.adoc`.

## Diagram-only structure — rasterizes legibly
`SM/docs/UML/diagrams/SM-platform.interface.message.svg` = 77 `<path>`,
**0 `<text>`**; `rsvg-convert -w 2400` fully legible. ONLY source for:
(a) all THREE interfaces inherit `I_STATUS` **directly** (no `I_VALIDITY_CHECKER`
hop — unlike ch.5; NO interface file in the whole SM set carries an `Inherit`
row, grep-verified); (b) return element multiplicity `EXTRACT [*]` on both
export calls (tables say `List<EXTRACT>` with a meaningless leading `0..1`);
(c) `import_tdds()` genuinely takes **no arguments**.

## The component-model contradiction (the load-bearing one)
`SM-platform.definition.svg` (embedded by master02 §openEHR Platform Model)
shows `MESSAGE_SERVICE` exposing **only `I_MESSAGE_SERVICE`** — the two
interfaces that carry every operation are attached to NO component. The
convention is not "one lollipop per component": ADMIN_SERVICE shows 3,
EHR_SERVICE shows 6. MESSAGE_SERVICE sits under "Retrieval Services".

## Silences worth citing before claiming a requirement
- ZERO `Pre_`, ZERO `Post_`, ZERO `.Errors`, ZERO `.Parameters` in all three
  interfaces (`i_ehr_extract_service` + `i_tdd_service` + `i_message_service`
  join `i_ehr_index`/`i_query_service` as the only condition-free files).
- "TDD" is never DEFINED in SM. The only definitional text in the whole
  vendored tree is `ITS-REST/docs/simplified_formats/master03-design_rationale.adoc`
  L379 (historical rationale) + the `application/openehr.tds2+xml` media type in
  `ITS-REST/specifications/docs/overview/Resources.md` L158. `import_tdd(tdd: String)`
  has no format constraint.
- The amendment record NEVER mentions message/extract/TDD — the chapter is
  0.9.0 initial writing (15 Sep 2017), never amended.
- **ZERO ITS-REST surface**: no message/extract/tdd path in any of the 7
  vendored openapi specs (grep-verified). CNF profiles still lists a
  "MESSAGE API" capability.
- RM EHR Extract (STABLE) defines `MESSAGE`, `ADDRESSED_MESSAGE`,
  `EXTRACT_REQUEST`, `SYNC_EXTRACT`, `X_VERSIONED_*` — no SM operation accepts
  or returns any of them.

## CNF anchor = `master13-func_tc_messaging.adoc` (140 L, all TBD)
Names the interfaces **`I_EHR_EXTRACT`/`I_TDD`** (real names are
`I_EHR_EXTRACT_SERVICE`/`I_TDD_SERVICE`) and its 3 link anchors
(`#_i_ehr_extract_interface`, `#_i_tdd_interface`, `#_message_package`) all
dangle against `class_index.adoc`'s real anchors. 7 operation sections: 3 name
operations that DO NOT EXIST (`export_ehr()` twice — duplicate sections at
L51+L77 — and `export_ehr_extract()`), and the two real IMPORT operations
(`import_ehr`, `import_ehr_extract`) have NO section. No robot suite exists;
`CNF/tests/.../test_data_sets/compositions/TDD/` fixtures serve the
I_EHR_COMPOSITION `format=TDD` variant, not I_TDD_SERVICE.
Profiles (`CNF/docs/profiles/master03-profiles.adoc` L60-61): Messaging =
"EHR Extract" + "TDS", both **OPTIONS tier only**.

## Cross-chapter collision
`export_ehrs` is declared TWICE in the SM class set with different signatures:
`I_EHR_EXTRACT_SERVICE.export_ehrs(an_ehr_id: UUID[1]): List<EXTRACT>` (ch.9)
vs `I_ADMIN_DUMP_LOAD.export_ehrs(file_sys_loc: String[1], ...)` (ch.15), and
CNF master13/master12 reference both. Also near-homograph hazard:
SM `EXPORT_SPEC` (ch.15) vs RM `EXTRACT_SPEC` (ch.9 argument).
