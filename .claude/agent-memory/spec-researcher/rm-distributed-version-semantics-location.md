---
name: rm-distributed-version-semantics-location
description: Where RM master06 §6.4 "Semantics in Distributed Systems" lives (copy/import, version merging, disjoint merging, container moving), which class tables + cross-component sections it anchors to, and the confirmed released-text defects incl. the IMPORTED_VERSION-vs-branch contradiction and the total wire/CNF/SM silence
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

# RM master06 §6.4 "Semantics in Distributed Systems" — navigation

Owner file: `RM/docs/common/master06-change_control_package.adoc` **L242-334**
(§6.5 Class Descriptions starts L335). Real sub-headings (do NOT assume):

- §6.4.1 **Copying** (L244) → §6.4.1.1 **The Copy Operation** (L246-259),
  §6.4.1.2 **Subsequent Local Modifications** (L261-278). The two "systematic
  copying rules" are NOT their own sub-section — they are the bullet pair at
  **L275-276**, inside §6.4.1.2.
- §6.4.2 **Version Merging** (L280-298) — the provenance rule is **L296**.
- §6.4.3 **Disjoint Merging** (L300-323) — the 6-step procedure is the nested
  bullet list **L308-321**.
- §6.4.4 **Moving Version Containers** (L325-333) — 3 paragraphs, no procedure.

Amendment record confirms the numbering: `common/master00-amendment_record.adoc`
L38 (SPECRM-93 "section 6.4.2" = Version Merging).

## Class-table anchors
`RM/docs/UML/classes/org.openehr.rm.common.{imported_version,original_version,
versioned_object,version,contribution}.adoc` (note the file names have NO
`change_control` segment). Load-bearing: `versioned_object.adoc` L112-124
`commit_original_merged_version`, L127-132 `commit_imported_version`;
`original_version.adoc` L26-28 + L54 + L57; `imported_version.adoc` (item 1..1,
uid/preceding/lifecycle/data all *effected*).
`BASE/.../org.openehr.base.base_types.object_version_id.adoc` L23 creating_system_id.

## Cross-component anchors §6.4 depends on
- EHR-id re-use on copy: `RM/docs/ehr/master04-ehr_package.adoc` §EHR Creation
  Semantics (L203-233); the as-of-time consequence is restated at **L278**.
- Interoperable transport of a copied version: `RM/docs/ehr_extract/
  master05-openehr_extract_package.adoc` L30-40 (`X_VERSIONED_OBJECT`).
- Import audit/change_type + never-modify rationale: master06 §6.2.5 L65,
  §6.2.6 L84-86, §6.2.7 L104 (imported version signs the act of importing).

## The wire/SM/CNF silence (grep-verified 2026-08-03)
- ITS-REST **docs text**: zero occurrences of merge/import/copied anywhere in
  `ITS-REST/specifications/docs/**` (only an unrelated Simplified-Formats
  "Merge all nested structures").
- ITS-REST **OAS**: 97 operations in `ITS-REST/specifications/operations/`;
  NONE for import, merge, disjoint merge or move. IMPORTED_VERSION is
  produce-only (`schemas/ehr/UVersionOfComposition.yaml` oneOf +
  `UMImportedVersionOfComposition.yaml`); `other_input_version_uids` is
  produce-only (`schemas/ehr/OriginalVersion.yaml`); `NewContribution.versions`
  items are `UpdateVersion` (no `_type`, no `item`, no `other_input_version_uids`).
  The only §6.4-relevant write affordance is `operations/ehr_create_with_id.yaml`
  (PUT /ehr/{ehr_id}, 409 `responses/409_EHR_with_id.yaml`).
- **SM**: zero. **CNF**: zero (schedule + robot both; the only "merge"/"import"
  hits are licence boilerplate and a `s_feeder_audit.adoc` TODO).

## Confirmed released-text defects (upstream-report candidates)
- **L290** says the system-B local modification branch is "an instance of
  `IMPORTED_VERSION<T>`" — contradicts §6.2.3 L37, §6.4.1.2 L263 and the same
  section's own figure caption L271 (locally created content ⇒ ORIGINAL_VERSION).
  The strongest §6.4 defect.
- §6.4.1.1 L248-253 enumerates FOUR receiving situations, gives procedures for
  three; the duplicate-EHR case gets no procedure and no cross-reference to §6.4.3.
- §6.4.4 states only the `creating_system_id` consequence — no move procedure,
  no source-container disposition, and no statement that the branch-on-foreign
  rule (L240/L263) is suspended for moves (it must be: L329 keeps the trunk line).
- `EHR/master04` L217 misplaces master06 L255's "intentional clone" sentence
  into the *new*-globally-unique-id paragraph.
- `versioned_object.adoc` `Uid_validity: extension.is_empty` (unqualified — should
  be `uid.extension`); Pre-conditions name `a_preceding_version_uid`/`a_ver_id`
  while the signatures declare `a_preceding_version_id`/`a_version_uid`.
- Editorial: L271 "Users in system B an also make"; L327 code-font plural
  `VERSIONED_OBJECTS`; L248 "a `OBSERVATION`".

## Cross-entry use: the state-at-time anchor
§6.4.1.2 **L278** ("the commit times always reflect the local … act of committal
… a query for the state of a Version container at earlier commit times correctly
returns what information existed at that time") together with §6.2.5 **L90**
("`_time_committed_` … should reflect the time of committal to an EHR server,
i.e. the time of availability to other users in the same system") is the ONLY
released grounding for the extancy anchor of `VERSIONED_OBJECT.version_at_time`.
Cite these before calling at-time semantics wholly unassigned.
