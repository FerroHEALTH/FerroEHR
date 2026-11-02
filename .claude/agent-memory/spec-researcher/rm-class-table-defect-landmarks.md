---
name: rm-class-table-defect-landmarks
description: RM landmarks re-verified first-hand — ehr_extract version-spec silence, EXTRACT_UPDATE_SPEC's phantom attribute, ITEM_TABLE's duplicated function description, TRANSLATION_DETAILS.accreditaton, the nine RM-1.1.0-only BMM classes
metadata:
  type: reference
---

# RM class-table / chapter defect landmarks (all re-verified 2026-08-21)

- **EHR-Extract default version selection**: `RM/docs/ehr_extract/master04-common_package.adoc`
  **§Version Specification L107-112** — "the assumption of 'latest available
  version' for each matched item", and the ONLY modifiers named are
  `include_all_versions` / `include_revision_history`. Grep for
  trunk|branch across `ehr_extract/*.adoc` + all
  `org.openehr.rm.ehr_extract.*.adoc` = **zero hits**, while
  `org.openehr.rm.common.versioned_object.adoc` §Functions L84-89 defines BOTH
  `latest_version()` ("most recently added … on trunk or any branch") and
  `latest_trunk_version()`. Class-side default text:
  `…ehr_extract.extract_version_spec.adoc` L9 / `…extract_spec.adoc` L19.
- **`…ehr_extract.extract_update_spec.adoc`**: §Invariants L46
  `Send_changes_only_validity: send_changes_only implies persist_in_server`
  over an attribute the class never declares (L17-37 = persist_in_server /
  repeat_period / trigger_events / update_method). The intro L11 names the
  flag; `…extract_entity_manifest.adoc` L9 calls it "the send_changes_only value
  for `EXTRACT_UPDATE_SPEC._update_method_`". The RM 1.2.0 BMM carries the same
  four properties + the same three invariants verbatim.
- **`TRANSLATION_DETAILS.accreditaton`** (typo): fixed in the RM 1.2.0 BMM and class page at
  `specifications-RM` `09e481e9` (2026-11-02); the released RM 1.1.0 BMM keeps it. Before that it survived in RM: class page
  `org.openehr.rm.common.translation_details.adoc` L24 + the RM BMM (all
  serialisations, 1.0.2→1.2.0). BASE fixed it via SPECPUB-6
  (`BASE/docs/resource/master00-amendment_record.adoc` L49) — BASE 1.2.0/1.3.0
  spell `accreditation`, BASE 1.0.4/1.1.0 still carry the typo. **Zero XSD
  occurrences of the typo** in either ITS-XML lineage
  (`crates/openehr-its/schemas/xml/its-xml-1.0.2-nsv1/ALL/Resource.xsd` L23,
  `…its-xml-2.0.0-nsv2/AM/Release-1.4/Resource.xsd` L27, plus the BASE ones).
  RM amendment records never mention SPECPUB-6.
- **`…data_structures.item_table.adoc`**: `has_row_with_name` (L55-58) and
  `has_column_with_name` (L61-64) carry WORD-FOR-WORD identical descriptions
  ("there is a column with name = a_key"); `ith_row` L49-52 and
  `element_at_cell_ij` L85-89 state no index base; the only numbering rule is
  `RM/docs/data_structures/master04-item_structure_package.adoc` **L43** ("the
  names of the containing CLUSTER of each row is the stringified number of the
  row"); §Description's two structural rules are absent from §Invariants (only
  `Valid_structure`); `row_with_key`/`has_row_with_key` need "key columns" the
  class cannot designate. `BASE/docs/UML/classes/org.openehr.base.foundation_types.list.adoc`
  declares only `first`/`last` (ancestor `Container` has has/count/is_empty/
  quantifiers) — the indexed `item alias "[]"` lives on `Array`, NOT on List.
- **Nine classes exist ONLY in the RM 1.1.0 BMM**: `org.openehr.rm.composition.view`
  = CITATION/VIEW_ENTRY/VIEW_ITEM/VIEW_SECTION/VIEW_STATUS and
  `org.openehr.rm.resource` = CONSUMABLE_USE/RESOURCE_USAGE/RESOURCE_USE/
  SERVICE_USE. Word-grep of all nine across every vendored `*.adoc`/`*.md` = 0
  hits; present in no other component's BMM of any generation; dropped by RM
  1.2.0 with no amendment-record entry.

Related: [[rm-class-defs-location]], [[bmm-schema-validity-landmarks]],
[[version-lifecycle-and-identification-location]].
