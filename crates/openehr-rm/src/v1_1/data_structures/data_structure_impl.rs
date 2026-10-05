// @generated-from-template templates/openehr-rm/data_structures/data_structure_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM spec functions for `DATA_STRUCTURE`.
//!
//! `DATA_STRUCTURE` is abstract, so the generated `DataStructure` is the
//! closed subtype enum and the one function it declares is realized here as
//! the dispatch across it. The hierarchies themselves are not re-derived: each
//! `ITEM_STRUCTURE` subtype effects `as_hierarchy` in its own module, which is
//! where the per-shape rules ("a single `CLUSTER` containing the `ELEMENTs` of
//! this list", the table's column transpose, …) live.
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_structures.data_structure.adoc`
//! §Functions + §Description — "Includes the `as_hierarchy` function which can
//! generate the equivalent CEN EN13606 single hierarchy for each subtype's
//! physical representation."

use crate::v1_1::data_structures::data_structure::DataStructure;
use crate::v1_1::data_structures::representation::item::Item;

impl DataStructure {
    /// Returns the CEN EN13606-compatible hierarchy of this structure's
    /// physical representation, or `None` when there is none to build.
    ///
    /// Spec: `org.openehr.rm.data_structures.data_structure.adoc` §Functions
    /// `as_hierarchy` — "Hierarchical equivalent of the physical
    /// representation of each subtype, compatible with CEN EN 13606
    /// structures."
    ///
    /// Two distinct absences answer `None`, and neither is a defect:
    ///
    /// - an EMPTY `ITEM_LIST` / `ITEM_TABLE` / `ITEM_TREE`, whose hierarchy
    ///   would be a `CLUSTER` over no items while `CLUSTER.items` is `1..*`
    ///   (`org.openehr.rm.data_structures.cluster.adoc` §Attributes);
    /// - a `HISTORY`, which is a `DATA_STRUCTURE` that provides no effecting
    ///   definition of this function: its own class page
    ///   (`org.openehr.rm.data_structures.history.adoc` §Functions) declares
    ///   only `is_periodic`, and the released text states no hierarchy for a
    ///   series of events. Answering `None` reports that the specification
    ///   gives no hierarchy here; inventing one would put a shape into
    ///   clinical data that no openEHR text defines.
    #[must_use]
    pub fn as_hierarchy(&self) -> Option<Item> {
        match self {
            Self::ItemList(structure) => structure.as_hierarchy().map(Item::Cluster),
            Self::ItemSingle(structure) => Some(Item::Element(structure.as_hierarchy().clone())),
            Self::ItemTable(structure) => structure.as_hierarchy().map(Item::Cluster),
            Self::ItemTree(structure) => structure.as_hierarchy().map(Item::Cluster),
            Self::History(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_1::data_structures::history::event::Event;
    use crate::v1_1::data_structures::history::history::History;
    use crate::v1_1::data_structures::history::point_event::PointEvent;
    use crate::v1_1::data_structures::item_structure::item_list::ItemList;
    use crate::v1_1::data_structures::item_structure::item_single::ItemSingle;
    use crate::v1_1::data_structures::item_structure::item_structure::ItemStructure;
    use crate::v1_1::data_structures::item_structure::item_tree::ItemTree;
    use crate::v1_1::data_structures::representation::element::Element;
    use crate::v1_1::data_types::basic::data_value::DataValue;
    use crate::v1_1::data_types::quantity::date_time::dv_date_time::DvDateTime;
    use crate::v1_1::data_types::text::dv_text::{DvText, DvTextData};

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

    fn list(items: Option<Vec<Element>>) -> DataStructure {
        DataStructure::ItemList(ItemList {
            name: text("list"),
            archetype_node_id: "at0000".to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            items,
        })
    }

    fn single() -> DataStructure {
        DataStructure::ItemSingle(ItemSingle {
            name: text("single"),
            archetype_node_id: "at0000".to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            item: Box::new(element("at0001", "the element")),
        })
    }

    fn empty_tree() -> DataStructure {
        DataStructure::ItemTree(ItemTree {
            name: text("tree"),
            archetype_node_id: "at0000".to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            items: None,
        })
    }

    fn history() -> DataStructure {
        DataStructure::History(History {
            name: text("history"),
            archetype_node_id: "at0000".to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            origin: DvDateTime {
                normal_status: None,
                normal_range: None,
                other_reference_ranges: None,
                magnitude_status: None,
                accuracy: None,
                value: "2026-01-01T00:00:00Z".to_owned(),
            },
            period: None,
            duration: None,
            summary: None,
            events: Some(vec![Event::PointEvent(PointEvent {
                name: text("event"),
                archetype_node_id: "at0001".to_owned(),
                uid: None,
                links: None,
                archetype_details: None,
                feeder_audit: None,
                time: DvDateTime {
                    normal_status: None,
                    normal_range: None,
                    other_reference_ranges: None,
                    magnitude_status: None,
                    accuracy: None,
                    value: "2026-01-01T00:00:00Z".to_owned(),
                },
                state: None,
                data: ItemStructure::ItemTree(Box::new(ItemTree {
                    name: text("tree"),
                    archetype_node_id: "at0002".to_owned(),
                    uid: None,
                    links: None,
                    archetype_details: None,
                    feeder_audit: None,
                    items: None,
                })),
            })]),
        })
    }

    /// Each `ITEM_STRUCTURE` subtype answers with its own hierarchy, and the
    /// dispatch reports it as the `ITEM` the function's return type names.
    #[test]
    fn each_item_structure_reports_its_own_hierarchy() {
        let list = list(Some(vec![element("at0001", "systolic")]));
        assert!(matches!(list.as_hierarchy(), Some(Item::Cluster(_))));

        // `ITEM_SINGLE.as_hierarchy` is "a single ELEMENT", not a cluster.
        assert!(matches!(single().as_hierarchy(), Some(Item::Element(_))));
    }

    /// An empty structure has no `CLUSTER` to be, so it has no hierarchy.
    #[test]
    fn an_empty_structure_has_no_hierarchy() {
        assert!(list(None).as_hierarchy().is_none());
        assert!(list(Some(Vec::new())).as_hierarchy().is_none());
        assert!(empty_tree().as_hierarchy().is_none());
    }

    /// `HISTORY` is a `DATA_STRUCTURE` for which the released text effects no
    /// hierarchy, so the dispatch reports none rather than inventing a shape.
    #[test]
    fn a_history_has_no_specified_hierarchy() {
        assert!(history().as_hierarchy().is_none());
    }
}
