// @generated-from-template templates/openehr-rm/data_structures/representation/cluster_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariant for `CLUSTER`.
//!
//! Only the inherited LOCATABLE `Archetype_node_id_valid`:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_structures.cluster.adoc`
//! declares no §Invariants section of its own.
//!
//! That page's §Attributes states `items` as a mandatory `List<ITEM>` (BMM
//! cardinality `1..*`), which the generated field carries as
//! `NonEmptyVec<Item>` — an empty `items` is unrepresentable
//! (`openehr_base::containers`) rather than merely unchecked.

use crate::v1_2::data_structures::representation::cluster::Cluster;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for Cluster {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_2::validate::generated::archetype_node_id_core(
            "CLUSTER",
            &self.archetype_node_id,
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::data_structures::representation::item::Item;
    use crate::v1_2::data_types::text::dv_text::{DvText, DvTextData};

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

    /// A minimal ELEMENT, the simplest legal `CLUSTER.items` member.
    fn element() -> Item {
        Item::Element(
            crate::v1_2::data_structures::representation::element::Element {
                name: text("element"),
                archetype_node_id: "at0002".to_owned(),
                uid: None,
                links: None,
                archetype_details: None,
                feeder_audit: None,
                null_flavour: None,
                value: None,
                null_reason: None,
            },
        )
    }

    fn cluster(node_id: &str) -> Cluster {
        Cluster {
            name: text("cluster"),
            archetype_node_id: node_id.to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            items: openehr_base::containers::NonEmptyVec::of(element()),
        }
    }

    /// `CLUSTER.items` is `1..*`
    /// (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_structures.cluster.adoc`
    /// §Attributes), and the emission shape carries that bound, so an empty item
    /// list is refused at CONSTRUCTION rather than reported by the invariant
    /// layer.
    #[test]
    fn an_empty_item_list_is_unrepresentable() {
        assert!(openehr_base::containers::NonEmptyVec::<Item>::new(Vec::new()).is_err());
    }

    #[test]
    fn empty_node_id_invalid() {
        assert_eq!(
            cluster("").invariants()[0].message,
            "Invariant Archetype_node_id_valid failed on type CLUSTER"
        );
    }
}
