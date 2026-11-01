---
name: rm-master06-class-descriptions-location
description: Where RM master06 §6.5 "Class Descriptions" lives (5 include:: pulls, zero own prose), which class members the four semantic sections never touched (time_created / revision_history() / commit_attestation() / Attestations_valid), and the two NEW released-text defects found there (commit_imported_version has no signing_key; the SPEC-200 VERSION.contribution.type invariant is missing)
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

# RM master06 §6.5 "Class Descriptions" — navigation

Owner file: `RM/docs/common/master06-change_control_package.adoc` **L335-345**
(file is 345 lines; §6.4 ends L334). §6.5 has **ZERO prose of its own** — it is
exactly five `include::{uml_export_dir}/classes/{pkg}<x>.adoc[]` lines:
`versioned_object` (L337), `version` (L339), `original_version` (L341),
`imported_version` (L343), `contribution` (L345). Files are
`RM/docs/UML/classes/org.openehr.rm.common.<x>.adoc` (no `change_control`
segment).

**NOT included here** (so out of scope for a §6.5 unit): `AUDIT_DETAILS`,
`ATTESTATION`, `REVISION_HISTORY`, `REVISION_HISTORY_ITEM` — those belong to
the ch.5 `common` package chapter; the `<<_audit_details_class,…>>` xrefs in
these tables resolve at whole-`common.html` scope, not within master06.

## Coverage residue after §6.1–§6.4 (companions:
[[version-lifecycle-and-identification-location]],
[[rm-distributed-version-semantics-location]])
The semantic sections walked nearly every member. The only class-table members
that appear in NO §6.1–§6.4 checklist entry AND have ZERO occurrences in
master06 prose (grep-verified, all 0 hits in the chapter):
`VERSIONED_OBJECT.time_created`, `revision_history()`, `commit_attestation()`,
`version_count()`/`all_versions()`/`is_original_version()`/`version_at_time()`
(the latter four ARE covered as §6.3 S17), and
`ORIGINAL_VERSION.Attestations_valid`.
§6.2's issue record (#991) is a THEMED PROSE summary (~40 reqs over 8 themes:
Typing / Versioned Objects / Version subtypes / Virtual Version Tree /
Contributions / Committal and Audits / Digital Signature / Attestation) — NOT
an itemized list, so §6.2 cross-refs can only be made by theme.

## Two NEW released-text defects found in these tables (2026-08-03)
Both corroborated in `RM/computable/BMM/openehr_rm_1.2.0.bmm.json`, so neither
is an adoc rendering slip; both grep-clean in `ambiguities.yaml` and in the
issue tracker at time of writing:
- **`commit_imported_version` is the ONLY commit function with no
  `signing_key` parameter** (`versioned_object.adoc` L127-132; the other three
  carry it at L105/L120/L138) — yet §6.2.7 L104 requires the IMPORTED_VERSION
  to carry "its own signature which signifies the act of importing".
- **The `VERSION` invariant restricting `contribution.type` to "CONTRIBUTION"
  is MISSING.** `common/master00-amendment_record.adoc` L167 records SPEC-200
  as ADDING it; `version.adoc` + BMM carry only 3 invariants. Loss-class
  precedent: L100 SPECPUB-3 re-instated an inheritance "lost in original UML
  conversion". The sibling constraint survives on EHR
  (`org.openehr.rm.ehr.ehr.adoc` L58 `Contributions_valid`).
- Minor/editorial: IMPORTED_VERSION's `uid()`/`preceding_version_uid()` carry
  formal `Post:` clauses, `lifecycle_state()`/`data()` state the same
  derivation only in prose Meaning.

## Already-reported — do NOT re-flag
#1751 (six: `Lifecycle_state_ valid` embedded space, `Group_id_version_life_cycle_state`,
single-colon id pattern, `uid()` triple order, `other_input_version_ids`,
`Uid_validity` missing receiver), #1758 (four VERSIONED_OBJECT
precondition/parameter name mismatches + master04 clone sentence + 3 typos),
#1749, #1750, #1767, #1768, #1769. Register-covered: `canonical_form` [.tbd]
serialisation (~L8080-8130), empty-CONTRIBUTION `minItems` asymmetry (~L2413,
#1528), `owner_id` namespace/type wire values (~L1832), off-wire
VERSIONED_OBJECT function set (~L288-336, AMB-26 directory-scoped; AMB-72
~L1962 SM has no revision-history op).

## Wire anchors for the residue
- `ITS-REST .../schemas/common/VersionedObject.yaml` — `required: [uid,
  owner_id, time_created]`; ITS-JSON `VERSIONED_OBJECT.json` same trio +
  `additionalProperties:false`. So `time_created` IS wire-mandatory although
  master06 prose never mentions it.
- `X_VERSIONED_OBJECT` (`…ehr_extract.x_versioned_object.adoc`) carries
  `time_created` AND `revision_history` as real attributes — corroboration
  that both are load-bearing.
- **Zero ITS-REST operations for attestation** (`ls operations/ | grep -i
  attest` = empty) → `commit_attestation`'s Pre (attestations only on
  ORIGINAL_VERSION) is unobservable at the released surface.
- `RevisionHistoryItem.yaml` types `audits` as plain `AUDIT_DETAILS` items
  (no `oneOf` with ATTESTATION); `Attestation.yaml` is `allOf` AUDIT_DETAILS
  with `_type` enum `[ATTESTATION]`, and AuditDetails' `_type` is an open
  string — so attestations are spellable there but not schema-bound.
- CNF: zero hits for VERSIONED_OBJECT `time_created`; zero revision_history
  test cases in the schedule.
