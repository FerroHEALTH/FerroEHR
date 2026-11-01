---
name: versioned-object-read-ops-location
description: Where the VERSIONED_* read-op requirements live (ITS-REST ops/responses/schemas, RM VERSIONED_OBJECT/REVISION_HISTORY/VERSION class tables, SM I_EHR_STATUS + I_EHR_COMPOSITION) + the REVISION_HISTORY ordering defect, the wrapped-vs-bare-array body question, and the versioned_composition delta (path-param container uid, logical deletion / missing 204 branch, OAS `data` required conflict)
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

# VERSIONED_OBJECT read operations (versioned_ehr_status / versioned_composition) — file map

Companion to [[ehr-status-ops-location]] (the non-versioned EHR_STATUS ops +
the SM I_EHR_STATUS defect list) and [[its-rest-wire-contract-location]].

Applies to all four read shapes: `GET .../versioned_X`, `.../revision_history`,
`.../version?version_at_time=`, `.../version/{version_uid}`.

## ITS-REST (wire)
- **The per-API docs text is EMPTY of operation detail.**
  `ITS-REST/specifications/docs/ehr/Description.md` is 25 lines
  (purpose/status/links only) — there are NO per-operation prose sections for
  the EHR API. All cross-cutting wire rules come from
  `docs/overview/Requests_and_responses.md` + `docs/overview/Resources.md`;
  the per-op route/param/status set exists ONLY in the decomposed OAS
  (`specifications/operations|responses|parameters|schemas|headers/*.yaml`)
  and `specifications/ehr.openapi.yaml` (`servers: https://{baseUrl}/v1`,
  `security: []`).
- Ops: `operations/versioned_ehr_status_{get,revision_history,version_get_at_time,version_get_by_id}.yaml`
  (same naming for `versioned_composition_*`).
- Responses: `responses/200_VERSIONED_EHR_STATUS.yaml`, `200_REVISION_HISTORY.yaml`,
  `200_VERSION_of_EHR_STATUS_{at_time,by_id}.yaml`,
  `404_unknown_ehr_id[_or_no_version_at_time|_or_version_uid].yaml`, `400.yaml`.
- Schemas: `schemas/ehr/VersionedEhrStatus.yaml` → `schemas/common/VersionedObject.yaml`;
  `schemas/common/RevisionHistory.yaml` (+`RevisionHistoryItem.yaml`);
  `schemas/ehr/UVersionOfEhrStatus.yaml` (oneOf ORIGINAL/IMPORTED, `_type`
  discriminator, `_type` REQUIRED on the UM* variants).
- **ETag/Last-Modified rule is docs-text-only**, `Requests_and_responses.md`
  §ETag and Last-Modified: both SHOULD be on VERSION/VERSIONED_OBJECT
  responses, `W/` weak prefix mandatory, Last-Modified from
  `VERSION.commit_audit.time_committed`. `Last-Modified` appears in NO OAS
  file at all (grep-verified) — docs text governs.
- Datetime/timezone rule for `version_at_time`: `Resources.md` §Datetime format.

## RM (semantics + body attributes)
- `RM/docs/UML/classes/org.openehr.rm.common.versioned_object.adoc` — the only
  3 serializable attributes (uid HIER_OBJECT_ID / owner_id OBJECT_REF /
  time_created); `version_count`, `revision_history`, `latest_version` etc are
  FUNCTIONS (never wire fields). Invariant `Uid_validity: extension.is_empty`.
- `…common.revision_history.adoc` / `…revision_history_item.adoc` — **known
  released-text CONTRADICTION**: the class Description says the list is
  "in most-recent-first order", the `items` attribute meaning says
  "most-recent-last order", and both function postconditions use `items.last`
  → most-recent-LAST (oldest-first) wins 2:1. `audits.first` = the commit audit.
- `…common.version.adoc` / `…original_version.adoc` / `…imported_version.adoc` —
  VERSION envelope; `data` is 0..1 on ORIGINAL_VERSION (logical deletion).
- `RM/docs/common/master06-change_control_package.adoc` §Version Identification
  (`object_id::creating_system_id::version_tree_id`), §Committal and Audits,
  §Logical Deletion.
- Distinct class, do not confuse: `X_VERSIONED_OBJECT` (ehr_extract) DOES carry
  `total_version_count`/`revision_history` as attributes.

