// @generated-from-template templates/openehr-rm/composition/content/entry/evaluation_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariants for `EVALUATION`.
//!
//! Inherited `Entry` + LOCATABLE invariants (`Is_archetype_root`,
//! `Archetype_node_id_valid`). See `observation_impl` for the NOTE on the
//! deferred terminology-bound `Entry` invariants.

use crate::v1_1::composition::content::entry::evaluation::Evaluation;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for Evaluation {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_1::validate::generated::entry_root_core(
            "EVALUATION",
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
    use crate::v1_1::data_structures::item_structure::item_structure::ItemStructure;
    use crate::v1_1::data_structures::item_structure::item_tree::ItemTree;
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

    fn data() -> ItemStructure {
        ItemStructure::ItemTree(Box::new(ItemTree {
            name: text("tree"),
            archetype_node_id: "at0002".to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            items: openehr_base::containers::present(Vec::new()),
        }))
    }

    fn evaluation() -> Evaluation {
        Evaluation {
            name: text("Problem"),
            archetype_node_id: "openEHR-EHR-EVALUATION.problem.v1".to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: Some(Archetyped {
                archetype_id: ArchetypeId {
                    value: "openEHR-EHR-EVALUATION.problem.v1".to_owned(),
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
            data: data(),
        }
    }

    #[test]
    fn valid_evaluation() {
        assert!(evaluation().invariants().is_empty());
    }

    #[test]
    fn missing_archetype_details_invalid() {
        let mut e = evaluation();
        e.archetype_details = None;
        assert!(
            e.invariants()
                .iter()
                .any(|m| m.message == "Invariant Is_archetype_root failed on type EVALUATION")
        );
    }
}
