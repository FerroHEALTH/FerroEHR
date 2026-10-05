// @generated-from-template templates/openehr-rm/composition/content/entry/activity_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariants for `ACTIVITY`.
//!
//! - `Action_archetype_id_valid` (`not action_archetype_id.is_empty`) —
//!   `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.composition.activity.adoc`
//!   §Invariants.
//! - `Archetype_node_id_valid` — the inherited LOCATABLE invariant
//!   (`…org.openehr.rm.common.locatable.adoc` §Invariants).

use crate::v1_2::composition::content::entry::activity::Activity;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for Activity {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_2::validate::generated::activity_core(
            &self.action_archetype_id,
            &self.archetype_node_id,
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::data_structures::item_structure::item_structure::ItemStructure;
    use crate::v1_2::data_structures::item_structure::item_tree::ItemTree;
    use crate::v1_2::data_types::text::dv_text::{DvText, DvTextData};

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

    fn description() -> ItemStructure {
        ItemStructure::ItemTree(Box::new(ItemTree {
            name: text("tree"),
            archetype_node_id: "at0003".to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            items: openehr_base::containers::present(Vec::new()),
        }))
    }

    fn activity(action_archetype_id: &str) -> Activity {
        Activity {
            name: text("activity"),
            archetype_node_id: "at0001".to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            timing: None,
            action_archetype_id: action_archetype_id.to_owned(),
            description: description(),
        }
    }

    #[test]
    fn valid_activity() {
        assert!(activity("/.*/").invariants().is_empty());
    }

    #[test]
    fn empty_action_archetype_id_invalid() {
        let v = activity("").invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Action_archetype_id_valid failed on type ACTIVITY"),
            "got {v:?}"
        );
    }
}
