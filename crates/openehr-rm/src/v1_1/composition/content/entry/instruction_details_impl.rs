// @generated-from-template templates/openehr-rm/composition/content/entry/instruction_details_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariant for `INSTRUCTION_DETAILS`.
//!
//! `Activity_path_valid` (`not activity_id.is_empty`) —
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.composition.instruction_details.adoc`
//! §Invariants.

use crate::v1_1::composition::content::entry::instruction_details::InstructionDetails;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for InstructionDetails {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        if self.activity_id.is_empty() {
            out.push(InvariantViolation::here(
                "Invariant Activity_path_valid failed on type INSTRUCTION_DETAILS",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openehr_base::v1_2::base_types::identification::locatable_ref::LocatableRef;
    use openehr_base::v1_2::base_types::identification::object_version_id::ObjectVersionId;
    use openehr_base::v1_2::base_types::identification::uid_based_id::UidBasedId;

    fn details(activity_id: &str) -> InstructionDetails {
        InstructionDetails {
            instruction_id: LocatableRef {
                namespace: "local".to_owned(),
                r#type: "COMPOSITION".to_owned(),
                id: UidBasedId::ObjectVersionId(
                    ObjectVersionId::new("abc::sys::1".to_owned())
                        .expect("a well-formed identifier"),
                ),
                path: None,
            },
            activity_id: activity_id.to_owned(),
            wf_details: None,
        }
    }

    #[test]
    fn valid_details() {
        assert!(details("/activities[at0001]").invariants().is_empty());
    }

    #[test]
    fn empty_activity_id_invalid() {
        assert_eq!(
            details("").invariants()[0].message,
            "Invariant Activity_path_valid failed on type INSTRUCTION_DETAILS"
        );
    }
}
