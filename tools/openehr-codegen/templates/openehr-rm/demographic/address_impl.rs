// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written RM spec functions for `ADDRESS`.
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.demographic.address.adoc`
//! §Functions + §Invariants.

use crate::v1_2::data_types::text::dv_text::DvText;
use crate::v1_2::demographic::address::Address;

impl Address {
    /// Returns the type of this address, e.g. electronic or locality.
    ///
    /// Spec: `org.openehr.rm.demographic.address.adoc` §Functions `type` —
    /// "Type of address, e.g. electronic, locality. Taken from value of
    /// inherited `name` attribute", with §Invariants `Type_valid: type = name`
    /// making the identity exact rather than merely usual. The name is
    /// borrowed, not copied: the invariant says the two ARE the same value.
    #[must_use]
    pub fn r#type(&self) -> &DvText {
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

    fn address(name: &str) -> Address {
        Address {
            name: text(name),
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

    /// `Type_valid: type = name` — the function returns the name itself, for
    /// every name, not a value derived from it.
    #[test]
    fn the_type_is_the_name() {
        for name in ["electronic", "locality", ""] {
            let address = address(name);
            assert_eq!(address.r#type(), &address.name);
        }
    }
}
