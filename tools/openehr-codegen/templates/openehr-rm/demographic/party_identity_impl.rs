// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! `PARTY_IDENTITY.purpose` (`org.openehr.rm.demographic.party_identity.adoc`).

use crate::v1_2::data_types::text::dv_text::DvText;
use crate::v1_2::demographic::party_identity::PartyIdentity;

impl PartyIdentity {
    /// The purpose of this identity: its `name` (`Purpose_valid: purpose = name`).
    #[must_use]
    pub fn purpose(&self) -> &DvText {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::data_structures::item_structure::item_structure::ItemStructure;
    use crate::v1_2::data_structures::item_structure::item_tree::ItemTree;
    use crate::v1_2::data_types::text::dv_text::DvTextData;

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

    fn identity(name: &str) -> PartyIdentity {
        PartyIdentity {
            name: text(name),
            archetype_node_id: "openEHR-DEMOGRAPHIC-PARTY_IDENTITY.person_name.v1".to_owned(),
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

    /// `Purpose_valid: purpose = name` — the function returns the name itself.
    #[test]
    fn the_purpose_is_the_name() {
        for name in ["legal", "nickname", ""] {
            let identity = identity(name);
            assert_eq!(identity.purpose(), &identity.name);
        }
    }
}
