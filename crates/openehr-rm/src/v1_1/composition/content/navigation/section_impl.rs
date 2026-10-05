// @generated-from-template templates/openehr-rm/composition/content/navigation/section_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariant for `SECTION`.
//!
//! Only the inherited LOCATABLE `Archetype_node_id_valid` is checked.
//! `SECTION`'s own `Items_valid` (`items /= Void implies not items.is_empty`,
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.composition.section.adoc`
//! §Invariants) needs no runtime check: `items` is an
//! `Option<NonEmptyVec<CONTENT_ITEM>>`, so present-but-empty is
//! unrepresentable (`openehr_base::containers`).

use crate::v1_1::composition::content::navigation::section::Section;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for Section {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_1::validate::generated::archetype_node_id_core(
            "SECTION",
            &self.archetype_node_id,
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_1::data_types::text::dv_text::{DvText, DvTextData};

    fn section(node_id: &str) -> Section {
        Section {
            name: DvText::DvText(DvTextData {
                value: "section".to_owned(),
                hyperlink: None,
                formatting: None,
                mappings: openehr_base::containers::present_nonempty(Vec::new()),
                language: None,
                encoding: None,
            }),
            archetype_node_id: node_id.to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            items: openehr_base::containers::present_nonempty(Vec::new()),
        }
    }

    #[test]
    fn valid_section() {
        assert!(section("at0001").invariants().is_empty());
    }

    #[test]
    fn empty_node_id_invalid() {
        assert_eq!(
            section("").invariants()[0].message,
            "Invariant Archetype_node_id_valid failed on type SECTION"
        );
    }
}
