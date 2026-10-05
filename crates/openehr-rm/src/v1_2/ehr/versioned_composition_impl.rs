// @generated-from-template templates/openehr-rm/ehr/versioned_composition_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Where the `VERSIONED_COMPOSITION` spec function is realized — and why it is
//! not realized HERE (documentation only, by design).
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.ehr.versioned_composition.adoc`
//! §Functions declares one operation, `is_persistent ()`: "Indicates whether
//! this composition set is persistent; derived from first version."
//!
//! It is not a function of the generated value. `VERSIONED_COMPOSITION`
//! inherits `VERSIONED_OBJECT<COMPOSITION>`, whose three attributes are `uid`,
//! `owner_id` and `time_created` — the versions themselves are held by the
//! repository, not by the object (see the sibling `versioned_object_impl`).
//! "Derived from first version" therefore needs a read of the stored version
//! set that no in-memory value can supply, and the class's own §Invariants
//! spell the same dependence: `Persistent_validity` quantifies over
//! `all_versions`, the repository query.
//!
//! A conforming platform answers it in its persistence layer, from the first
//! committed version's `COMPOSITION.is_persistent`
//! (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.composition.composition.adoc`
//! §Functions), which IS a function of a value and is realized on
//! `COMPOSITION` itself.
