---
name: sm-ehr-service-chapter5-location
description: Where SM Platform ch.5 (EHR service) reference text lives — master05 is include-only, the §5.1 SVG is the ONLY source of the inheritance chain + commit_contribution [1..*], and which cross-cutting rules live in master02/master03
metadata:
  type: reference
---

Reference text only: the Service Model is not an authority (owner ruling
2026-10-27); it grounds no verdict and no feature.

# SM Platform ch.5 "EHR Service" — navigation

`SM/docs/openehr_platform/master05-ehr_service.adoc` is **30 lines, INCLUDE-ONLY**:
§Overview = ONE sentence + the package SVG; §Class Definitions = 9 `include::`
pulls, ZERO own prose. Every requirement therefore lives in
`SM/docs/UML/classes/`: `i_ehr_service`, `i_ehr`, `i_ehr_status`,
`i_ehr_directory`, `i_ehr_composition`, `i_ehr_contribution`, `ehr_summary`,
`uv_folder`, `uv_composition`. Feature counts: 9 / 4 attrs / 10 / 10 / 8 / 5.

## THE §5.1 DIAGRAM IS READABLE — and it is normative content the tables lack
`SM/docs/UML/diagrams/SM-platform.interface.ehr.svg` has **0 `<text>` elements
(394 `<path>`)**, but `rsvg-convert -w 2600` renders it fully legible (unlike the
RM lifecycle SVG). It is the ONLY source for:
- the inheritance chain **`I_STATUS` <- `I_VALIDITY_CHECKER` <- {I_EHR_SERVICE,
  I_EHR_DIRECTORY, I_EHR_STATUS, I_EHR_COMPOSITION, I_EHR_CONTRIBUTION}**
  (shared-trunk generalization; NO class table declares an `*Inherit*` row);
- `commit_contribution(versions: UPDATE_VERSION **[1..*]**)` — the tables say
  `List<UPDATE_VERSION>[1]`, so the diagram is the only non-empty constraint;
- explicit return multiplicities (`FOLDER [0..1]`, `EHR_SUMMARY [*]`, `UUID [*]`);
- `EHR_CALL_STATUS_TYPE` shown as a `CALL_STATUS_TYPE` specialisation.
Always rasterize this SVG before declaring ch.5 silent on structure.

## Cross-cutting rules that ch.5 inherits (NOT in master05)
- `master02-overview.adoc` §List Handling — the ONLY definition of
  `item_offset` (0-based, 0 = from first) / `items_to_fetch` (0 = 'all').
- §Global Naming Conventions — `_version_uid_` = `VERSION._uid.value_`
  (`uuid::system::N`) vs `_versioned_object_uid_` = plain UUID. This is the
  ground for every "typed UUID but must be OBJECT_VERSION_ID" defect.
- §Interface Calls — the atomicity/formal-equivalence rule ("transactionally
  protected", "any single call constitutes a self-standing transaction").
- §Anatomy of an Abstract Call Specification — the pre/post/exception template
  (its worked `create_ehr_with_id` example uses error names `Ehr_already_exists`
  / `Auth_error` that ch.5 and CALL_STATUS_TYPE do NOT use).
- §Functional Style — command/query separation; auth assumed already done;
  failures via `last_call_failed()`/`last_call_status()`.
- `master03-common_package.adoc` §Version Update Semantics — `preceding_version_uid`
  mandatory except first version; `lifecycle_state` always required;
  `time_committed`+`system_id` are SERVER-generated; ATTESTATION supplied in full.
- `i_validity_checker.adoc` defines `definitions_valid`/**`content_valid`** (ch.5
  preconditions misname the latter `valid_content` — 8 sites across ch.5+ch.6).

## Orphaned / unrendered class file
`ehr_call_status_type.adoc` (EHR-specific codes) is included by **NO chapter**
(grep: only `class_index.adoc` links it) — it never renders in the published
spec body although the §5.1 diagram displays it. Its `composition_archetype_invalid`
code is raised by NO operation anywhere.

## Error codes declared by ch.5 that exist in NO enumeration
`ehr_does_not_exist` (4 sites), `esubject_id_does_not_exist` (typo, 1),
`version_does_not_exist` (1), `versioned_composition_does_not_exist` (1),
`definition_unknown` + `content_invalid` (8 sites each, and NOT in
`definition_call_status_type.adoc` either). Enumerations live in
`call_status_type.adoc` / `ehr_call_status_type.adoc` / `definition_call_status_type.adoc`.

## Status / pin
`manifest_vars.adoc` → `:spec_status: TRIAL`; amendment record = "SM Release
1.0.0 (unreleased)", last entry 13 Dec 2021; vendored @ `23ffc4711c`. The
amendment record names `I_DEFINITIONS_CHECKER`/`DEFINITIONS_CHECKER_STATUS`
which do not exist in the class set (renamed to `I_VALIDITY_CHECKER`; no
`*_CHECKER_STATUS` file at all).

Per-interface defect lists already live in [[ehr-status-ops-location]],
[[composition-crud-ops-location]], [[directory-api-location]],
[[contribution-ops-location]]. CNF anchors: `CNF/docs/platform_test_schedule/`
master06 (I_EHR_SERVICE + I_EHR_STATUS), master07 (composition), master08
(contribution), master09 (directory); robot `CNF/tests/platform/robot/I_EHR_*`.
