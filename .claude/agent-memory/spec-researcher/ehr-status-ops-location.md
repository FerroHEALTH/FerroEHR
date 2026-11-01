---
name: ehr-status-ops-location
description: Where the ITS-REST EHR_STATUS operation requirements live, the EHR-API docs-prose stub fact, RM EHR_STATUS class/invariant file, and the SM I_EHR_STATUS gaps + spec defects
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

# EHR_STATUS (3 ops) — where the requirements live

**ITS-REST EHR API prose is a STUB.** `ITS-REST/specifications/docs/ehr/Description.md`
is purpose/status/links ONLY — there are NO per-operation prose sections for the
EHR API (same for admin/definition/demographic/system; only `overview/` and
`query/` carry real prose). Consequence: for any EHR-API op the *only* normative
prose is `docs/overview/{Requests_and_responses,Resources,Glossary_and_conventions,
Preface}.md`; every route/param/status-branch/header detail is in the decomposed
OAS (`operations/`, `responses/`, `parameters/`, `headers/`, `schemas/`). Do not
go looking for an "EHR_STATUS section" in the docs text — it does not exist.

Routes are declared in `ITS-REST/specifications/ehr.openapi.yaml` (paths block,
~L35-42 + tags L124). Ops: `operations/ehr_status_{get_by_version_id,
get_at_time,update}.yaml`. Responses used: `200_EHR_STATUS_retrieved`,
`200_EHR_STATUS_updated`, `204_version_updated`, `400`,
`404_unknown_ehr_id{,_or_version_uid,_or_no_version_at_time}`, `412_EHR_STATUS`.
Headers: `headers/{ETag,Location_EHR_STATUS,Location_version,Location_deprecated,
ContentType_LOCATABLE,openehr-item-tag,openehr-version-item-tag}.yaml`. There is
**no `Last-Modified` header file at all** in `headers/` — Last-Modified is
prose-only (overview §ETag and Last-Modified, SHOULD).

RM: `RM/docs/UML/classes/org.openehr.rm.ehr.ehr_status.adoc` (class table; single
invariant `Is_archetype_root: is_archetype_root`; subject 1..1 PARTY_SELF).
Narrative: `RM/docs/ehr/master04-ehr_package.adoc` §EHR Status (L42-46),
§EHR Creation (L213-224), §EHR Active Status (L235-244) — the load-bearing rule
that `is_modifiable=false` blocks EHR *contents*, never the EHR_STATUS object
itself. `VERSIONED_EHR_STATUS` class table is a 3-line stub (inherits
VERSIONED_OBJECT — go to `org.openehr.rm.common.versioned_object.adoc` for
`commit_original_version` pre `all_version_ids.has(a_preceding_version_uid) or
else version_count = 0`).

## SM I_EHR_STATUS — gaps + confirmed spec defects
`SM/docs/UML/classes/i_ehr_status.adoc` (included by
`SM/docs/openehr_platform/master05-ehr_service.adoc`, which is include-directives
only — no prose).
- **GAP: no whole-object update operation.** Only granular
  `set_/clear_ehr_{queryable,modifiable}` + `update_other_details`. The REST
  `PUT /ehr_status` (full replacement, incl. `subject`) has NO SM counterpart.
- DEFECT: `get_ehr_status_at_time` declares only `a_time` but its precondition
  is `has_ehr (an_ehr_id)` — `an_ehr_id` is undeclared.
- DEFECT: `get_versioned_ehr_status` precondition
  `has_ehr_status_version (an_ehr_id, a_version_uid)` — `a_version_uid` undeclared.
- DEFECT: version uids typed `UUID`, but a `version_uid` is an
  OBJECT_VERSION_ID (`object_id::creating_system_id::version_tree_id`).
- DEFECT: `clear_ehr_modifiable` (L104-110) meaning reads "ensures it is treated
  as active" (should be inactive) while its own post is
  `not …is_modifiable` — but its spelling of `_is_modifiable_` is CORRECT.
  The `_is_modifable_` misspelling is in **`set_ehr_modifiable` (L80)**, a
  different operation — do not attribute the typo to `clear_ehr_modifiable`.
  Line map: L49 missing `an_ehr_id`, L80 typo, L110 contradiction, L154
  undeclared `a_version_uid`.
- SM has NO `revision_history` operation anywhere (grep `revision_history`
  under `SM/docs/` = zero hits), while ITS-REST declares
  `operations/versioned_ehr_status_revision_history.yaml`; the at-time/at-version
  SM ops return `EHR_STATUS` while the REST routes serve the VERSION envelope
  (`responses/200_VERSION_of_EHR_STATUS_{at_time,by_id}.yaml`).
- Only declared error across all ops: `ehr_id_does_not_exist`.

## CNF coverage (stalled guide, not authority)
`CNF/docs/platform_test_schedule/master06-func_tc_ehr.adoc` §EHR_STATUS Test
Cases (L268+) — cases exist ONLY for get_ehr_status (ok/bad_ehr) and the four
flag setters/clearers; robot at `CNF/tests/platform/robot/I_EHR_STATUS/`.
**No case for**: version_at_time, get-by-version_uid, If-Match/412, Prefer
variants, ETag. The 16-row valid EHR_STATUS data-set matrix is at L45-70 of the
same file (see [[cnf-test-case-format-location]]).
