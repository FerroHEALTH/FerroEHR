// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written RM spec functions for `PATHABLE`.
//!
//! `PATHABLE` is abstract, so the generated `Pathable` is the closed subtype
//! enum over every pathable RM class, and the four resolvable functions it
//! declares are realized here. The pathing itself is NOT re-derived: it lives
//! once in [`crate::v1_2::paths`], over the canonical-JSON value tree, and a
//! typed value reaches it by encoding itself with this crate's own
//! canonical-JSON impls — so a typed path answer and a wire path answer cannot
//! differ.
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.pathable.adoc`
//! §Functions; the path grammar is BASE
//! `docs/specs/openehr/BASE/docs/architecture_overview/master11-paths.adoc`.
//!
//! Two of the six functions are NOT realized here, and the reason is the same
//! in both cases — a value alone is not the structure the function is defined
//! over:
//!
//! - `parent ()` is "the parent of this node in a compositional hierarchy",
//!   which §Description says "is defined as abstract in the model, and may be
//!   implemented in any way convenient". No RM attribute carries it, and this
//!   repository stores no owning back-references, so the parent is a
//!   root-anchored lookup ([`crate::v1_2::paths::parent_of`]) — it needs the
//!   enclosing structure, which a node does not hold.
//! - `path_of_item (a_loc)` is "the path to an item relative to the root of
//!   this archetyped structure", i.e. the path of a node that IS inside this
//!   structure. The primitive answering it
//!   ([`crate::v1_2::paths::path_of_item`]) locates that node by identity;
//!   two independently held typed values have no identity relation, and
//!   matching by equality instead would return the path of *an* equal node
//!   rather than the one asked about, which is a different question wherever a
//!   structure repeats a value.

#![expect(
    clippy::disallowed_types,
    reason = "the RM path engine navigates the canonical JSON value tree (#1694 boundary class)"
)]

use crate::v1_2::common::archetyped::pathable::Pathable;
use crate::v1_2::paths::{RmPath, TypedPathError, canonical_value};
use serde_json::Value;

impl Pathable {
    /// Returns the item at `a_path`, relative to this node.
    ///
    /// Spec: `org.openehr.rm.common.pathable.adoc` §Functions `item_at_path` —
    /// "The item at a path (relative to this item); only valid for unique
    /// paths, i.e. paths that resolve to a single item"
    /// (`Pre: path_unique (a_path)`). Where that precondition does not hold,
    /// the first item in document order is returned; ask
    /// [`Self::items_at_path`] to see them all.
    ///
    /// The item is a canonical-JSON value because the function's own return
    /// type is `Any`: a path may land on an RM object, on a primitive leaf, or
    /// on a container member, and no single generated type spans those.
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

    /// Returns every item `a_path` resolves to, in document order.
    ///
    /// Spec: `org.openehr.rm.common.pathable.adoc` §Functions `items_at_path`
    /// — "List of items corresponding to a non-unique path". A path that
    /// resolves to nothing yields an empty list.
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

    /// Returns `true` when `a_path` resolves to at least one item under this
    /// node.
    ///
    /// Spec: `org.openehr.rm.common.pathable.adoc` §Functions `path_exists` —
    /// "True if the path exists in the data with respect to the current item"
    /// (`Pre: not a_path.is_empty`). The function's return type is `Boolean`,
    /// so a path this data cannot answer is `false`: an empty or malformed
    /// path expression exists in no data, and neither does one that resolves
    /// to nothing.
    #[must_use]
    pub fn path_exists(&self, a_path: &str) -> bool {
        self.items_at_path(a_path)
            .is_ok_and(|items| !items.is_empty())
    }

    /// Returns `true` when `a_path` resolves to exactly one item under this
    /// node.
    ///
    /// Spec: `org.openehr.rm.common.pathable.adoc` §Functions `path_unique` —
    /// "True if the path corresponds to a single item in the data"
    /// (`Pre: path_exists (a_path)`), which is why a path resolving to nothing
    /// is not unique either.
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
            value: Some(DataValue::DvText(text(name))),
            null_reason: None,
        }
    }

    /// A tree of two elements, one node id repeated under two names — the
    /// shape that separates a unique path from a merely existing one.
    fn tree() -> Pathable {
        Pathable::ItemTree(ItemTree {
            name: text("tree"),
            archetype_node_id: "openEHR-EHR-ITEM_TREE.tree.v1".to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            items: Some(vec![
                Item::Element(element("at0001", "systolic")),
                Item::Element(element("at0001", "diastolic")),
            ]),
        })
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
