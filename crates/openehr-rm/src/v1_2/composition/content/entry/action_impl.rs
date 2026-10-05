// @generated-from-template templates/openehr-rm/composition/content/entry/action_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariants for `ACTION`.
//!
//! Inherited `Entry` + LOCATABLE invariants (`Is_archetype_root`,
//! `Archetype_node_id_valid`). See `observation_impl` for the NOTE.

use crate::v1_2::composition::content::entry::action::Action;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for Action {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_2::validate::generated::entry_root_core(
            "ACTION",
            self.archetype_details.is_some(),
            &self.archetype_node_id,
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::common::archetyped::archetyped::Archetyped;
    use crate::v1_2::common::generic::party_proxy::PartyProxy;
    use crate::v1_2::common::generic::party_self::PartySelf;
    use crate::v1_2::composition::content::entry::ism_transition::IsmTransition;
    use crate::v1_2::data_structures::item_structure::item_structure::ItemStructure;
    use crate::v1_2::data_structures::item_structure::item_tree::ItemTree;
    use crate::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
    use crate::v1_2::data_types::text::code_phrase::CodePhrase;
    use crate::v1_2::data_types::text::dv_coded_text::DvCodedText;
    use crate::v1_2::data_types::text::dv_text::{DvText, DvTextData};
    use openehr_base::v1_3::prelude::{ArchetypeId, TerminologyId};

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

    fn coded(value: &str, terminology: &str, cs: &str) -> DvCodedText {
        DvCodedText {
            value: value.to_owned(),
            hyperlink: None,
            formatting: None,
            mappings: openehr_base::containers::present_nonempty(Vec::new()),
            language: None,
            encoding: None,
            defining_code: code(terminology, cs),
        }
    }

    fn action() -> Action {
        Action {
            name: text("Administer"),
            archetype_node_id: "openEHR-EHR-ACTION.medication.v1".to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: Some(Archetyped {
                archetype_id: ArchetypeId {
                    value: "openEHR-EHR-ACTION.medication.v1".to_owned(),
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
            time: DvDateTime {
                normal_status: None,
                normal_range: None,
                other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
                magnitude_status: None,
                accuracy: None,
                value: "2021-01-01T10:00:00".to_owned(),
            },
            ism_transition: IsmTransition {
                current_state: coded("active", "openehr", "245"),
                transition: None,
                careflow_step: None,
                reason: openehr_base::containers::present(Vec::new()),
            },
            instruction_details: None,
            description: ItemStructure::ItemTree(Box::new(ItemTree {
                name: text("tree"),
                archetype_node_id: "at0002".to_owned(),
                uid: None,
                links: openehr_base::containers::present_nonempty(Vec::new()),
                archetype_details: None,
                feeder_audit: None,
                items: openehr_base::containers::present(Vec::new()),
            })),
        }
    }

    #[test]
    fn valid_action() {
        assert!(action().invariants().is_empty());
    }

    #[test]
    fn missing_archetype_details_invalid() {
        let mut a = action();
        a.archetype_details = None;
        assert!(
            a.invariants()
                .iter()
                .any(|m| m.message == "Invariant Is_archetype_root failed on type ACTION")
        );
    }
}
