// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The `ITEM_LIST` functions (`org.openehr.rm.data_structures.item_list.adoc`);
//! an absent list reads as empty.

use crate::v1_2::data_structures::item_structure::item_list::ItemList;
use crate::v1_2::data_structures::representation::cluster::Cluster;
use crate::v1_2::data_structures::representation::element::Element;
use crate::v1_2::data_structures::representation::item::Item;
use crate::v1_2::data_types::text::dv_text::DvText;

impl ItemList {
    /// Count of all items (`item_count`).
    #[must_use]
    pub fn item_count(&self) -> usize {
        self.items.as_deref().unwrap_or_default().len()
    }

    /// The names of all items, in list order (`names`).
    #[must_use]
    pub fn names(&self) -> Vec<DvText> {
        self.items
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|item| item.name.clone())
            .collect()
    }

    /// The first item named `a_name`, or `None` (`named_item`).
    #[must_use]
    pub fn named_item(&self, a_name: &str) -> Option<&Element> {
        self.items
            .as_deref()?
            .iter()
            .find(|item| text_of(&item.name) == a_name)
    }

    /// The `i`-th item, from 1 (`ith_item`; BASE master02 §List).
    #[must_use]
    pub fn ith_item(&self, i: usize) -> Option<&Element> {
        self.items.as_deref()?.get(i.checked_sub(1)?)
    }

    /// One `CLUSTER` carrying this list's `ELEMENT`s (`as_hierarchy`), or `None`
    /// for an empty list, since `CLUSTER.items` is `1..*`.
    #[must_use]
    pub fn as_hierarchy(&self) -> Option<Cluster> {
        let items: Vec<Item> = self
            .items
            .as_deref()?
            .iter()
            .map(|element| Item::from(element.clone()))
            .collect();
        Some(Cluster {
            name: self.name.clone(),
            archetype_node_id: self.archetype_node_id.clone(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            // The only failure is an empty `items`, which is this function's
            // absent case rather than a defect — see the doc comment.
            items: openehr_base::containers::NonEmptyVec::new(items).ok()?,
        })
    }
}

/// The text of a name, whichever `DV_TEXT` form carries it.
fn text_of(name: &DvText) -> &str {
    match name {
        DvText::DvText(text) => &text.value,
        DvText::DvCodedText(text) => &text.value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::data_types::basic::data_value::DataValue;
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

    fn element(name: &str) -> Element {
        Element {
            name: text(name),
            archetype_node_id: "at0001".to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            null_flavour: None,
            value: Some(Box::new(DataValue::DvText(text(name)))),
            null_reason: None,
        }
    }

    fn list(names: &[&str]) -> ItemList {
        ItemList {
            name: text("list"),
            archetype_node_id: "at0000".to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            items: Some(names.iter().map(|n| element(n)).collect()),
        }
    }

    /// An absent `items` and an empty one are the same list: the page says the
    /// list may be empty, and `items` is `0..1`.
    #[test]
    fn an_absent_list_and_an_empty_list_agree() {
        let mut empty = list(&[]);
        assert_eq!(empty.item_count(), 0);
        assert_eq!(empty.names(), Vec::<DvText>::new());
        assert!(empty.named_item("x").is_none());
        assert!(empty.ith_item(1).is_none());
        assert!(empty.as_hierarchy().is_none());

        empty.items = None;
        assert_eq!(empty.item_count(), 0);
        assert_eq!(empty.names(), Vec::<DvText>::new());
        assert!(empty.named_item("x").is_none());
        assert!(empty.ith_item(1).is_none());
        assert!(empty.as_hierarchy().is_none());
    }

    #[test]
    fn item_count_and_names_follow_list_order() {
        let l = list(&["systolic", "diastolic"]);
        assert_eq!(l.item_count(), 2);
        assert_eq!(
            l.names().iter().map(text_of).collect::<Vec<_>>(),
            vec!["systolic", "diastolic"]
        );
    }

    /// `ith_item` is 1-based, so index 0 addresses nothing and `item_count()`
    /// addresses the last item.
    #[test]
    fn ith_item_is_one_based_at_both_boundaries() {
        let l = list(&["a", "b", "c"]);
        assert!(l.ith_item(0).is_none());
        assert_eq!(l.ith_item(1).map(|e| text_of(&e.name)), Some("a"));
        assert_eq!(l.ith_item(3).map(|e| text_of(&e.name)), Some("c"));
        assert!(l.ith_item(4).is_none());
    }

    /// Item names are not unique by any rule on the page, so a repeat is legal
    /// and the first in list order wins.
    #[test]
    fn named_item_takes_the_first_of_a_repeated_name() {
        let mut l = list(&["dose", "dose"]);
        if let Some(items) = l.items.as_mut()
            && let Some(second) = items.get_mut(1)
        {
            second.archetype_node_id = "at0002".to_owned();
        }
        assert_eq!(
            l.named_item("dose").map(|e| e.archetype_node_id.as_str()),
            Some("at0001")
        );
        assert!(l.named_item("absent").is_none());
    }

    /// The hierarchy is one `CLUSTER` carrying every `ELEMENT`, keeping the
    /// list's own name and node id.
    #[test]
    fn as_hierarchy_is_one_cluster_over_every_element() {
        let l = list(&["a", "b"]);
        let cluster = l.as_hierarchy().expect("a non-empty list has a hierarchy");
        assert_eq!(text_of(&cluster.name), "list");
        assert_eq!(cluster.archetype_node_id, "at0000");
        assert_eq!(cluster.items.len(), 2);
        assert!(
            cluster
                .items
                .iter()
                .all(|item| matches!(item, Item::Element(_)))
        );
    }
}
