// @generated-from-template templates/openehr-rm/integration/generic_entry_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! `GENERIC_ENTRY` invariants (RM 1.2.0
//! `org.openehr.rm.integration.generic_entry.adoc`): the inherited
//! `Archetype_node_id_valid`.

use crate::v1_2::integration::generic_entry::GenericEntry;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for GenericEntry {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_2::validate::generated::archetype_node_id_core(
            "GENERIC_ENTRY",
            &self.archetype_node_id,
            out,
        );
    }
}
