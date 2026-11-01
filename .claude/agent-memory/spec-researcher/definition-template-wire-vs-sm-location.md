---
name: definition-template-wire-vs-sm-location
description: Where the ADL1.4/ADL2 template wire meets SM I_DEFINITION_ADL14/ADL2 — the op-by-op realization map, which SM ops DO have a wire (SPECITS-86 filters), the /example gap (SPECITS-58), and the identifier/pattern-language divergences
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

# Definition template API ↔ SM I_DEFINITION_* — realization map

Companion to [[adl2-rest-wire-contract-location]] (ADL2 wire shapes),
[[adl14-aom14-validity-location]] (ADL1.4 validity), [[sm-definition-package-location]]
(SM ch.4), [[template-metadata-version-location]] (the `version` field).

## The wire (9 template ops, `docs/specs/openehr/ITS-REST/specifications/operations/`)
`definition_template_adl1.4_{list,upload,get,example_get}.yaml` (4) +
`definition_template_adl2_{list,upload,get,example_get,version_get}.yaml` (5).

## SM→wire realization (re-verified first-hand 2026-08-21)
`SM/docs/UML/classes/i_definition_adl14.adoc` OPT half declares SEVEN ops:
`has_opt`, `valid_opt`, `upload_opt`, `get_opt`, `list_opts`,
`list_matching_opts`, `delete_opt`, `opts_count`.
- **DO have a wire:** `upload_opt`→`_adl1.4_upload`, `get_opt`→`_adl1.4_get`,
  `list_opts`/**`list_matching_opts`**→`_adl1.4_list`.
  **`list_matching_*` IS realized** — SPECITS-86 (11 Feb 2026, overview
  `Amendment_record.md` L50-55) added the `template_id` + `version` filter
  parameters to listTemplates on BOTH adl1.4 and adl2, so
  `definition_template_adl1.4_list.yaml` declares
  `filter_template_id` + `concept` + `filter_version` + `offset` + `fetch` —
  matching `list_matching_opts(id_pattern, item_offset, items_to_fetch)`.
  Any claim that `list_matching_opts` reaches no endpoint is FALSE.
- **Genuinely no wire:** `has_opt`, `opts_count`, `delete_opt` (no ADL1.4 DELETE
  route at all — see [[definition-artefact-delete-ownership]]), and `valid_opt`
  (only implicitly via upload validation).

## wire→SM gap: `/example` has NO service-model anchor
`definition_template_adl1.4_example_get.yaml` + `definition_template_adl2_example_get.yaml`
(added by **SPECITS-58**, "Add support for /example sub-resource under
template-definition endpoint", 17 Dec 2025, `Amendment_record.md` L57-63) realize
NO operation in either SM interface — neither `i_definition_adl14.adoc` nor
`i_definition_adl2.adoc` has any example/instance-generation function.
Params: `example_type.yaml` + `example_detail_level.yaml` (`required`/`medium`/
`complete`), `Accept_LOCATABLE`; responses 200/400/404/406.

## The two hard SM↔ITS-REST divergences on this surface
1. **Duplicate upload.** SM `i_definition_adl2.adoc` §upload_artefact:
   "Upload an ADL2 artefact, i.e. archetype, template or operational_template.
   **If an artefact with the same physical identifier and namespace exists,
   replace it.**" vs `definition_template_adl2_upload.yaml` declaring `'409':
   409_template_already_exists.yaml` ("`409 Conflict` is returned when a template
   with same `template_id` already exists"). Mutually exclusive.
   The ADL1.4 side is a SILENCE not a conflict: `upload_opt`'s Meaning is only
   "Upload an ADL 1.4 Operational Template (OPT)." with `Pre_valid` — no
   duplicate rule — while `definition_template_adl1.4_upload.yaml` ALSO declares
   the same 409.
2. **Filter pattern language.** SM `list_matching_archetypes`/`list_matching_opts`
   (adl14) and `list_matching_artefacts` (adl2) say only **"match a regex
   pattern"** and declare `.Errors * invalid_id_pattern`.
   **The "PERL regular expression" wording is NOT in those files** — it appears
   ONLY in `i_definition_query.adoc` (`_id_pattern_`/`_artefact_id_pattern_`
   Parameters block, the query-definition interface). Do not attribute PERL to
   the ADL interfaces.
   Wire side: `parameters/query/filter_template_id.yaml` "Pattern for matching
   `template_id` (supports wildcards `*`)" and `parameters/query/concept.yaml`
   "**PAttern** for matching `concept` (supports wildcards `*`)" (typo is in the
   released text). Both `*_list.yaml` ops declare a **single `200`** and no
   rejection branch — so `invalid_id_pattern` has no wire realization.

## Identifier divergence (ADL 1.4 OPTs)
SM keys OPTs by **UUID**: `has_opt(an_opt_id: UUID)`, `get_opt(an_opt_id: UUID)`,
`delete_opt(an_id: UUID)`, `list_opts(): List<UUID>` — while the sibling
`list_matching_opts(): List<ARCHETYPE_ID>` disagrees with all four inside the
same interface. The wire keys everything by the `template_id` string
(`parameters/path/template_id.yaml`) and `schemas/definition/TemplateMetadata.yaml`
exposes NO `uid` (props: template_id, version[deprecated], concept, archetype_id,
created_timestamp) — confirmed by `responses/200_TemplateList_adl1_4.yaml`'s own
example. So the SM key is unobtainable over the only released transport.

## The ADL1.4 partial-`template_id` promise with no grammar
`parameters/path/template_id.yaml`: "Template identifier or partial reference.
A partial `template_id` will resolve to “latest” major version of that template"
(curly quotes in the released text), with 4 `examples:` — `legacy` "Vital Signs",
`legacy_version` "vital_signs.v1", `hrid`
"org.highmed::openEHR-EHR-COMPOSITION.t_vital_signs.v1.0.0", `partial`
"openEHR-EHR-COMPOSITION.t_vital_signs.v1".
**AM defines no legacy template-id grammar at all: `grep -rn template_id
AM/docs/` returns ZERO hits.** And there is NO §Templates in
`AM/docs/ADL1.4/master02-overview.adoc` (its headings: What is ADL? / Structure /
An Example / Semantics / Computational Context / XML form of Archetypes / Changes
from Previous Versions) — the AM §Templates sections are
`AM/docs/Overview/master04-semantic_overview.adoc` L102 (prose only, no
identifier grammar), `AM/docs/ADL2/master10-templates.adoc` and
`AM/docs/AOM2/master10-templates.adoc`. The ADL2 side DOES have the grammar:
`AM/docs/UML/classes/org.openehr.am.aom2.archetype_hrid.adoc` (+ `p_archetype_hrid`).
