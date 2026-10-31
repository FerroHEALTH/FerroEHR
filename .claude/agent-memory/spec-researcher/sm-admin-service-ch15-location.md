---
name: sm-admin-service-ch15-location
description: SM Platform ch.15 (Admin service) map — master15 is include-only, FOUR of its argument types are include-orphaned enumerations, the diagram-only export_fail_list attribute, EXPORT_SPEC referenced by nothing, and the ch.5 duplicate-name pairs
metadata:
  type: reference
---

Reference text only: the Service Model is not an authority (owner ruling
2026-10-27); it grounds no verdict and no feature.

# SM Platform ch.15 "Admin Service" — navigation

The SM-side companion of [[admin-api-location]] (the 2-op ITS-REST Admin API +
our catalogue/server state — do not duplicate it here); sibling of
[[sm-message-service-ch9-location]]; cross-cutting rules in
[[sm-ehr-service-chapter5-location]].

## File map
`SM/docs/openehr_platform/master15-admin_service.adoc` = **22 lines,
INCLUDE-ONLY** (one Overview sentence + the package SVG + 5 `include::`).
Included: `i_admin_service` (6 calls), `i_admin_archive` (2), `i_admin_dump_load`
(2), `dump_load_fail_report` (4 attrs), `export_spec` (4 attrs).
**NOT included but used as argument/attribute types:** `platform_service`,
`export_format`, `compression_format`, `encoding_format` — all four are
include-orphaned, so the published body carries dangling
`<<_platform_service_enumeration,…>>` (4 sites in i_admin_service),
`<<_export_format_enumeration,…>>`/`<<_compression_format_enumeration,…>>`/
`<<_encoding_format_enumeration,…>>` (3 in i_admin_dump_load + 3 in export_spec).
The literals are visible ONLY in the diagrams. master03 L9's own bullet list
promises `PLATFORM_SERVICE` and never includes it.

## Exact orphan census (computed, not guessed)
122 class files, 111 included by some SM document → **exactly 11 true orphans**:
`compression_format`, `ehr_call_status_type`, `encoding_format`, `export_format`,
`i_system_log`, `platform_service`, `result_query_descriptor`, `s_dv_boolean`,
`sp_variable_category`, `sp_variable_def`, `t`. FOUR of the 11 (not three) are
ch.15 types — `platform_service` is the fourth and the most load-bearing.

## Diagram-only content — rasterizes legibly
`SM/docs/UML/diagrams/SM-platform.interface.admin.svg` = 148 `<path>`,
0 `<text>`; `rsvg-convert -w 2600` legible. ONLY source for:
- `I_STATUS` <- {`I_ADMIN_SERVICE`, `I_ADMIN_ARCHIVE`, `I_ADMIN_DUMP_LOAD`}
  (shared-trunk generalization; no class file has an `Inherit` row);
- **`I_ADMIN_DUMP_LOAD.export_fail_list : DUMP_LOAD_FAIL_REPORT [*] {readOnly}`**
  — an entire attribute compartment the class TABLE omits; it is the only thing
  in the whole SM that references `DUMP_LOAD_FAIL_REPORT`;
- `archive_ehrs(ehr_ids: UUID [*])` / `archive_parties(party_ids: UUID [*])` and
  `list_contributions(...): UUID [*]` (tables say `List<UUID>[0..1]` / leading `0..1`);
- `EXPORT_SPEC`'s enum-typed features rendered as ASSOCIATIONS (roles
  `logical_format`/`compression_format`/`encoding`, all `0..1`) with
  `segment_split_size` as its only attribute; `ENCODING_FORMAT` box is EMPTY.
`SM-platform.common.svg` (master03) is the only rendering of PLATFORM_SERVICE's
8 literals: Admin, Definitions, Ehr, Ehr_index, Demographic, Message, Query,
System_log — **Terminology and Subject_proxy are missing** although both are
components in `SM-platform.definition.svg` and rows in master02's service table.

## Confirmed ch.15 defects (all first-hand)
- `EXPORT_SPEC` is referenced by **no signature and no attribute anywhere**
  (grep: only class_index + its own file) — `segment_split_size` is unreachable;
  `export_ehrs` takes the three enums loose instead of an EXPORT_SPEC.
- `I_ADMIN_DUMP_LOAD.load_ehrs` (a READ of the file system) declares error
  `file_not_writable`; `ENCODING_FORMAT` has zero members.
- `physical_ehr_delete`'s `Pre_has_ehr` names `I_EHR_SERVICE.has_ehr` — an
  interface I_ADMIN_SERVICE does not inherit. `physical_party_delete` has NO
  precondition although `I_PARTY.has_party` exists. ZERO postconditions in the
  whole chapter.
- Duplicate names vs ch.5: `list_contributions` + `contribution_count` also
  exist on `I_EHR_CONTRIBUTION` with different signatures (EHR-scoped, param
  `time_range`, and list_contributions there DOES carry `item_offset`/
  `items_to_fetch` — the admin, system-wide twin does not, contradicting
  master02 §List Handling). Amendment 0.9.5/SPECPR-304 removed only the
  `I_EHR` duplicate.
- Three rival count vocabularies: `EHR_SUMMARY.composition_count` vs
  `versioned_composition_count` vs `composition_version_count`.
- `.Parameters` blocks document only `a_service` (list_contributions) and
  `file_sys_loc` (export_ehrs); the 3 count calls document nothing.
- PLATFORM_SERVICE is one of exactly TWO SM enumerations with Capitalised
  literals (the other is RESOURCE_INSTANCE_TYPE, resource_instance_type.adoc
  L16-24); the remaining seven are lower_snake — never claim uniqueness
  (census 2026-08-21, the #2510 correction).

## CNF anchors
`CNF/docs/platform_test_schedule/master12-func_tc_admin.adoc` (160 L): 9
operation sections x 2 "aaaa"/"bbbb" cases, all TBD; **OMITS `load_ehrs`**;
`#_admin_package` anchor dangles (chapter is "Admin Service"), the 3 interface
anchors are correct. Profiles L53-58: 6 Admin capabilities (Activity Report,
Physical Deletion, EHR Dump/Load, Bulk EHR load, EHR Archive, Demographic
Archive), **OPTIONS tier only**, with no stated mapping to the 10 SM operations.
