---
name: is-modifiable-guard-location
description: Where the is_modifiable content-write guard is (and is not) defined across RM/SM/ITS-REST/CNF, incl. the total silence on evaluation timing, contribution member ordering, and the refusal status code
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

# `is_modifiable` content-write guard — where the requirements live

**The ENTIRE normative rule is two RM prose sentences + one class-table
sentence.** Everything about *enforcement* (when evaluated, what status code,
mixed-contribution ordering) is SILENT in every released component.

## The three (and only three) normative statements
1. `RM/docs/ehr/master04-ehr_package.adoc` §EHR Active Status (L235-244) —
   defines `is_modifiable` scope: "contents" = everything OTHER than
   EHR_STATUS, i.e. `compositions` + `folders` + future content. Also the
   deactivation-reason list and "An inactive EHR is still visible … will
   remain queryable".
2. Same file §EHR Creation (L213-224), the `_is_modifiable_` bullet — carries
   the parenthetical exemption "(the `EHR_STATUS` object itself is always
   modifiable)".
3. `RM/docs/UML/classes/org.openehr.rm.ehr.ehr_status.adoc` — class
   Description ("This object is always modifiable…") + the `is_modifiable`
   attribute Meaning ("True if the EHR, other than the `EHR_STATUS` object, is
   allowed to be written to. The `EHR_STATUS` object itself can always be
   written to."). The ONLY invariant on the class is `Is_archetype_root`.

## Confirmed SILENCES (greps run first-hand)
- `grep -rn modifiable ITS-REST/` → hits ONLY in `operations/ehr_create*.yaml`
  (default value) and `schemas/ehr/EhrStatus.yaml`. **No ITS-REST text anywhere
  mandates a status code for a write to a non-modifiable EHR.** Not in the
  status-code table (`docs/overview/Requests_and_responses.md` L215-240), not
  on `contribution_create` (only 400/404/409), not on composition ops.
- `grep -rn modifiable SM/` → only `i_ehr_status.adoc`
  (set/clear ops) + `i_ehr_service.adoc` (default status on 4 create ops).
  `I_EHR_CONTRIBUTION.commit_contribution` pre = `has_ehr` ONLY;
  `I_EHR_COMPOSITION.create/update_composition` pres = has_ehr +
  definitions_valid + valid_content ONLY. **No is_modifiable precondition
  exists anywhere in SM.** `CALL_STATUS_TYPE` has no not-modifiable code.
- No text anywhere defines WHEN the flag is evaluated relative to a commit.
- `CONTRIBUTION.versions` is `List<OBJECT_REF>` (BASE `List<T>` =
  "Ordered container that may contain duplicates") but NO text gives that
  order application/evaluation semantics or intermediate-state visibility.
  The only ordering-adjacent rule is the atomicity sentence in
  `RM/docs/common/master06-change_control_package.adoc` §Committal and Audits
  L92: "Contributions are similar to nested transactions. An attempt to commit
  a Contribution should only succeed if each Version and/or Attestation in the
  Contribution is committed successfully."

## CNF
- `CNF/docs/platform_test_schedule/master08-func_tc_ehr_contribution.adoc`
  §EHR_STATUS CONTRIBUTION Commit Data Sets (L190-230) — the 15-row matrix
  lists `is_modifiable = false` rows as **ACCEPTED** EHR_STATUS commits
  (corroborates the exemption). Rows 7-9 and 10-12 are a **duplicated block**
  (`false|true|…` twice) — the intended `false|false|…` variants are only
  partly present: a released-text defect.
- L335 NOTE (after `commit_contribution-valid_invalid_compositions`): "the
  whole commit should behave like a transaction and fail, no `CONTRIBUTIONs`
  or `VERSIONs` should be created on the server."
- **NO CNF case anywhere commits content to a deactivated EHR**, and no case
  mixes EHR_STATUS + COMPOSITION in one CONTRIBUTION. `master06-func_tc_ehr`
  §set/clear_ehr_modifiable cases only assert the flag round-trips.
