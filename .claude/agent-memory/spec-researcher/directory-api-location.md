---
name: directory-api-location
description: Where the DIRECTORY (EHR FOLDER) 5-operation API + FOLDER RM + SM I_EHR_DIRECTORY + CNF live, and the enumerated released-text gaps (incl. FOLDER.items referential validity)
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

DIRECTORY / EHR FOLDER API spec locations (ITS-REST is OAS-split YAML; the EHR-API
docs prose is a STUB — see [[ehr-status-ops-location]]).

- Routes: `ITS-REST/specifications/ehr.openapi.yaml` L77-88 — `/ehr/{ehr_id}/directory`
  (post/put/delete/get) + `/ehr/{ehr_id}/directory/{version_uid}` (get). No `folders`
  route, no `versioned_directory` route, no FOLDER `tags` route (only composition +
  ehr_status have `/tags`).
- Operations: `operations/directory_{create,update,delete,get_at_time,get_by_version_id}.yaml`.
  Built bundle mirror: `computable/OAS/ehr-{validation,codegen,html}.openapi.yaml` L570-674
  (verified identical to the decomposed files).
- Responses: `201_directory`, `200_directory_updated`, `204_version_updated` (SHARED with
  ehr_status/composition/demographic updates — it, not the 200, declares
  `Location_version` + the two item-tag headers), `204_deleted`, `204_deleted_at_time`,
  `200_FOLDER_retrieved`, `412_directory`, `404_unknown_ehr_id`, `400`,
  `404_directory_unknown_ehr_id_or_no_version_{at_time,uid}_or_no_path`.
- Params: `parameters/query/{path,version_at_time}.yaml`, `path/{ehr_id,version_uid}.yaml`,
  `header/{If-Match,Prefer,Accept_LOCATABLE,ContentType_LOCATABLE}.yaml`.
- Headers: `headers/{ETag_FOLDER,ETag,Location_directory,Location_version,Location_deprecated,ContentType_LOCATABLE}.yaml`.
- Schema: `schemas/ehr/Folder.yaml` (+ `UMFolder.yaml`); items ->
  `schemas/base_types/UObjectRefOfUidBasedId.yaml` -> `ObjectRef.yaml` (namespace/type/id
  ALL required). `schemas/ehr/Ehr.yaml` carries **NO** directory/folders/compositions/
  contributions properties at all.

**THE `path` QUERY PARAM: its ONLY released definition is `parameters/query/path.yaml`**
("slash-separated values of the name attribute of FOLDERs in the directory",
example `episodes/a/b/c`) + SM `has_path` Meaning (same words). The overview docs
(`docs/overview/*.md`) mention directory/folder only 3 times and NEVER the path param —
grep-verified. RM's own directory paths are a DIFFERENT syntax
(`/folders[hospital episodes]/items[1]`, `RM/docs/common/master05-directory_package.adoc §Paths`).

RM grounding: `RM/docs/common/master05-directory_package.adoc` (VERSIONED_FOLDER =
VERSIONED_OBJECT<FOLDER>) + `RM/docs/UML/classes/org.openehr.rm.common.{folder,versioned_folder}.adoc`
(FOLDER declares NO invariants) + `RM/docs/ehr/master04-ehr_package.adoc §Folders` (L102-131)
and L237 (`is_modifiable` covers Folders) + `master06-change_control_package.adoc §Contributions`
(change_type 249/250/251/523) + `§Logical Deletion`. See [[folder-directory-model-location]].

SM: `SM/docs/UML/classes/i_ehr_directory.adoc` — 9 ops (has_directory, has_path,
create/update/delete_directory, get_directory, get_directory_at_time,
has_directory_version, get_directory_at_version, get_versioned_directory);
`uv_folder.adoc` (UV_FOLDER = UPDATE_VERSION<FOLDER>) + `update_version.adoc` +
`openehr_platform/master03-common_package.adoc §Version Update Semantics`.
`master05-ehr_service.adoc` is INCLUDE-ONLY (30 lines, zero directory prose).

**SM validity predicates (reference text only; SM's commit-time validation vocabulary):**
`create_directory`/`update_directory` carry `Pre_definitions_valid: definitions_valid(...)`
+ `Pre_content_valid: valid_content(...)`, errors `definition_unknown`/`content_invalid`.
Both predicates are DEFINED in `SM/docs/UML/classes/i_validity_checker.adoc`:
`definitions_valid` = archetype/template ids known in the definitions service;
`content_valid` = "content structure is a valid instance of the relevant RM classes".
Neither covers OBJECT_REF resolvability. **DEFECT: the interface calls it
`valid_content(...)`, I_VALIDITY_CHECKER declares `content_valid(...)`** — same mismatch in
i_ehr_composition / i_party / i_party_relationship / i_demographic_service.

**FOLDER.items referential validity = TOTAL SILENCE (verified first-hand, all 5 sources):**
- ITS-REST directory_create declares ONLY 201/400/404; directory_update 200/204/400/404/412.
  **NO 422 on any directory op** — `responses/422.yaml` is referenced ONLY by
  composition_create/update + the 8 demographic party create/update ops (grep-verified).
  No 409 on any directory op either.
- RM FOLDER.items Meaning: "The list of references to other (usually) versioned objects
  logically in this folder." FOLDER has no `Items_valid` invariant (SECTION, DV_PARAGRAPH,
  ATTESTATION all DO declare one) — historically it had some, removed by SPEC-49
  (`RM/docs/ehr/master00-amendment_record.adoc` L420).
