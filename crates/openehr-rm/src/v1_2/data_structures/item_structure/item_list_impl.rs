// @generated-from-template templates/openehr-rm/data_structures/item_structure/item_list_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM spec functions for `ITEM_LIST`.
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_structures.item_list.adoc`
//! §Functions. `items` is `0..1` and the page's §Description says "The list may
//! be empty", so every accessor treats an absent list and an empty one alike.

use crate::v1_2::data_structures::item_structure::item_list::ItemList;
use crate::v1_2::data_structures::representation::cluster::Cluster;
use crate::v1_2::data_structures::representation::element::Element;
use crate::v1_2::data_structures::representation::item::Item;
use crate::v1_2::data_types::text::dv_text::DvText;

impl ItemList {
    /// Count of all items.
    ///
    /// Spec: `item_list.adoc` §Functions `item_count`.
    #[must_use]
    pub fn item_count(&self) -> usize {
        self.items.as_deref().unwrap_or_default().len()
    }

    /// The names of all items, in list order.
    ///
    /// Spec: `item_list.adoc` §Functions `names`.
    #[must_use]
    pub fn names(&self) -> Vec<DvText> {
        self.items
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|item| item.name.clone())
            .collect()
    }

    /// The item named `a_name`, or `None` when no item carries that name.
    ///
    /// Spec: `item_list.adoc` §Functions `named_item`. The first match wins:
    /// nothing on the page makes item names unique within a list, so a repeated
    /// name is a legal structure and list order is the only tie-break the model
    /// offers.
    #[must_use]
    pub fn named_item(&self, a_name: &str) -> Option<&Element> {
        self.items
            .as_deref()?
            .iter()
            .find(|item| text_of(&item.name) == a_name)
    }

    /// The `i`-th item, counting from 1.
    ///
    /// Spec: `item_list.adoc` §Functions `ith_item`. The index is 1-based, as
    /// everywhere else the RM indexes a container — BASE `foundation_types`
    /// master02 §List, whose `i_th` is defined over `1 .. count`. `i = 0` and
    /// any index past the end are `None`.
    #[must_use]
    pub fn ith_item(&self, i: usize) -> Option<&Element> {
        self.items.as_deref()?.get(i.checked_sub(1)?)
    }

    /// A CEN EN13606-compatible hierarchy: one `CLUSTER` carrying this list's
    /// `ELEMENT`s.
    ///
    /// Spec: `item_list.adoc` §Functions `as_hierarchy` — "Generate a CEN
    /// EN13606-compatible hierarchy consisting of a single `CLUSTER` containing
    /// the `ELEMENTs` of this list."
    ///
    /// `None` for an empty list: `CLUSTER.items` is `1..*`
    /// (`…org.openehr.rm.data_structures.cluster.adoc` §Attributes), so a
    /// cluster over no items is not a `CLUSTER`, and this page explicitly
    /// permits the list to be empty.
    #[must_use]
    pub fn as_hierarchy(&self) -> Option<Cluster> {
        let items: Vec<Item> = self
            .items
            .as_deref()?
            .iter()
            .map(|element| Item::Element(element.clone()))
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
            value: Some(DataValue::DvText(text(name))),
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
        assert!(empty.names().is_empty());
        assert!(empty.named_item("x").is_none());
        assert!(empty.ith_item(1).is_none());
        assert!(empty.as_hierarchy().is_none());

        empty.items = None;
        assert_eq!(empty.item_count(), 0);
        assert!(empty.names().is_empty());
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
