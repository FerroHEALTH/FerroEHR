// @generated-from-template templates/openehr-rm/composition/content/entry/instruction_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariants for `INSTRUCTION`.
//!
//! Inherited `Entry` + LOCATABLE invariants (`Is_archetype_root`,
//! `Archetype_node_id_valid`); `observation_impl` carries the account of the
//! ENTRY invariants this crate does not check.
//!
//! `INSTRUCTION`'s own `Activities_valid` (`activities /= Void implies not
//! activities.is_empty`,
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.composition.instruction.adoc`
//! §Invariants) needs no runtime check: `activities` is an
//! `Option<NonEmptyVec<ACTIVITY>>`, so present-but-empty is unrepresentable
//! (`openehr_base::containers`).

use crate::v1_1::composition::content::entry::instruction::Instruction;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for Instruction {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_1::validate::generated::entry_root_core(
            "INSTRUCTION",
            self.archetype_details.is_some(),
            &self.archetype_node_id,
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_1::common::archetyped::archetyped::Archetyped;
    use crate::v1_1::common::generic::party_proxy::PartyProxy;
    use crate::v1_1::common::generic::party_self::PartySelf;
    use crate::v1_1::data_types::text::code_phrase::CodePhrase;
    use crate::v1_1::data_types::text::dv_text::{DvText, DvTextData};
    use openehr_base::v1_2::prelude::{ArchetypeId, TerminologyId};

    fn text(value: &str) -> DvText {
        DvText::DvText(DvTextData {
            value: value.to_owned(),
            hyperlink: None,
            formatting: None,
            mappings: openehr_base::containers::present_nonempty(Vec::new()),
            language: None,
            encoding: None,
        })
    }

    fn code(terminology: &str, code: &str) -> CodePhrase {
        CodePhrase {
            terminology_id: TerminologyId {
                value: terminology.to_owned(),
            },
            code_string: code.to_owned(),
            preferred_term: None,
        }
    }

    fn instruction() -> Instruction {
        Instruction {
            name: text("Medication order"),
            archetype_node_id: "openEHR-EHR-INSTRUCTION.medication.v1".to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: Some(Archetyped {
                archetype_id: ArchetypeId {
                    value: "openEHR-EHR-INSTRUCTION.medication.v1".to_owned(),
                },
                template_id: None,
                rm_version: "1.1.0".to_owned(),
            }),
            feeder_audit: None,
            language: code("ISO_639-1", "en"),
            encoding: code("IANA_character-sets", "UTF-8"),
            other_participations: openehr_base::containers::present_nonempty(Vec::new()),
            workflow_id: None,
            subject: PartyProxy::PartySelf(PartySelf { external_ref: None }),
            provider: None,
            protocol: None,
            guideline_id: None,
            narrative: text("Take once daily"),
            expiry_time: None,
            wf_definition: None,
            activities: openehr_base::containers::present_nonempty(Vec::new()),
        }
    }

    #[test]
    fn valid_instruction() {
        assert!(instruction().invariants().is_empty());
    }

    #[test]
    fn missing_archetype_details_invalid() {
        let mut i = instruction();
        i.archetype_details = None;
        assert!(
            i.invariants()
                .iter()
                .any(|m| m.message == "Invariant Is_archetype_root failed on type INSTRUCTION")
        );
    }
}
