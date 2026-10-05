// @generated-from-template templates/openehr-rm/integration/generic_entry_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariants for `GENERIC_ENTRY`.
//!
//! Spec: RM 1.2.0 integration `master02-integration_package.adoc` +
//! `org.openehr.rm.integration.generic_entry.adoc` — GENERIC_ENTRY carries
//! only the generic `data: ITEM` (1..1, enforced structurally by the typed
//! deserialize) plus the inherited LOCATABLE duties
//! (`Archetype_node_id_valid`; the sibling `Links_valid` is structural and
//! `Archetyped_valid` runs in the `openehr-its` instance pass).

use crate::v1_1::integration::generic_entry::GenericEntry;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for GenericEntry {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_1::validate::generated::archetype_node_id_core(
            "GENERIC_ENTRY",
            &self.archetype_node_id,
            out,
        );
    }
}
