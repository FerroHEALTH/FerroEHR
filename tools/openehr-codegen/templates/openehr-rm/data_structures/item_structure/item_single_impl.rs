// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! `ITEM_SINGLE.as_hierarchy` (`org.openehr.rm.data_structures.item_single.adoc`).

use crate::v1_2::data_structures::item_structure::item_single::ItemSingle;
use crate::v1_2::data_structures::representation::element::Element;

impl ItemSingle {
    /// The single `ELEMENT` this structure carries (`as_hierarchy`).
    #[must_use]
    pub fn as_hierarchy(&self) -> &Element {
        &self.item
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::data_types::basic::data_value::DataValue;
    use crate::v1_2::data_types::text::dv_text::DvText;
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

    /// The hierarchy IS the carried element — same name, same node id, same
    /// value — not a copy shaped like one.
    #[test]
    fn as_hierarchy_is_the_carried_element() {
        let single = ItemSingle {
            name: text("single"),
            archetype_node_id: "at0000".to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            item: Box::new(Element {
                name: text("the element"),
                archetype_node_id: "at0001".to_owned(),
                uid: None,
                links: None,
                archetype_details: None,
                feeder_audit: None,
                null_flavour: None,
                value: Some(Box::new(DataValue::DvText(text("v")))),
                null_reason: None,
            }),
        };
        let hierarchy = single.as_hierarchy();
        assert_eq!(hierarchy.archetype_node_id, "at0001");
        assert!(std::ptr::eq(hierarchy, single.item.as_ref()));
    }
}
