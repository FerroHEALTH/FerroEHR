---
name: admin-api-location
description: Where the ITS-REST 1.1.0 Admin API (exactly 2 EHR-delete ops) lives, its DEVELOPMENT status, the SM I_ADMIN_SERVICE/I_ADMIN_ARCHIVE/I_ADMIN_DUMP_LOAD reference text (not an authority) + defects, the RM indelibility-vs-physical-delete tension, and the EHRbase-legacy Robot suite that is NOT the vendored Admin API
metadata:
  type: reference
---

# Admin API (ITS-REST 1.1.0) — file map

Companion to [[its-rest-wire-contract-location]], [[demographic-api-location]],
[[composition-crud-ops-location]].

## Route inventory — `ITS-REST/specifications/admin.openapi.yaml` (39 lines)
`info.x-status: DEVELOPMENT`, `version: development`, `security: []` (same as
every other group — security is NOT differentiated per group). **Exactly TWO
paths, both DELETE, both EHR-scoped:**
- `/admin/ehr/{ehr_id}` → `operations/admin_ehr_delete.yaml` (202/204/404)
- `/admin/ehr/all{?ehr_id*}` → `operations/admin_ehr_delete_all.yaml`
  (202/204/404/405)

NO admin composition/contribution/directory/template/party/query routes exist
in the vendored spec — those are EHRbase-legacy inventions.

## Docs prose = STUB
`docs/admin/Description.md` (28 lines: purpose / related docs / DEVELOPMENT
status). Zero per-operation prose. `admin` appears in the whole ITS-REST docs
text only in `overview/Specifications.md` (the API list),
`overview/Amendment_record.md` (SPECITS-80 "Add admin support to delete EHR",
14 Mar 2025) and `manifest.json`. All cross-cutting rules come from
`docs/overview/Requests_and_responses.md`.

## Admin-only wire facts (unique in the whole vendored OAS)
- `responses/202.yaml` and `responses/405.yaml` are referenced by admin
  operations ONLY. **202 is NOT in the overview's one normative status table**
  (covered by "Additional status codes MAY be used").
- `responses/204_deleted_hard.yaml` = "physically deleted (i.e. hard-delete)"
  — used only by the two admin ops.
- `parameters/query/ehr_id_Admin.yaml` — the only `style: form, explode: true`
  repeated query param; `required` absent (=false); schema is a SCALAR
  `string/uuid` while `examples.multiple` is an ARRAY (OAS-shape defect).
- `/admin/ehr/all{?ehr_id*}` is the ONLY RFC-6570 query-expansion path template
  in the vendored OAS (invalid OAS 3.0.3 path templating) and collides with
  `/admin/ehr/{ehr_id}` for the literal token `all`.

