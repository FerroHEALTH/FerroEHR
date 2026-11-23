// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The `ITEM_TREE` functions (`org.openehr.rm.data_structures.item_tree.adoc`)
//! through [`crate::v1_2::paths`]; an absent list reads as empty.

use crate::v1_2::data_structures::item_structure::item_tree::ItemTree;
use crate::v1_2::data_structures::representation::cluster::Cluster;
use crate::v1_2::data_structures::representation::element::Element;
use crate::v1_2::paths::{RmPath, TypedPathError, canonical_value};

impl ItemTree {
    /// Returns the leaf element at `a_path` (`element_at_path`), or `None` when
    /// the path reaches no `ELEMENT`.
    ///
    /// # Errors
    /// Returns [`TypedPathError::Path`] when `a_path` is not a well-formed
    /// openEHR path, and [`TypedPathError::Encode`] when this tree cannot be
    /// encoded as canonical JSON.
    pub fn element_at_path(&self, a_path: &str) -> Result<Option<Element>, TypedPathError> {
        let path: RmPath = a_path.parse()?;
        let root = canonical_value(self)?;
        // NOTE: a node that is not an `ELEMENT` is a non-leaf path, not a defect.
        Ok(crate::v1_2::paths::item_at_path(&root, &path)
            .and_then(|node| serde_json::from_value::<Element>(node.clone()).ok()))
    }

    /// Whether `a_path` is a valid leaf path of this tree (`has_element_path`); a
    /// malformed path is `false`.
    #[must_use]
    pub fn has_element_path(&self, a_path: &str) -> bool {
        matches!(self.element_at_path(a_path), Ok(Some(_)))
    }

    /// One `CLUSTER` carrying this tree's items under its name and node id
    /// (`as_hierarchy`), or `None` for an empty tree, since `CLUSTER.items` is
    /// `1..*`.
    #[must_use]
    pub fn as_hierarchy(&self) -> Option<Cluster> {
        let items = self.items.clone()?;
        Some(Cluster {
            name: self.name.clone(),
            archetype_node_id: self.archetype_node_id.clone(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            // The only failure is an empty `items`, which is this function's
            // absent case rather than a defect — see the doc comment.
            items: openehr_base::containers::NonEmptyVec::new(items).ok()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::data_structures::representation::item::Item;
    use crate::v1_2::data_types::basic::data_value::DataValue;
    use crate::v1_2::data_types::text::dv_text::{DvText, DvTextData};

    fn text(value: &str) -> DvText {
        DvText::DvText(DvTextData {
            value: value.to_owned(),
            hyperlink: None,
            formatting: None,
            mappings: None,
            language: None,
            encoding: None,
        })
    }

    fn element(node_id: &str, name: &str) -> Element {
        Element {
            name: text(name),
            archetype_node_id: node_id.to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            null_flavour: None,
            value: Some(Box::new(DataValue::DvText(text(name)))),
            null_reason: None,
        }
    }

    fn cluster(node_id: &str, name: &str, items: Vec<Item>) -> Option<Cluster> {
        Some(Cluster {
            name: text(name),
            archetype_node_id: node_id.to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            items: openehr_base::containers::NonEmptyVec::new(items).ok()?,
        })
    }

    /// A tree carrying one leaf and one interior cluster with a leaf of its
    /// own — enough to separate a leaf path from a non-leaf one.
    fn tree(items: Option<Vec<Item>>) -> ItemTree {
        ItemTree {
            name: text("tree"),
            archetype_node_id: "openEHR-EHR-ITEM_TREE.tree.v1".to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            items,
        }
    }

    fn populated() -> Option<ItemTree> {
        Some(tree(Some(vec![
            Item::from(element("at0001", "systolic")),
            Item::from(cluster(
                "at0002",
                "group",
                vec![Item::from(element("at0003", "diastolic"))],
            )?),
        ])))
    }

    /// An absent `items` and an empty one are the same tree.
    #[test]
    fn an_absent_tree_and_an_empty_tree_agree() {
        for items in [None, Some(Vec::new())] {
            let tree = tree(items);
            assert!(tree.as_hierarchy().is_none());
            assert!(!tree.has_element_path("/items[at0001]"));
            assert!(matches!(tree.element_at_path("/items[at0001]"), Ok(None)));
        }
    }

    /// "Return the leaf element at the path": a path onto an `ELEMENT` yields
    /// that element, at the top level and nested alike.
    #[test]
    fn a_leaf_path_yields_its_element() {
        let tree = populated().expect("a non-empty cluster");
        let leaf = tree
            .element_at_path("/items[at0001]")
            .expect("a well-formed path")
            .expect("a leaf at that path");
        assert_eq!(leaf.archetype_node_id, "at0001");

        let nested = tree
            .element_at_path("/items[at0002]/items[at0003]")
            .expect("a well-formed path")
            .expect("a leaf at that path");
        assert_eq!(nested.archetype_node_id, "at0003");
    }

    /// An interior `CLUSTER`, an unresolved path and a malformed one are not leaf
    /// paths.
    #[test]
    fn only_a_path_onto_an_element_is_a_leaf_path() {
        let tree = populated().expect("a non-empty cluster");
        assert!(tree.has_element_path("/items[at0001]"));
        assert!(tree.has_element_path("/items[at0002]/items[at0003]"));
        assert!(!tree.has_element_path("/items[at0002]"));
        assert!(!tree.has_element_path("/items[at9999]"));
        assert!(!tree.has_element_path(""));
    }

    /// The hierarchy is one `CLUSTER` over the tree's own items, under the
    /// tree's name and node id — the physical representation, unchanged.
    #[test]
    fn the_hierarchy_is_the_trees_own_items_under_one_cluster() {
        let tree = populated().expect("a non-empty cluster");
        let hierarchy = tree.as_hierarchy().expect("a non-empty tree");
        assert_eq!(hierarchy.name, tree.name);
        assert_eq!(hierarchy.archetype_node_id, tree.archetype_node_id);
        assert_eq!(hierarchy.items.len(), 2);
        assert_eq!(
            hierarchy.items.first(),
            tree.items.as_ref().and_then(|items| items.first())
        );
    }
}
