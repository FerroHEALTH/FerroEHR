// @generated-from-template templates/openehr-rm/common/archetyped/pathable_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! The `PATHABLE` path functions (`org.openehr.rm.common.pathable.adoc`
//! §Functions; BASE `master11-paths.adoc`), over the canonical-JSON encoding
//! of the value through [`crate::v1_2::paths`].
//!
//! `parent` and `path_of_item` need the enclosing structure, so they are
//! [`crate::v1_2::paths::parent_of`] and
//! [`crate::v1_2::paths::path_of_item`].

#![expect(
    clippy::disallowed_types,
    reason = "the RM path engine navigates the canonical JSON value tree (#1694 boundary class)"
)]

use crate::v1_2::common::archetyped::pathable::Pathable;
use crate::v1_2::paths::{RmPath, TypedPathError, canonical_value};
use serde_json::Value;

impl Pathable {
    /// Returns the item at `a_path`, relative to this node (`item_at_path`); for a
    /// non-unique path, the first in document order.
    ///
    /// # Errors
    /// Returns [`TypedPathError::Path`] when `a_path` is not a well-formed
    /// openEHR path, and [`TypedPathError::Encode`] when this value cannot be
    /// encoded as canonical JSON.
    pub fn item_at_path(&self, a_path: &str) -> Result<Option<Value>, TypedPathError> {
        let path: RmPath = a_path.parse()?;
        let root = canonical_value(self)?;
        Ok(crate::v1_2::paths::item_at_path(&root, &path).cloned())
    }

    /// Returns every item `a_path` resolves to, in document order
    /// (`items_at_path`).
    ///
    /// # Errors
    /// The same two failures as [`Self::item_at_path`].
    pub fn items_at_path(&self, a_path: &str) -> Result<Vec<Value>, TypedPathError> {
        let path: RmPath = a_path.parse()?;
        let root = canonical_value(self)?;
        Ok(crate::v1_2::paths::items_at_path(&root, &path)
            .into_iter()
            .cloned()
            .collect())
    }

    /// Whether `a_path` resolves to at least one item (`path_exists`); a
    /// malformed path is `false`.
    #[must_use]
    pub fn path_exists(&self, a_path: &str) -> bool {
        self.items_at_path(a_path)
            .is_ok_and(|items| !items.is_empty())
    }

    /// Whether `a_path` resolves to exactly one item (`path_unique`).
    #[must_use]
    pub fn path_unique(&self, a_path: &str) -> bool {
        self.items_at_path(a_path)
            .is_ok_and(|items| items.len() == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::data_structures::item_structure::item_tree::ItemTree;
    use crate::v1_2::data_structures::representation::element::Element;
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

    /// A tree of two elements, one node id repeated under two names — the
    /// shape that separates a unique path from a merely existing one.
    fn tree() -> Pathable {
        Pathable::ItemTree(Box::new(ItemTree {
            name: text("tree"),
            archetype_node_id: "openEHR-EHR-ITEM_TREE.tree.v1".to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            items: Some(vec![
                Item::from(element("at0001", "systolic")),
                Item::from(element("at0001", "diastolic")),
            ]),
        }))
    }

    /// A path that resolves exists; one that does not, does not — and a
    /// malformed expression is not a path this data answers.
    #[test]
    fn a_path_exists_when_it_resolves() {
        let tree = tree();
        assert!(tree.path_exists("/items[at0001]"));
        assert!(!tree.path_exists("/items[at9999]"));
        assert!(!tree.path_exists(""));
    }

    /// "True if the path corresponds to a single item": the repeated node id
    /// is not unique, the name predicate that separates the two is.
    #[test]
    fn uniqueness_is_about_how_many_items_answer() {
        let tree = tree();
        assert!(!tree.path_unique("/items[at0001]"));
        assert!(tree.path_unique("/items[at0001,'systolic']"));
        assert!(!tree.path_unique("/items[at9999]"));
    }

    /// `items_at_path` reports every match in document order; `item_at_path`
    /// reports the first of them.
    #[test]
    fn the_items_are_reported_in_document_order() {
        let tree = tree();
        let items = tree
            .items_at_path("/items[at0001]")
            .expect("a well-formed path");
        assert_eq!(items.len(), 2);

        let first = tree
            .item_at_path("/items[at0001]")
            .expect("a well-formed path")
            .expect("a resolving path");
        assert_eq!(items.first(), Some(&first));
        assert_eq!(
            first.pointer("/name/value").and_then(Value::as_str),
            Some("systolic")
        );
    }

    /// A malformed path expression is a typed refusal on the fallible
    /// entry points, not an empty answer.
    #[test]
    fn a_malformed_path_is_refused() {
        assert!(matches!(
            tree().item_at_path("//"),
            Err(TypedPathError::Path(_))
        ));
    }
}