- Contrast anchors that show the RM states scope rules when it means them:
  EHR class invariants (`org.openehr.rm.ehr.ehr.adoc` L57-77) constrain only `.type`, never
  existence; `EHR.tags` Meaning (L55) DOES say "Tag `_target_` values can only be within the
  same EHR" — no equivalent for folders/items. LINK class doc explicitly contemplates links
  "which can be broken when the extract is created".
- BASE OBJECT_REF Description: "may exist locally or be maintained outside the current
  namespace, e.g. in another service" + `base_types/master05-identification_package.adoc
  §References` (foreign-key analogy, distributed referencing).
- CNF master09 has only 3 create_ + 3 update_ cases (empty_ehr / ehr_with_directory /
  bad_ehr) — none about item refs; its §Tests of Reference FOLDER structure NOTE links a
  Discourse thread "what's allowed in FOLDER items", i.e. openly unsettled.
- **CNF Robot POSITIVE EVIDENCE for acceptance**: fixtures
  `_resources/test_data_sets/directory/{empty,subfolders_in}_directory*_items.json` +
  `update/3_add_items.json` all carry the SAME hardcoded, never-created uid
  `d936409e-901f-4994-8d33-ed104d46015b`, namespace `my.system.id`, type
  `VERSIONED_COMPOSITION`, and are used by delete_directory-ehr_with_directory (expects the
  create to succeed then 204), get_directory-directory_with_structure,
  get_directory_at_time-*, update_directory-ehr_with_directory (`validate PUT response - 200
  updated`). The keyword `validate POST response - 400 invalid content`
  (`directory_keywords.robot` L710) exists but is used by NO test and its own doc scopes it
  to "could not be converted to a valid directory FOLDER".
- ITS-REST `Requests_and_responses.md §Prefer resolving Object references` defines
  `Prefer: return=representation, resolve_refs` for READ but says nothing about an
  unresolvable ref.

CNF (stalled guide): `CNF/docs/platform_test_schedule/master09-func_tc_ehr_directory.adoc`
(cases C.1–L.3). Robot: `CNF/tests/platform/robot/I_EHR_DIRECTORY/**` +
`_resources/keywords/directory_keywords.robot` (1209 lines; the validate-* keywords carry the
concrete status codes; L730 literally says the 409-already-exists case "is not (yet) in the
SPEC"). Fixtures: `_resources/test_data_sets/directory/*.json`.

## "EHR has NO directory" on RETRIEVAL is *not* spec-silent (settled 2026-08-21)
Before registering an empty-vs-error ambiguity for directory GET, read the two
404 response FILES — their own sentences ground the outcome:
`ITS-REST/specifications/responses/404_directory_unknown_ehr_id_or_no_version_at_time_or_no_path.yaml`
= "`404 Not Found` is returned when an EHR with `ehr_id` does not exist, **or when
a directory does not exist** at the specified `version_at_time`, or when `path`
does not exist within the directory" (sibling `…_no_version_uid_…` says the same
for `version_uid`). `directory_get_at_time.yaml` declares ONLY 200/204/404 and
`200_FOLDER_retrieved.yaml` carries a `schemas/ehr/Folder.yaml` body — there is
**no empty-structure branch anywhere**, so the "empty structure" option has zero
released ground. The EHR-API docs text (`specifications/docs/ehr/Description.md`)
is a stub and `overview/Requests_and_responses.md` never mentions `directory`
(grep-verified), so the OAS legitimately fills that docs-text silence.
Residual gaps are only the WRITE paths (see the gap list below).

RELEASED-TEXT GAPS for directory (all confirmed first-hand):
create-on-existing-directory status; update/delete when EHR has no directory;
`path` leading-slash + root-name-included + escaping + items-addressable; the
`path`-miss branch on a 204-deleted version; `EHR.directory`/`folders` never on the wire;
committal + item-tag request headers absent from all 5 op files (docs text mandates them);
is_modifiable rejection status; body-`uid`-vs-If-Match mismatch; DELETE 204 declares no
headers at all; **FOLDER.items referential validity (no rule, no status code, no test case);
FOLDER.items `.type` value constraint (unlike EHR.compositions/folders, unconstrained)**.
