---
name: composition-crud-ops-location
description: Where the ITS-REST COMPOSITION CRUD (create/get/update/delete) requirements live, plus the confirmed released-text gaps (deleted-GET 204, openehr-template-id, DELETE 409-vs-If-Match) and SM I_EHR_COMPOSITION defects
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

# COMPOSITION CRUD (4 ops) — where the requirements live

Routes declared in `ITS-REST/specifications/ehr.openapi.yaml` L55-64
(`/ehr/{ehr_id}/composition`, `.../composition/{uid_based_id}`; base
`https://{baseUrl}/v1`, `security: []`). EHR-API prose is a STUB
(`docs/ehr/Description.md` = purpose/status/links only — see
[[ehr-status-ops-location]]), so the ONLY normative prose is
`docs/overview/{Requests_and_responses,Resources,Glossary_and_conventions}.md`.

Ops: `operations/composition_{create,get,update,delete}.yaml`.
Path params: `parameters/path/uid_based_id.yaml` (dual form, GET),
`uid_based_id_as_versioned_object_uid.yaml` (PUT, `format: uuid`),
`uid_based_id_as_version_uid.yaml` (DELETE).
Responses used: `201_COMPOSITION`, `200_COMPOSITION_{retrieved,updated}`,
`204_{deleted_at_time,version_updated,version_deleted}`, `400`,
`400_already_deleted`, `404_unknown_ehr_id{,_or_uid_based_id,_or_no_version_at_time}`,
`412_COMPOSITION`, `409_COMPOSITION_with_uid_based_id`, `422`.
Headers: `headers/{ETag,ETag_COMPOSITION,Location_COMPOSITION,Location_version,
Location_deprecated,ContentType_LOCATABLE,openehr-item-tag,openehr-version-item-tag}.yaml`.

## The four load-bearing released-text facts
- **uid_based_id dual form**: `Resources.md` §Multiple identifiers for the same
  resource — explicit version reference vs implicit latest version reference;
  "the implicit URI will only resolve to the same resource as the explicit
  versioned URI as long as no new versions are created".
- **DELETE is 409-gated, not If-Match-gated**: `409_COMPOSITION_with_uid_based_id`
  = "supplied `uid_based_id` doesn't match the latest version" — the PATH doubles
  as the precondition; `composition_delete.yaml` lists NO If-Match param and NO
  Prefer. Only PUT carries `If-Match` (required:true) → 412.
- **`openehr-template-id` exists ONLY in prose** (`Requests_and_responses.md`
  §openehr-template-id) — there is NO `parameters/header/openehr-template-id.yaml`
  and neither create nor update lists it. Same for `openehr-version` /
  `openehr-audit-details` (prose-mandated, absent from every operation file).
- **`openehr-version`/`openehr-audit-details`/`Prefer` semantics** all live in
  `Requests_and_responses.md` (see [[its-rest-wire-contract-location]]).

## Confirmed released-text SILENCES (flag, do not invent)
1. GET of a version_uid whose version is logically deleted: `composition_get`'s
   204 branch text is scoped to `version_at_time` (`204_deleted_at_time.yaml`);
   no branch defined for the by-version_uid deleted case. (CNF Robot
   `_resources/keywords/composition_keywords.robot` "get deleted composition"
   asserts 204 — stalled guide, NOT authority.)
2. No 409 on `composition_create` (persistent-twice is unspecified — see
   [[persistent-composition-uniqueness]]).
3. No status code for committing to an EHR with `is_modifiable=false`
   (RM ehr master04 §EHR Active Status defines the rule, ITS-REST assigns no code).
4. DELETE has no Prefer/Accept and no defined body; `204_version_deleted` carries
   only ETag + deprecated Location — ETag semantics on DELETE (deleted version vs
   preceding) not stated.
5. No 412 branch on DELETE; no 422 branch on DELETE.

## RM grounding
- `RM/docs/UML/classes/org.openehr.rm.composition.composition.adoc` — class table
  + 5 invariants (Category_validity, Territory_valid, Language_valid,
  Content_valid `content /= Void implies not content.is_empty`, Is_archetype_root).
- `RM/docs/ehr/master05-composition_package.adoc` — composer/event-context/content
  narrative; the "Persistent Compositions may optionally have an Event context" note.
- `RM/docs/common/master06-change_control_package.adoc` §Contributions (change_type
  249/250/251/523/666 per logical change) + §Logical Deletion (the 4-step procedure:
  new Version, data deleted, lifecycle_state=deleted, commit normally) + §Version
  Lifecycle (532/553/523/800/801). **523 is BOTH an audit_change_type code AND a
  version-lifecycle-state code** — TERM `SupportTerminology/master04-representation.adoc`
  L69-82 lists the `audit_change_type` + `composition_category` groups verbatim.

## SM `SM/docs/UML/classes/i_ehr_composition.adoc` — defects
- `get_composition_latest` precondition uses undeclared `a_version_uid`
  (param is `a_versioned_object_uid`).
- `delete_composition` types `a_version_uid` as `UUID` but text says it identifies
  the current top Composition Version (must be OBJECT_VERSION_ID).
- `get_composition_at_version` errors say `ehr_does_not_exist` (elsewhere
  `ehr_id_does_not_exist`).
- Preconditions call `valid_content(...)` but `i_validity_checker.adoc` declares
  `content_valid(...)` — name mismatch.
- No `preceding_version_uid` parameter on `update_composition` — CNF master07
  L475 explicitly calls this out as an SM spec gap.

## CNF (stalled guide)
`CNF/docs/platform_test_schedule/master07-func_tc_ehr_composition.adoc` — the
whole COMPOSITION suite (has/get_latest/at_time/at_version/versioned/create/
update/delete). Robot at `CNF/tests/platform/robot/I_EHR_COMPOSITION/`; the
status codes it asserts (create bad_opt→422, invalid→400, bad_ehr→404,
persistent-twice→400 [tagged `future`], delete→204, delete non-existent→404,
GET deleted→204) come from the keyword file, not the released spec.