## SM reference text (not an authority) — `SM/docs/openehr_platform/master15-admin_service.adoc`
Full ch.15 map (calls, args, diagram-only features, the 11-orphan census, the
duplicate-name pairs) now lives in [[sm-admin-service-ch15-location]] — extend
that file, not this one. 10 operations across `I_ADMIN_SERVICE` (6),
`I_ADMIN_ARCHIVE` (2), `I_ADMIN_DUMP_LOAD` (2).
**Corrections to earlier notes:** the orphaned enumerations are FOUR, not three
(`platform_service` too — it types 4 of the 6 `I_ADMIN_SERVICE` calls);
`DUMP_LOAD_FAIL_REPORT` IS referenced, but only by a **diagram-only** attribute
`I_ADMIN_DUMP_LOAD.export_fail_list : DUMP_LOAD_FAIL_REPORT [*] {readOnly}` that
the class table omits; `EXPORT_SPEC` is the one genuinely referenced by nothing.
**`I_EHR_SERVICE` declares NO delete at all** — `physical_ehr_delete` is the SM's
only EHR deletion (grep-verified over every `delete*` in the SM class set), and it
takes `an_ehr_id: UUID [1]` — ONE mandatory id, no list form, no all-EHRs form —
so the released `admin_ehr_delete_all` route realizes NO SM operation.
**Do NOT write "nothing else in the SM is set-scoped"**: `i_query_service.adoc`
L20 + L43 (`execute_stored_query`, `execute_ad_hoc_query`) both take
`ehr_ids: List<UUID>[0..1]` and their `.Parameters` blocks spell out the
optional-means-all rule explicitly ("If none supplied, a full population query
will be performed on all EHRs whose status has the `is_queryable` flag set"),
which is a STRONGER statement of that shape than `I_ADMIN_ARCHIVE` gives.
The archive pair (`i_admin_archive.adoc`) is the only set-scoped *write*.

## RM tension (the load-bearing one)
`RM/docs/common/master06-change_control_package.adoc` §Logical Deletion (L192):
"information can only ever be logically deleted"; §Contributions (L58): "a
versioned repository … is by definition indelible".
**Indelibility is asserted in SIX places, not two** — also master06 L5
(properties list), `RM/docs/ehr/master04-ehr_package.adoc` L56 (a COMPOSITION
requirement bullet), and BOTH
`BASE/docs/architecture_overview/master07-security.adoc` L138 ("health record
information cannot be deleted") + L187, and
`BASE/docs/architecture_overview/master08-versioning.adoc` L16. So BASE
RE-STATES the principle and reconciles nothing.
**GDPR grep, corrected 2026-08-21 — SIX files, not two**: the two admin
operation descriptions (`ITS-REST/specifications/operations/admin_ehr_delete{,_all}.yaml`)
+ the three assembled `ITS-REST/computable/OAS/admin-{codegen,html,validation}.openapi.yaml`
bundles that repeat them verbatim + **one non-sentence hit previously missed**:
a `Consent (GDPR)` box label inside
`BASE/docs/architecture_overview/diagrams/shared_ehr.svg` (foreignObject text,
about consent, NOT about deletion). "Data protection" as a JUSTIFICATION for
erasure still appears in exactly two sentences — the two admin op descriptions.
Registered as AMB-10 (physical VERSIONED_OBJECT deletion undefined) — extend,
don't duplicate.

## CNF
- Schedule chapter = `CNF/docs/platform_test_schedule/master12-func_tc_admin.adoc`
  — pure SKELETON (Dependencies/Test Environment/Test Data Sets all TBD;
  9 SM-op sections × 2 cases named "aaaa"/"bbbb", every body "TBD"). It OMITS
  `I_ADMIN_DUMP_LOAD.load_ehrs` (only 9 of the 10 SM ops).
- `CNF/docs/profiles/master03-profiles.adoc` §Functional — all six Admin
  capabilities (Activity Report, Physical Deletion, EHR Dump/Load, Bulk EHR
  load, EHR Archive, Demographic Archive) AND "ADMIN API" are **OPTIONS tier
  only** (not CORE, not STANDARD).
- `CNF/tests/platform/robot/I_ADMIN_SERVICE/` (6 suites) is **EHRbase legacy,
  not the vendored Admin API**: Apache-2.0 Vitasystems/HMS headers, hits
  `${admin_baseurl}` = `http://localhost:8080/ferroehr/rest/admin`, asserts
  EHRbase's own PG table counts, and covers routes openEHR never defined
  (admin composition/contribution/directory/template delete + PUT template).
  **Unrunnable as vendored**: `${admin_baseurl}` is only a COMMENTED line in
  `_resources/suite_settings.robot` (L52), and `_resources/variables/` +
  most of `libraries/` are absent from the vendored tree.

## Our catalogue state (Veredictum's artifacts)
`schedule/admin/` has 18 cases across 9 SM ops (2 each); only
`physical_ehr_delete-{delete_existing,delete_non_existing}` have a realized
`bindings/its-rest/I_ADMIN_SERVICE.physical_ehr_delete.yaml`
(DELETE /admin/ehr/{ehr_id}; ok_empty=204 alt 202; not_found=404). The other 7
carry `unrealized:` bindings citing AMB-33. `load_ehrs` has no case at all —
it lives in `vocab/wire_surface.yaml` `sm_operations` as `off_wire`.
**BOTH gaps recorded here are CLOSED — re-verified 2026-08-21, do not re-cite
them:** `admin_ehr_delete_all` now has its own binding
(`bindings/its-rest/I_ADMIN_SERVICE.physical_ehr_delete-delete_all.yaml`, path
`/admin/ehr/all`) plus delete_all cases (subset / unfiltered / subset_repeated /
malformed_id / cascade); and `vocab/outcomes.yaml` DOES define `accepted`
(L70, class success) and `method_not_allowed` — added by the 2026-07-28 schedule
release (#544). Both admin bindings now declare `accepted: status: 202` AND keep
`ok_empty.alt_status: [202]` (a deletion-happened case must not refute an async
server). The residue is permanent, not pending: the released asynchrony is a
service-side MAY with **no client-selectable trigger**, so no case can drive the
202 arm (`vocab/wire_surface.yaml` element
`admin-delete-async-acceptance-branch` states it).

## Our server
Admin surface is `{base_path}/admin/**` = `/ferroehr/rest/openehr/v1/admin/**`
(NOT `/rest/admin` — that string survives only as a stale v1 reference in
`app/ferroehr-rest/src/extensions/access/authz/classify.rs` L13).
`app/ferroehr-rest/src/api/admin/{mod,dispatch,openapi_routes}.rs`: the two
spec routes plus three OWN EXTENSIONS (`DELETE /admin/template/{id}`,
`DELETE /admin/query/{name}/{version}`, `GET /admin/config`). Disabled group
answers **405 + empty `Allow`**; the "→ 404 when off" phrasings in
`dispatch.rs` L31 and `openapi_routes.rs` are STALE doc comments.
