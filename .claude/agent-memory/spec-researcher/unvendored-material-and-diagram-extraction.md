---
name: unvendored-material-and-diagram-extraction
description: What spec material is NOT under docs/specs/openehr (CNF pseudo-code scripts, TERM XSDs, live governance pages) and how to read it read-only; plus the two different SVG label encodings (foreignObject vs outlined paths) and how to extract each
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

Navigation for the cases where "grep the vendored tree" returns nothing but the claim is
still checkable first-hand.

## Vendored-tree HOLES (a claim citing these cannot be verified by grepping docs/specs)
- **CNF pseudo-code scripts `scripts/openehr_platform/*.txt` (34 files) are NOT vendored,
  deliberately.** `scripts/vendor/spec-docs.sh` L86-92 carries the reason in-tree. To verify
  a claim about them, read them at the pin, read-only:
  `gh api "repos/openEHR/specifications-CNF/contents/scripts/openehr_platform?ref=<commit>" --jq '.[].name'`
  then per file `--jq '.content' | base64 -d`. The CNF pin lives in
  `docs/specs/openehr/CNF/PROVENANCE.md` (and the same table in the vendor script).
  Vendored CNF = `docs/` (test schedule) + `tests/` (Robot suites) only.
- **TERM computable XSDs are not under docs/specs** — `TERM/computable/XML/` holds only the
  `.xml` data + per-language dirs. The schemas (`PropertyUnitData.xsd`,
  `openehr_terminology.xsd`, `openehr_external_terminologies.xsd`) are vendored at
  `crates/openehr-term/assets/schema/`, byte-identical, upstream path
  `computable/XML/schema/*.xsd` per `crates/openehr-term/assets/PROVENANCE.md`.
  The terminology CODE SETS (e.g. `integrity check algorithms` = SHA-1/224/256/384/512/
  512-224/512-256, no `SHA-2`) are in
  `docs/specs/openehr/TERM/computable/XML/en/openehr_terminology.xml` (`<codeset openehr_id=…>`).
- **LANG ODIN `.bmm` schemas** (the ones that exercise P_BMM edge cases) live at
  `tools/openehr-codegen/vendor/bmm/components/LANG/odin/*.bmm`; the JSON BMMs (codegen input)
  at `tools/openehr-codegen/vendor/bmm/components/<COMP>/json/*.bmm.json` — RM has FIVE
  generations vendored (1.0.2 … 1.2.0), so "every published generation declares X" claims are
  re-derivable there rather than from an amendment record.
- **The openEHR release strategy is a live governance page, not vendored**:
  `curl -sS -L https://specifications.openehr.org/governance/release_strategy` works and
  carries verbatim "2nd position: used to indicate significant additions that do not change
  the semantics of the existing part of the release" + a "Major Changes" paragraph
  ("changes actually alter semantics of existing artefacts … a new major release is
  declared"). Use it for minor-vs-major compatibility arguments.

## SVG labels come in TWO encodings — check before concluding "text-free"
1. **foreignObject (draw.io export, RM diagrams)** — `<text>` elements exist but say
   `[Not supported by viewer]`; the real labels are HTML inside `<foreignObject>`. Extract
   with a regex over `<foreignObject.*?</foreignObject>` and strip tags (no rasterization
   needed). Example: `RM/docs/data_structures/diagrams/instance_item_table.svg` yields
   `CLUSTER name = "1" archetype_node_id = at0010|row|` ×3 — the row-encoding proof.
   Many RM diagrams also ship a sibling `.xml` (draw.io source) with the same labels.
2. **outlined paths (ALL SM diagrams)** — `<text>` count is 0 and `<path>` count is high
   (e.g. `SM-platform.interface.definitions.svg`: 0 text / 297 paths). Only rasterization
   reads them: `rsvg-convert -w 3200 <svg> -o out.png` (add `magick out.png -resize 1800x`
   for a whole-diagram read, `-crop` for detail). Every one of the 22 SM SVGs is 0-text.
   Facts recoverable ONLY this way (no `Inherit` row exists in any `SM/docs/UML/classes/i_*.adoc`):
   the `I_STATUS <- I_VALIDITY_CHECKER <- services` chain, `commit_contribution(… versions :
   UPDATE_VERSION [1..*] …)` (the class table says `List<UPDATE_VERSION>[1]`),
   `UV_COMPOSITION/UV_FOLDER/UV_PARTY/UV_PARTY_RELATIONSHIP = UPDATE_VERSION<T>` bindings,
   and the enumeration memberships (CALL_STATUS_TYPE 10 / EHR_CALL_STATUS_TYPE 6 /
   DEFINITION_CALL_STATUS_TYPE 7).

3. **outlined paths (ALL AM UML diagrams too)** — same 0-text shape; `rsvg-convert -z 2
   <svg> -o out.png` renders them fully legibly at ~2000px. **They are not decoration: the
   AM primitive-package diagrams carry attributes the CLASS TABLES omit** — e.g.
   `AM-aom14.archetype.constraint_model.primitive.svg` shows `C_DATE.timezone_validity`,
   which `org.openehr.am.aom14.c_date.adoc` and the AOM1.4 BMM both lack (and which the
   ITS-XML `AM/Release-1.4/Archetype.xsd` L283 also declares). Rasterize before concluding
   "the model does not declare X". See [[temporal-constraint-timezone-anchors]].

## SM orphan-class census (reproducible one-liner)
`for f in docs/UML/classes/*.adoc; do grep -rqF "classes/$(basename $f)" docs/*/*.adoc || echo $f; done`
run from `docs/specs/openehr/SM` → exactly **11** files included by none of the three books
(`openehr_platform/`, `serial_data_formats/`, `simplified_im_b/`): compression_format,
ehr_call_status_type, encoding_format, export_format, i_system_log, platform_service,
result_query_descriptor, s_dv_boolean, sp_variable_category, sp_variable_def, t.
`docs/openehr_platform/master.adoc` include list jumps master10 → master12 → master15.
See [[sm-admin-service-ch15-location]], [[sm-ehr-service-chapter5-location]].
