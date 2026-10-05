// @generated-from-template templates/openehr-rm/common/change_control/versioned_object_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Where the `VERSIONED_OBJECT` spec functions are realized — and why none of
//! them is realized HERE (hand-written spec behaviour; documentation only, by
//! design).
//!
//! Spec: RM
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.versioned_object.adoc`
//! §Functions declares sixteen operations on a version container:
//! `version_count`, `all_version_ids`, `all_versions`, `has_version_at_time`,
//! `has_version_id`, `version_with_id`, `is_original_version`,
//! `version_at_time`, `revision_history`, `latest_version`,
//! `latest_trunk_version`, `trunk_lifecycle_state`, plus the four committal
//! operations `commit_original_version`, `commit_original_merged_version`,
//! `commit_imported_version` and `commit_attestation`.
//!
//! NONE of them is a function of the generated value. `VERSIONED_OBJECT`
//! declares exactly three attributes — `uid`, `owner_id`, `time_created` — and
//! the versions themselves are not among them: the container's contents are
//! held by the repository, not by the object. Every function above therefore
//! needs repository context (a read of the version set, or a write into it)
//! that no in-memory `VersionedObject` value can supply, and an implementation
//! over the three attributes could only fabricate an answer — for instance a
//! `version_count` of zero on a container that in fact holds versions, which is
//! precisely the silent wrong answer this codebase refuses to produce.
//!
//! They are therefore repository operations by nature: a conforming platform
//! realizes the twelve query functions and the committal operations in its
//! own persistence/versioning layers, over the stored version set (see the SM
//! `I_EHR_CONTRIBUTION` / change-control semantics,
//! `docs/specs/openehr/RM/docs/common/master06-change_control_package.adoc`).
//!
//! NOTE: `commit_original_merged_version` has NO wire realization,
//! deliberately — the released REST wire declares no merge-commit shape
//! (`UpdateVersion.yaml` has no `other_input_version_uids` property), so
//! merge provenance is PRODUCE-only (`OriginalVersion.yaml` on reads,
//! reaching storage only through verbatim foreign reproduction — master06
//! §Copying); the absence is the conformant behaviour, not a gap.
//!
//! This module exists so that fact is written down beside the class it belongs
//! to, rather than left as an unexplained absence next to the sibling
//! `version_impl` / `original_version_impl` / `imported_version_impl` files —
//! whose functions ARE value-realizable and are realized there.
