// @generated-from-template templates/openehr-rm/data_structures/item_structure/item_tree_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM spec functions for `ITEM_TREE`.
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_structures.item_tree.adoc`
//! §Functions. `items` is `0..1` and §Description says "The tree may be
//! empty", so every accessor treats an absent list and an empty one alike.
//!
//! The two path functions run over the canonical-JSON value tree through
//! [`crate::v1_2::paths`], the crate's one path engine — a leaf path means the
//! same thing here as it does on the wire.

use crate::v1_2::data_structures::item_structure::item_tree::ItemTree;
use crate::v1_2::data_structures::representation::cluster::Cluster;
use crate::v1_2::data_structures::representation::element::Element;
use crate::v1_2::paths::{RmPath, TypedPathError, canonical_value};

impl ItemTree {
    /// Returns the leaf element at `a_path`, or `None` when the path resolves
    /// to nothing or to something that is not an `ELEMENT`.
    ///
    /// Spec: `org.openehr.rm.data_structures.item_tree.adoc` §Functions
    /// `element_at_path` — "Return the leaf element at the path `a_path`".
    ///
    /// # Errors
    /// Returns [`TypedPathError::Path`] when `a_path` is not a well-formed
    /// openEHR path, and [`TypedPathError::Encode`] when this tree cannot be
    /// encoded as canonical JSON.
    pub fn element_at_path(&self, a_path: &str) -> Result<Option<Element>, TypedPathError> {
        let path: RmPath = a_path.parse()?;
        let root = canonical_value(self)?;
        // NOTE: a node that does not decode as an `ELEMENT` is a legitimately
        // absent leaf, not a defect — an interior `CLUSTER` is a perfectly
        // valid node at a path that is simply not a leaf path.
        Ok(crate::v1_2::paths::item_at_path(&root, &path)
            .and_then(|node| serde_json::from_value::<Element>(node.clone()).ok()))
    }

    /// Returns `true` when `a_path` is a valid leaf path of this tree.
    ///
    /// Spec: `org.openehr.rm.data_structures.item_tree.adoc` §Functions
    /// `has_element_path` — "True if path `a_path` is a valid leaf path". The
    /// function's return type is `Boolean`, so a malformed path expression is
    /// `false`: it is not a leaf path of this tree, or of any other.
    #[must_use]
    pub fn has_element_path(&self, a_path: &str) -> bool {
        matches!(self.element_at_path(a_path), Ok(Some(_)))
    }

    /// Returns a CEN EN13606-compatible hierarchy: one `CLUSTER` carrying this
    /// tree's items.
    ///
    /// Spec: `org.openehr.rm.data_structures.item_tree.adoc` §Functions
    /// `as_hierarchy` — "Generate a CEN EN13606-compatible hierarchy, which is
    /// the same as the tree's physical representation", i.e. the items are
    /// carried across unchanged, under the tree's own name and node id.
    ///
    /// `None` for an empty tree: `CLUSTER.items` is `1..*`
    /// (`org.openehr.rm.data_structures.cluster.adoc` §Attributes), so a
    /// cluster over no items is not a `CLUSTER`, and this page explicitly
    /// permits the tree to be empty.
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
            value: Some(DataValue::DvText(text(name))),
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
            Item::Element(element("at0001", "systolic")),
            Item::Cluster(cluster(
                "at0002",
                "group",
                vec![Item::Element(element("at0003", "diastolic"))],
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

    /// "True if path is a valid leaf path" — an interior `CLUSTER` sits at a
    /// perfectly good path that is not a LEAF path, and neither is a path that
    /// resolves to nothing or a malformed expression.
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
