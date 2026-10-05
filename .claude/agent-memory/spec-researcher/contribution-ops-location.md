---
name: contribution-ops-location
description: Where the ITS-REST CONTRIBUTION create/get requirements live (NewContribution/UPDATE_VERSION/UPDATE_AUDIT wire shape), the RM/SM/TERM grounding, and the confirmed released-text gaps + the AMB-57 staleness finding
metadata:
  type: reference
---

# CONTRIBUTION (2 ops) — where the requirements live

Routes in `ITS-REST/specifications/ehr.openapi.yaml` L89-94
(`/ehr/{ehr_id}/contribution` POST, `.../contribution/{contribution_uid}` GET).
EHR-API prose is a STUB (`docs/ehr/Description.md`) — see
[[composition-crud-ops-location]]. Only normative prose =
`docs/overview/{Requests_and_responses,Resources}.md`.

Ops: `operations/contribution_{create,get}.yaml` — BOTH carry long normative
`description:` blocks (Simplified-Formats rules, UPDATE_AUDIT `_type` handling,
uid/system_id/time_committed rules). These op descriptions are the ONLY place
several rules exist.

## The commit wire shape (1.1.0-new — do NOT assume the 1.0.x ORIGINAL_VERSION shape)
- Request body = `schemas/ehr/NewContribution.yaml` (`uid?`, `versions[]`,
  `audit`) — required `versions` + `audit`; **no `minItems`** on versions.
- `versions[i]` = `schemas/ehr/UpdateVersion.yaml` = UPDATE_VERSION:
  `preceding_version_uid? | signature? | lifecycle_state (req) | attestations? |
  data (req) | commit_audit (req)`. **No `change_type` on the version itself** —
  it lives in `commit_audit`. **No `uid`, no `contribution`** (server-minted).
  IMPORTED_VERSION is NOT reachable from the commit body.
- `audit` / `commit_audit` = `schemas/common/UpdateAudit.yaml` (change_type +
  committer required; optional `system_id`, `description`; `_type` SHOULD be
  `UPDATE_AUDIT`, servers SHOULD also accept `AUDIT_DETAILS`/omitted).
- Response/read body = `schemas/common/Contribution.yaml` (uid/versions/audit
  ALL required, `versions` `minItems: 1`, items = `ObjectRefOfObjectVersionId`,
  audit = full `AUDIT_DETAILS`). Example uses `type: COMPOSITION`/`FOLDER`
  (data type, not VERSION) + `namespace: local`.
- Responses: `201_CONTRIBUTION` (ETag_CONTRIBUTION + Location_CONTRIBUTION +
  Content-Type; body oneOf Contribution|Identifier), `200_CONTRIBUTION`,
  `400_CONTRIBUTION`, `404_unknown_ehr_id` (create), `404_CONTRIBUTION` (get),
  `409` (generic duplicate).

## Load-bearing released-text facts
- **SPECITS-84 landing did not settle AMB-57 — it CREATED the #1530
  contradiction**: the Amendment_record.md (27 Apr 2026) puts the wt MIME
  promise on both contribution ops, but the ops' read side carries no `data`
  member for a simplified body to live in, so the promise sits on operations
  whose response schema cannot express it. Never cite the landing as a
  dismissal of the ambiguity; the standing record is #1530.
- **AMB-54 is partially assigned after all**: `responses/400_CONTRIBUTION.yaml`
  says 400 covers "the modification type does not match the operation - i.e.
  first version of a MODIFICATION". The mirror case (creation WITH a
  preceding_version_uid) stays unassigned.
- **AMB-22 undercounts**: `has_contribution` AND `contribution_count` also have
  no ITS-REST wire, not just `list_contributions`.
- contribution_create has **NO 422** (composition_create does) and **NO 412 /
  If-Match**; contribution_get has **no Prefer** despite the generic
  `Prefer: return=representation, resolve_refs` prose (Requests_and_responses
  §Prefer resolving Object references).

## RM / SM / TERM grounding
- `RM/docs/common/master06-change_control_package.adoc` §Contributions (the 5
  logical change kinds + codes; the CONTRIBUTION.audit **aggregate** rule "not
  expected to be used as a computable value"), §Committal and Audits (the
  system_id/committer/time_committed **copy-down** rule; server-computed
  time_committed; the atomicity sentence "should only succeed if each Version
  and/or Attestation in the Contribution is committed successfully"),
  §Logical Deletion, §Copying (IMPORTED_VERSION).
- `RM/docs/UML/classes/org.openehr.rm.common.{contribution,version,
  original_version,imported_version,audit_details,attestation}.adoc` —
  CONTRIBUTION.versions is `1..1 List<OBJECT_REF>` with **no non-empty
  invariant**; AUDIT_DETAILS.description is **0..1 (NOT mandatory)**;
  VERSION invariant `Preceding_version_uid_validity:
  uid.version_tree_id.is_first xor preceding_version_uid /= Void`.
- `SM/docs/UML/classes/i_ehr_contribution.adoc` +
  `SM/docs/openehr_platform/master03-common_package.adoc` §Version Update
  Semantics + `SM/docs/UML/classes/{update_version,update_audit}.adoc`.
  SM names the version audit `audit`, ITS-REST names it `commit_audit`; SM has
  no `signature` on UPDATE_VERSION and no `system_id` on UPDATE_AUDIT.
- TERM `SupportTerminology/master04-representation.adoc` L74-82 = the
  audit_change_type group (249/250/251/252/523/666/253).

## CNF (stalled guide)
`CNF/docs/platform_test_schedule/master08-func_tc_ehr_contribution.adoc` — the
one_commit / multi-version decision tables + 4 SM-operation case families
(commit/list/has/get). Robot at `CNF/tests/platform/robot/I_EHR_CONTRIBUTION/`
(codes 201/400/422/404 from `_resources/keywords/contribution_keywords.robot`);
the JSON fixtures under `_resources/test_data_sets/contributions/` are the
LEGACY `_type: ORIGINAL_VERSION` + full AUDIT_DETAILS shape and several are
structurally corrupt (a whole CONTRIBUTION nested inside `versions[0].data`).

## committer on commit_audit (adjudicated for #3550, 2026-10-05)
- Required on BOTH `audit` and every `versions[i].commit_audit`: OAS
  `UpdateAudit.yaml` L12-14 + contribution_create desc L9-10 (server-assigned =
  ONLY time_committed + optional system_id) + SM master03 L23 (same two) + SM
  update_audit/update_version 1..1 + Amendment_record L22-28 SPECITS-95 (made
  system_id optional, left committer required = intent evidence).
- RM master06 §Committal and Audits L88 copy rule is a persisted-object
  lowercase "should" (RM declares no BCP14; master01-preface §Conformance L43-45
  defers wire conformance to ITSs) — NOT a wire relaxation; no RM/OAS conflict.
- Real silence: member committer != audit.committer (overwrite/keep/refuse) —
  nowhere specified. Header path (Requests_and_responses L81/L93) server-default
  committer is NOT transferable to the CONTRIBUTION body.
- CNF: master08 never mentions committer; all 47 CNF contribution JSONs carry it.
