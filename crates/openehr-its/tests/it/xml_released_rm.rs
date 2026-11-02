// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Canonical-XML round trips of the released RM 1.1.0 generation
//! (`openehr_rm::v1_1`), which the codec covers beside the development one.

use openehr_its::xml::runtime::{Namespace, from_xml, to_xml};
use openehr_rm::v1_1::common::resource::translation_details::TranslationDetails;
use openehr_rm::v1_1::composition::view::view_section::ViewSection;
use openehr_rm::v1_1::demographic::person::Person;

/// A `PARTY_REF` to the PERSON `id`.
fn party_ref(id: &str) -> serde_json::Value {
    serde_json::json!({
        "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
        "id": { "_type": "HIER_OBJECT_ID", "value": id }
    })
}

/// RM 1.1.0 `PARTY.reverse_relationships` (`demographic.party.adoc` 1.1.0
/// §Attributes) is the element ITS-XML RM Release-1.1.0 `Demographic.xsd`
/// declares right after `relationships`, and it survives the round trip.
#[test]
fn a_released_person_carries_reverse_relationships() {
    let json = serde_json::json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "name": { "_type": "DV_TEXT", "value": "Target" },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "legal name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "structure" }
            }
        }],
        "relationships": [{
            "_type": "PARTY_RELATIONSHIP",
            "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1",
            "name": { "_type": "DV_TEXT", "value": "parent-of" },
            "source": party_ref("0e1b2c3d-0000-4000-8000-000000000001"),
            "target": party_ref("0e1b2c3d-0000-4000-8000-000000000002")
        }],
        "reverse_relationships": [{
            "_type": "LOCATABLE_REF",
            "namespace": "local",
            "type": "PARTY_RELATIONSHIP",
            "id": { "_type": "HIER_OBJECT_ID", "value": "0e1b2c3d-0000-4000-8000-000000000003" },
            "path": "/relationships[openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1]"
        }]
    });
    let person: Person =
        openehr_its::json::from_canonical_value(&json).expect("an RM 1.1.0 PERSON");

    let xml = to_xml(&person, "party", Namespace::V2).expect("serialize");
    let relationships = xml.find("<relationships").expect("relationships element");
    let reverse = xml
        .find("<reverse_relationships")
        .expect("reverse_relationships element");
    assert!(
        relationships < reverse,
        "Demographic.xsd places reverse_relationships after relationships: {xml}"
    );
    let parsed: Person = from_xml(&xml).expect("parse back");
    assert_eq!(
        parsed, person,
        "reverse_relationships must survive the round trip"
    );
}

/// The RM 1.1.0 BMM spells `TRANSLATION_DETAILS.accreditaton`; the codec writes
/// and reads it as the `accreditation` element ITS-XML 1.0.2 `Resource.xsd`
/// declares.
#[test]
fn a_released_translation_writes_the_xsd_accreditation_element() {
    let json = r#"{
        "_type": "TRANSLATION_DETAILS",
        "language": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" },
            "code_string": "nl"
        },
        "author": { "name": "Translator 1" },
        "accreditaton": "NL-TR-0042"
    }"#;
    let details: TranslationDetails =
        openehr_its::json::from_canonical_json(json).expect("an RM 1.1.0 TRANSLATION_DETAILS");

    let xml = to_xml(&details, "translations", Namespace::V2).expect("serialize");
    assert!(
        xml.contains("<accreditation>NL-TR-0042</accreditation>"),
        "{xml}"
    );
    let parsed: TranslationDetails = from_xml(&xml).expect("parse back");
    assert_eq!(parsed, details);
}

/// `VIEW_SECTION` and `CITATION` (RM 1.1.0 BMM `composition.content.view`),
/// which no ITS-XML schema declares, take `CONTENT_ITEM`'s shape: the
/// `archetype_node_id` attribute `LOCATABLE` requires, then their own fields.
#[test]
fn a_view_section_with_a_citation_round_trips() {
    let json = serde_json::json!({
        "_type": "VIEW_SECTION",
        "archetype_node_id": "at0001",
        "name": { "_type": "DV_TEXT", "value": "view" },
        "items": [{
            "_type": "CITATION",
            "archetype_node_id": "at0002",
            "name": { "_type": "DV_TEXT", "value": "citation" },
            "source_type": "SECTION",
            "source_ref": {
                "_type": "LOCATABLE_REF", "namespace": "local", "type": "SECTION",
                "id": { "_type": "HIER_OBJECT_ID", "value": "0e1b2c3d-0000-4000-8000-000000000004" },
                "path": "/content[at0003]"
            },
            "source_parent_ref": {
                "_type": "LOCATABLE_REF", "namespace": "local", "type": "COMPOSITION",
                "id": { "_type": "HIER_OBJECT_ID", "value": "0e1b2c3d-0000-4000-8000-000000000004" },
                "path": "/"
            },
            "resolved": {
                "_type": "SECTION",
                "archetype_node_id": "at0003",
                "name": { "_type": "DV_TEXT", "value": "cited" }
            }
        }]
    });
    let section: ViewSection =
        openehr_its::json::from_canonical_value(&json).expect("an RM 1.1.0 VIEW_SECTION");

    let xml = to_xml(&section, "content", Namespace::V2).expect("serialize");
    assert!(
        xml.contains(r#"archetype_node_id="at0001""#),
        "VIEW_SECTION.archetype_node_id must be an attribute: {xml}"
    );
    assert!(
        xml.contains(r#"archetype_node_id="at0002""#),
        "CITATION.archetype_node_id must be an attribute: {xml}"
    );
    assert!(
        !xml.contains("<archetype_node_id>"),
        "no archetype_node_id child element: {xml}"
    );
    let parsed: ViewSection = from_xml(&xml).expect("parse back");
    assert_eq!(
        parsed, section,
        "the view section must survive the round trip"
    );
}
