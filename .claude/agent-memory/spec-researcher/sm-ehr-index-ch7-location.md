---
name: sm-ehr-index-ch7-location
description: Where SM Platform ch.7 (EHR Index service) reference text lives — master07 prose + 4 class files, the 5 write-only calls, the total absence of any READ call, and the zero ITS-REST/CNF footprint
metadata:
  type: reference
---

Reference text only: the Service Model is not an authority (owner ruling
2026-10-27); it grounds no verdict and no feature.

# SM Platform ch.7 "EHR Index Service" — navigation

Sibling of [[sm-ehr-service-chapter5-location]] (which owns the master02/master03
cross-cutting conventions every SM chapter inherits) and
[[sm-query-service-chapter8-location]].

## File map (small and total)
`SM/docs/openehr_platform/master07-ehr_index_service.adoc` = **31 lines**:
§Overview L5-20 (the ONLY prose; L11 primary-function/privacy rationale, L13-16
the two duplicate-error cases, L18 RESOURCE_STATUS rationale, L20 LOCATION_DESC
rationale) + §Class Definitions = 4 `include::` pulls, no own normative table.
Class files (`SM/docs/UML/classes/`): `i_ehr_index` (5 calls), `resource_status`
(4 attrs), `resource_instance_type` (3 literals), `location_desc` (**empty
class, zero attributes**).

## Diagram-only structure — rasterizes legibly
`SM/docs/UML/diagrams/SM-platform.interface.ehr_index.svg` = 99 `<path>`,
**0 `<text>`**; `rsvg-convert -w 2400` is fully legible. It is the ONLY source
for: (a) `I_EHR_INDEX` **inherits `I_STATUS`** directly (no `I_VALIDITY_CHECKER`
hop, unlike ch.5); (b) all 5 calls have **NO return type at all** (so the class
table's leading `0..1` column — elsewhere the return multiplicity — is
meaningless here); (c) `RESOURCE_STATUS -> RESOURCE_INSTANCE_TYPE` is a
composition, role `instance_type`, mult `1`; (d) `start_valid_time`/
`end_valid_time` genuinely have NO type in the source UML.

## The load-bearing silences (verify before claiming a read op exists)
- **I_EHR_INDEX has no read/query call whatsoever** — no get_subjects_for_ehr,
  no get_ehrs_for_subject, no has_*, no count. The chapter's own L11 ("the EHR
  Index *has to be used to obtain* the subject identifier") is unimplementable
  from the declared interface.
- The subject->EHR read exists instead on `i_ehr_service.adoc`
  (`has_ehr_for_subject`, `get_ehrs_for_subject`) keyed on **`PARTY_REF`**, and
  it resolves via `EHR_STATUS.subject` — the very coupling master07 L11 exists
  to avoid. I_EHR_INDEX types the same concept as the wider **`OBJECT_REF`**.
- `i_ehr_index.adoc` + `i_query_service.adoc` are the ONLY TWO interface files
  in the whole SM class set with **zero Pre_/Post_ conditions**
  (`grep -ln "Pre_" i_*.adoc` lists the other 14).
- `subject_id_does_not_exist` (4 sites) exists in NO enumeration
  (`call_status_type.adoc` has `party_id_does_not_exist`; no
  `INDEX_CALL_STATUS_TYPE` file exists). `ehr_id_does_not_exist` IS in
  `call_status_type.adoc`.
- `add_ehr_subject` carries NO `.Errors` block though it takes an ehr_id.
- `resource_status.adoc` L20+L24 are the ONLY two `@@` unresolved types in the
  entire SM class set (grep-verified) — start/end_valid_time have no type.
- **ZERO ITS-REST surface** (`grep -ril ehr_index ITS-REST/` = empty; the only
  subject-keyed op is EHR-API `operations/ehr_get_by_subject.yaml` with required
  `subject_id`+`subject_namespace`). **ZERO CNF footprint**: no chapter in
  `CNF/docs/platform_test_schedule/master.adoc`, no `I_EHR_INDEX` robot dir.
- Provenance: amendment record 0.9.3 "Add EHR Index section", 14 Feb 2018 —
  **never amended since**; `:spec_status: TRIAL`.
