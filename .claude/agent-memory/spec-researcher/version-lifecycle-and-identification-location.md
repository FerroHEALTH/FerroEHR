---
name: version-lifecycle-and-identification-location
description: Where RM master06 §6.3 Versioning Semantics lives (lifecycle state machine, logical deletion, version identification/branching), how to READ the lifecycle SVG (text is paths — rasterize), and the confirmed released-text defects around is_first/branching and commit-time data-required
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

# RM §6.3 "Versioning Semantics" — navigation

Owner file: `RM/docs/common/master06-change_control_package.adoc` (345 lines,
5 top-level `==` sections → §6.1 Overview, §6.2 Basic Semantics, **§6.3
Versioning Semantics (L127-241)**, §6.4 Semantics in Distributed Systems
(L242-334), §6.5 Class Descriptions). The amendment record confirms this
numbering (SPECRM-93 cites "section 6.4.2" = Version Merging).

§6.3 sub-sections: Version Lifecycle (L129) → Incomplete Content (L141),
Abandoned and Inactive States (L155); Logical Deletion (L190); Version
Identification (L201) → Local Versioning (L217), Distributed Versioning (L228).
**Copying / Version Merging / Disjoint Merging / Moving Version Containers are
§6.4, NOT §6.3** — a task that names "merging" as part of §6.3 is mis-scoped.

## The lifecycle state machine is only in an image
`RM/docs/UML/diagrams/RM-version_lifecycle.svg` — all text is rendered as SVG
**paths**, so grep/XML text extraction returns nothing. Read it with
`rsvg-convert -w 3200 <svg> -o out.png` then the Read tool (crop with
`magick`). The prose enumerates only the abandoned/inactive transitions; the
diagram additionally carries `create_draft`, `create_final`, `complete`,
`update` (self-loops + a COMPLETE→INCOMPLETE edge) and **`revert`
DELETED→INCOMPLETE / DELETED→COMPLETE**, which the prose never mentions and
which ITS-REST has no operation for (`400_already_deleted`).

## `incomplete` is NOT an under-specified state (settled 2026-08-21)
Do not report "commit/retrieval behaviour in `incomplete` is unspecified" —
**§Incomplete Content (L141-153) specifies it in detail**: L147 NOTE = "a limited
form of invalidity is allowed: mandatory attributes may be absent … single-valued
attributes may have null values and container attributes may be empty … All other
validity requirements must be satisfied … data may be missing, but it may not be
wrong"; L149 = same template/archetype "with all existence and cardinality lower
limits set to zero"; L151 = incomplete→complete requires missing fields populated
and generates a new VERSION regardless; L139 = every transition needs a new commit.
`RM/docs/ehr/master05-composition_package.adoc` §Composition Content L90 names
`VERSION._lifecycle_state_ = 'incomplete'` as the 'draft' empty-COMPOSITION case.
**The genuine gap is the EHR_STATUS axis, and it lives in CNF, not RM**:
`CNF/docs/platform_test_schedule/master08-func_tc_ehr_contribution.adoc` L203 —
"`CONTRIBUTIONs` with `VERSION`, where `VERSION.lifecycle_state` = `incomplete`
should be rejected, because the `incomplete` state doesn't apply to `EHR_STATUS`.
Though there is an open issue related to this: SPECPR-368". The RM carries NO
EHR_STATUS carve-out (`org.openehr.rm.ehr.{ehr_status,versioned_ehr_status}.adoc`
contain the string `lifecycle` ZERO times — grep-verified), and SPECPR-368 appears
in the whole vendored tree ONLY at that one CNF line.

## Class-table anchors §6.3 relies on
`RM/docs/UML/classes/org.openehr.rm.common.{version,original_version,imported_version,versioned_object}.adoc`;
`BASE/docs/UML/classes/org.openehr.base.base_types.{object_version_id,version_tree_id,uid_based_id,hier_object_id}.adoc`.
Terminology codes 532/553/523/800/801:
`TERM/docs/SupportTerminology/codesets/openehr_terminology-vocabularies.adoc`
§Version Lifecycle State (~L174-195).

## Confirmed released-text defects (upstream-report candidates)
- `VERSION_TREE_ID.is_first` = "trunk_version is 1" only, so `1.1.1` is
  `is_first` → collides with `VERSION.Preceding_version_uid_validity`
  (`uid.version_tree_id.is_first xor preceding_version_uid /= Void`), which
  then forbids a preceding version on the first version of branch 1.
- `ORIGINAL_VERSION.Is_merged_validity` names `other_input_version_ids`; the
  attribute is `other_input_version_uids`.
- `VERSION` invariant cites `Group_id_version_lifecycle_state`; the constant
  defined in `org.openehr.rm.support.openehr_terminology_group_identifiers.adoc`
  L72 is `Group_id_version_life_cycle_state`.
- `VERSION.uid()` doc gives the triple order `{object_id, version_tree_id,
  creating_system_id}`; the lexical form is `object_id::creating_system_id::version_tree_id`.
- master06 L205 writes the id pattern with SINGLE colons (`"1234:system_id:version_tree_id"`).
- Commit-time `data` is 1..1 in BOTH `SM/docs/UML/classes/update_version.adoc`
  and `ITS-REST .../schemas/{ehr,demographic}/UpdateVersion.yaml`, so the §6.3
  Logical Deletion procedure ("delete its `data`") is unrepresentable through
  a CONTRIBUTION commit; only the DELETE endpoints can produce it.
- `NewContribution.versions` items are `UpdateVersion` only → **no
  `other_input_version_uids` and no IMPORTED_VERSION can be submitted**;
  both are producible on version reads (`schemas/ehr/UVersionOfComposition.yaml`
  oneOf, `OriginalVersion.yaml`). Merge + import have no wire-in path.

## Wire + CNF anchors for §6.3 behaviour
- `ITS-REST .../docs/overview/Resources.md` §Identifier types (version_uid
  lexical form) + §Multiple identifiers; `Requests_and_responses.md`
  §openehr-version and openehr-audit-details (lifecycle_state on commit,
  server sets `time_committed`, server sets `system_id` when absent),
  §ETag and Last-Modified, §If-Match and accidental overwrites.
- `SM/docs/openehr_platform/master03-common_package.adoc` §Version Update
  Semantics — server constructs ORIGINAL_VERSION; preceding_version_uid
  required except first version; lifecycle_state required always.
- CNF (stalled guide): `master08-func_tc_ehr_contribution.adoc` L55-210 is the
  only change_type × lifecycle_state accept/reject matrix; `master07` L560-610
  the delete-as-new-deleted-version postcondition. **No CNF case anywhere
  exercises version_tree_id numbering, branching, creating_system_id, merge or
  import** (grep-verified).