## ITS-JSON
- `ITS-JSON/components/RM/Release-1.1.0/Common/{REVISION_HISTORY,REVISION_HISTORY_ITEM,VERSIONED_OBJECT,ORIGINAL_VERSION}.json`.
- REVISION_HISTORY is a WRAPPED object `{items:[…]}`, `required:[items]` —
  never a bare array. (The CNF Robot suite asserts a bare array; it is a
  stalled non-authoritative guide, and ITS-REST amendment SPECITS-52
  "Fix wrong example on revision history of the VERSIONED_COMPOSITION and
  VERSIONED_EHR_STATUS" post-dates it.)
- **Gap:** there is NO `VERSIONED_EHR_STATUS`/`VERSIONED_COMPOSITION`
  definition in ITS-JSON; `VERSIONED_OBJECT.json` has `_type` const
  `VERSIONED_OBJECT` + `additionalProperties:false`.

## SM
- `SM/docs/UML/classes/i_ehr_status.adoc` (included by
  `SM/docs/openehr_platform/master05-ehr_service.adoc`). Functions:
  `get_versioned_ehr_status`, `get_ehr_status_at_time`,
  `get_ehr_status_at_version`, `has_ehr_status_version`. Only error listed
  anywhere: `ehr_id_does_not_exist`.
- **SM is entirely silent on revision history** (grep `revision_history` across
  `SM/docs/` = zero hits).
- SM defects to re-flag: `get_ehr_status_at_time` omits the `an_ehr_id`
  parameter its precondition uses; `get_versioned_ehr_status` carries a
  spurious `Pre_has_ehr_status_version` referencing an absent `a_version_uid`;
  `a_version_uid` is typed `UUID` though the wire id is an OBJECT_VERSION_ID.

## COMPOSITION-specific delta (vs the EHR_STATUS shape)
- Container uid is a **PATH param** here:
  `parameters/path/versioned_object_uid_COMPOSITION.yaml` (`format: uuid`) +
  `parameters/path/version_uid_COMPOSITION.yaml` (plain string). The three
  404 response files are `404_unknown_ehr_id_or_versioned_object_uid[_or_no_version_at_time|_or_version_uid].yaml`.
  Ops declare ONLY 200 + 404 — no 400, no 204.
- RM class: `RM/docs/UML/classes/org.openehr.rm.ehr.versioned_composition.adoc`
  — adds NO attributes, adds `is_persistent()` (function) + 2 invariants
  (`Archetype_node_id_valid`, `Persistent_validity`) that quantify over
  `all_versions` (never wire fields).
- Logical deletion: RM `common/master06-change_control_package.adoc`
  §Logical Deletion + §Version Lifecycle (523|deleted|, TERM
  `SupportTerminology/codesets/openehr_terminology-vocabularies.adoc`
  L41 audit-change-type 523 / L192 lifecycle-state 523).
  **The sibling `composition_get` op HAS a `204` branch
  (`responses/204_deleted_at_time.yaml`); the four versioned_composition
  ops have NONE** — the deleted-version read behaviour is a spec silence.
- **OAS-vs-RM/ITS-JSON conflict:** `schemas/ehr/Version.yaml` (and
  `VersionOfComposition.yaml`) put `data` in `required`, but RM
  `original_version.adoc` types `data` 0..1 and ITS-JSON
  `ORIGINAL_VERSION.json` required = contribution/commit_audit/uid/
  lifecycle_state (no data) → a logically-deleted ORIGINAL_VERSION is
  unrepresentable in the OAS schema only.
- SM: `SM/docs/UML/classes/i_ehr_composition.adoc` (included by
  `openehr_platform/master05-ehr_service.adoc`). Only `get_versioned_composition`
  maps 1:1; the version-envelope routes have no SM counterpart (the at_time/
  at_version functions return bare COMPOSITION = the `/composition/...` routes).
- `ITS-REST .../docs/overview/Resources.md` §Multiple identifiers is about the
  **`/composition/{uid_based_id}`** route, not the versioned_composition routes;
  its explicit-version example URL has a typo (`...example.com:5`, single colon).
- CNF: schedule `master07-func_tc_ehr_composition.adoc` §Get VERSIONED
  COMPOSITION = 3 cases for the container read only (none for
  revision_history / version-at-time / version-by-id); its `Test runners`
  links name robot files that do NOT exist (the vendored dir has
  `I_EHR_COMPOSITION/get_versioned_composition/C.6__A–D*.robot`).

## CNF
- Schedule `CNF/docs/platform_test_schedule/master06-func_tc_ehr.adoc`
  §EHR_STATUS Test Cases covers ONLY get_ehr_status + set/clear
  queryable/modifiable — NO test case for the four versioned read ops.
- Legacy Robot suite (stalled guide only):
  `CNF/tests/platform/robot/I_EHR_STATUS/get_versioned_ehr_status/C.6__A–D*.robot`.
