// @generated-from-template templates/openehr-rm/demographic/contact_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! `CONTACT.purpose` (`org.openehr.rm.demographic.contact.adoc`).

use crate::v1_1::data_types::text::dv_text::DvText;
use crate::v1_1::demographic::contact::Contact;

impl Contact {
    /// The purpose of this contact: its `name` (`Purpose_valid: purpose = name`).
    #[must_use]
    pub fn purpose(&self) -> &DvText {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_1::data_structures::item_structure::item_structure::ItemStructure;
    use crate::v1_1::data_structures::item_structure::item_tree::ItemTree;
    use crate::v1_1::data_types::text::dv_text::DvTextData;
    use crate::v1_1::demographic::address::Address;

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

    fn address() -> Address {
        Address {
            name: text("electronic"),
            archetype_node_id: "openEHR-DEMOGRAPHIC-ADDRESS.address.v1".to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            details: ItemStructure::ItemTree(Box::new(ItemTree {
                name: text("tree"),
                archetype_node_id: "at0001".to_owned(),
                uid: None,
                links: None,
                archetype_details: None,
                feeder_audit: None,
                items: None,
            })),
        }
    }

    fn contact(name: &str) -> Contact {
        Contact {
            name: text(name),
            archetype_node_id: "openEHR-DEMOGRAPHIC-CONTACT.contact.v1".to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            addresses: openehr_base::containers::NonEmptyVec::of(address()),
            time_validity: None,
        }
    }

    /// `Purpose_valid: purpose = name` — the function returns the name itself.
    #[test]
    fn the_purpose_is_the_name() {
        for name in ["mail", "daytime phone", ""] {
            let contact = contact(name);
            assert_eq!(contact.purpose(), &contact.name);
        }
    }
}
