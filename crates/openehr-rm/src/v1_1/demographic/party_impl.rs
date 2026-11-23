// @generated-from-template templates/openehr-rm/overrides/v1_1/demographic/party_impl.rs (per-generation override) — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! `PARTY.type` (`org.openehr.rm.demographic.party.adoc`), dispatched over the
//! five party types. `reverse_relationships` reads the demographic repository
//! (`Post_reverse_relationships_validity`), so the service realizes it.
//!
//! NOTE: the `v1_1` override; RM 1.1.0 declares `PARTY.reverse_relationships`
//! (`party.adoc` §Attributes), so the fixtures differ.

use crate::v1_1::data_types::text::dv_text::DvText;
use crate::v1_1::demographic::party::Party;

impl Party {
    /// The type of this party: its `name` (`Type_valid: type = name`).
    #[must_use]
    pub fn r#type(&self) -> &DvText {
        match self {
            Self::Agent(party) => &party.name,
            Self::Group(party) => &party.name,
            Self::Organisation(party) => &party.name,
            Self::Person(party) => &party.name,
            Self::Role(party) => &party.name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_1::data_structures::item_structure::item_structure::ItemStructure;
    use crate::v1_1::data_structures::item_structure::item_tree::ItemTree;
    use crate::v1_1::data_types::text::dv_text::DvTextData;
    use crate::v1_1::demographic::organisation::Organisation;
    use crate::v1_1::demographic::party_identity::PartyIdentity;
    use crate::v1_1::demographic::person::Person;

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

    fn identities() -> openehr_base::containers::NonEmptyVec<PartyIdentity> {
        openehr_base::containers::NonEmptyVec::of(PartyIdentity {
            name: text("legal"),
            archetype_node_id: "openEHR-DEMOGRAPHIC-PARTY_IDENTITY.person_name.v1".to_owned(),
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
        })
    }

    fn person(name: &str) -> Party {
        Party::Person(Box::new(Person {
            name: text(name),
            archetype_node_id: "openEHR-DEMOGRAPHIC-PERSON.person.v1".to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            identities: identities(),
            contacts: None,
            details: None,
            relationships: None,
            reverse_relationships: None,
            languages: None,
            roles: None,
        }))
    }

    fn organisation(name: &str) -> Party {
        Party::Organisation(Box::new(Organisation {
            name: text(name),
            archetype_node_id: "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v1".to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            identities: identities(),
            contacts: None,
            details: None,
            relationships: None,
            reverse_relationships: None,
            languages: None,
            roles: None,
        }))
    }

    /// `Type_valid: type = name` — the dispatch returns each subtype's own
    /// runtime name, never a value derived from the Rust variant.
    #[test]
    fn the_type_is_the_name_of_whichever_party_this_is() {
        let a_person = person("PERSON");
        assert_eq!(a_person.r#type(), &text("PERSON"));

        let an_organisation = organisation("ORGANISATION");
        assert_eq!(an_organisation.r#type(), &text("ORGANISATION"));

        // The name carries the ROLE name where the party is a role, so the
        // function follows the data, not the variant.
        let general_practitioner = person("general practitioner");
        assert_eq!(general_practitioner.r#type(), &text("general practitioner"));
    }
}
