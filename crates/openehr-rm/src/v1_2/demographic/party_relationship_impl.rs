// @generated-from-template templates/openehr-rm/demographic/party_relationship_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM spec functions for `PARTY_RELATIONSHIP`.
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.demographic.party_relationship.adoc`
//! §Functions + §Invariants.

use crate::v1_2::data_types::text::dv_text::DvText;
use crate::v1_2::demographic::party_relationship::PartyRelationship;

impl PartyRelationship {
    /// Returns the type of this relationship, e.g. employment or authority.
    ///
    /// Spec: `org.openehr.rm.demographic.party_relationship.adoc` §Functions
    /// `type` — "Type of relationship, such as employment, authority, health
    /// provision" — pinned to the inherited runtime name by §Invariants
    /// `Type_validity: type = name`, exactly as its `ADDRESS` / `CONTACT` /
    /// `PARTY_IDENTITY` siblings state in their own §Functions prose.
    #[must_use]
    pub fn r#type(&self) -> &DvText {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::data_types::text::dv_text::DvTextData;
    use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
    use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
    use openehr_base::v1_3::base_types::identification::party_ref::PartyRef;

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

    fn party_ref(uuid: &str) -> Option<PartyRef> {
        Some(PartyRef {
            namespace: "demographic".to_owned(),
            r#type: "PERSON".to_owned(),
            id: ObjectId::HierObjectId(HierObjectId::new(uuid.to_owned()).ok()?),
        })
    }

    fn relationship(name: &str) -> Option<PartyRelationship> {
        Some(PartyRelationship {
            name: text(name),
            archetype_node_id: "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.employment.v1".to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            details: None,
            target: party_ref("11111111-1111-4111-8111-111111111111")?,
            time_validity: None,
            source: party_ref("22222222-2222-4222-8222-222222222222")?,
        })
    }

    /// `Type_validity: type = name` — the function returns the name itself.
    #[test]
    fn the_type_is_the_name() {
        for name in ["employment", "health provision", ""] {
            let relationship = relationship(name).expect("well-formed party identifiers");
            assert_eq!(relationship.r#type(), &relationship.name);
        }
    }
}
