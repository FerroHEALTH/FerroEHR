---
name: rm-common-master06-versioning-semantics
description: Verified findings for RM common master06 §6.3 (Versioning Semantics) — the 553|incomplete| relaxation stops at the archetype layer (check_mandatory_containers still 422s), lifecycle 523 + data is committable, non-COMPOSITION kinds get no relaxation, zero 800/801/branch/transition CNF coverage; plus what IS conformant in versioning/change.rs
metadata:
  type: feedback
---

Verified 2026-08-03 against RM 1.2.0
`docs/specs/openehr/RM/docs/common/master06-change_control_package.adoc`
lines 127–241 (§6.3 = Version Lifecycle [Incomplete Content · Abandoned and
Inactive States] · Logical Deletion · Version Identification [Local ·
Distributed]).

**CONFORMANT, do not re-check:**
- One commit engine: `versioning/change.rs::apply_change` is the ONLY write
  path; the only `UPDATE vo_version` statements touch `sys_period`
  (`storage/version_repo/commit.rs:269`, `import.rs:229/238`), the only
  `DELETE FROM vo_version` is the admin route (`service/admin/delete.rs:452`,
  AMB-88). So "any transition = a new version" holds structurally.
- Trunk numbering `tip.trunk + 1` under a per-vo advisory lock
  (`change.rs:342-348`, `storage/version_repo/placement.rs`), branch fork
  `t.(max_branch+1).1` on a foreign-`creating_system_id` tip (`change.rs:353-356`)
  — B29/B30/B33/B34, tested by `app/ferroehr/tests/it/service_branching.rs`.
- The B35 uniqueness tuple IS a DB constraint: `uq_vo_version_tree UNIQUE
  (vo_id, creating_system_id, trunk_version, branch_number, branch_version)`
  (`migrations/ehr/0001_baseline.sql`).
- `resource_etag` emits `W/"…"` (negotiate.rs:919); `strip_etag` accepts weak +
  bare (overview/version_id.rs:115). If-Match 400/412 + delete 409/400 have
  full CNF coverage (`*-missing_if_match`, `*-stale_if_match`,
  `delete_composition-{not_latest_version,already_deleted,etag_names_new_version}`).
- B20 unchanged-content-still-versions has a CNF case
  (`update_composition-same_content`).

**VERIFIED DEFECTS:**
- **The §Incomplete Content relaxation stops at the archetype layer.**
  `service/ehr/validation.rs:119` runs `validate_rm_and_terminology`
  UNCONDITIONALLY, and that reaches
  `crates/openehr-sdt/src/rm_instance/mod.rs:250 check_mandatory_containers`,
  which 422s an absent OR present-but-empty mandatory container — exactly what
  the §Incomplete Content NOTE says a 553 commit must tolerate ("container
  attributes may be empty, even though they may have minimum … cardinality …
  of one"). Repro: the CNF `cnf.composition.lab_result.cluster_no_items`
  fixture committed with `openehr-version: lifecycle_state.code_string="553"`.
  Only `flat::validation`'s `relax_lower_bounds` is gated on `incomplete`.
- **Only COMPOSITION gets the relaxation at all** —
  `service/ehr/validation.rs:214` passes `incomplete` to the Composition arm
  and drops it for EHR_STATUS/EHR_ACCESS/FOLDER/party kinds, though
  §Incomplete Content names "EHR Compositions, Demographic Parties etc".
- **A `523|deleted|` version can be committed WITH data.** `classify` routes
  change_type 523 → `Action::Delete` (data forbidden), but a change_type
  251 + `lifecycle_state: 523` (or the same via the `openehr-version` header on
  a direct PUT) lands in `Change::Modify` (`change.rs:540-583`) with node rows
  written; `validate_transition(COMPLETE→DELETED)` allows it. `read.deleted()`
  (`versioning/read.rs:126`) is lifecycle-only, so the resource then reads 204
  while its node rows stay queryable. §Logical Deletion's four-step procedure
  requires the data deleted.
- **`UPDATE_VERSION.lifecycle_state` is optional in practice**:
  `contribution.rs:972 lifecycle_of` returns `Option`, defaulting to 532 —
  SM master03 §Version Update Semantics says "must be supplied in all cases"
  and `UpdateVersion.yaml` marks it required.
- **The transition-legality 422 is invented and unregistered**:
  `versioning/lifecycle.rs:107 validate_transition` refuses e.g.
  `complete → incomplete`, `abandoned → complete` with a 422 naming
  "RM common master06 §Version Lifecycle state machine". No released text
  assigns a status code, there is no `ambiguities.yaml` entry, and NO CNF case
  pins it.
- **Zero end-to-end coverage of `800|inactive|` / `801|abandoned|`**, of any
  branch version id, and of any illegal-transition refusal — neither in
  Veredictum's `artifacts/schedule/` nor in the crate test suites.
- **`other_input_version_uids` accepted on the CONTRIBUTION wire**
  (`contribution.rs:377`) though `UpdateVersion.yaml` declares no such
  property. AMB-89 registers the IMPORT gap only, not the MERGE write path.
- **`VERSION_TREE_ID` invariant naming cites archie, not the spec**:
  `crates/openehr-base/src/base_types/identification/version_tree_id_impl.rs:8`
  ("Invariants (archie `VersionTreeId`)") and the emitted message
  `Value_format_valid` (:151) — a name that appears NOWHERE in the vendored
  specs. The six real BASE names (Trunk_version_valid, Branch_number_valid,
  Branch_version_valid, Branch_validity, Is_branch_validity,
  Is_first_validity) are never emitted.

**Spec-internal slips seen here (unregistered):** BASE `version.adoc` L72
spells the invariant `Lifecycle_state_ valid` (embedded space);
`versioned_object.adoc` L158 `Uid_validity: extension.is_empty` names no
receiver.

**Update 2026-10-14:** "523 + data committable" is CLOSED: versioning/lifecycle.rs reject_deleted_with_data (422) on direct commit and message/import.rs refuse it. See [[aql-row-grain-bindings]].
