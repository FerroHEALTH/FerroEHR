// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Regression test (canonical-XML audit): `LOCATABLE` subtypes that
//! live *outside* the v1 `ALL/` XSD bundle — `EHR_STATUS` / `EHR_ACCESS` (EHR
//! package), the demographic `PARTY` hierarchy, and the extract `LOCATABLE`
//! subtypes — must serialize `archetype_node_id` as the required XML
//! **attribute** on the element, never as a child element.
//!
//! `LOCATABLE` (v1 `Structure.xsd`, identical in v2 `Common.xsd`) declares
//! `<xs:attribute name="archetype_node_id" ... use="required"/>`; `Ehr.xsd` /
//! `Demographic.xsd` / `EhrExtract.xsd` (v2, RM 1.1.0) define these types as
//! `extension base="LOCATABLE"`. Before the fix the emitter's XSD closure lacked
//! these types, so they fell back to attribute-less BMM field order and emitted
//! `<archetype_node_id>…</archetype_node_id>` — invalid canonical XML.

use openehr_its::xml::runtime::from_xml;
use openehr_its::xml::to_canonical_xml;
use openehr_rm::prelude::{EhrAccess, EhrStatus, GenericContentItem, Person};

/// Assert a serialized LOCATABLE subtype carries `archetype_node_id` as an XML
/// attribute (`archetype_node_id="…"`) and *not* as a child element
/// (`<archetype_node_id>`), then that it round-trips back to the same value.
fn assert_archetype_node_id_is_attribute(xml: &str, expected: &str) {
    assert!(
        xml.contains(&format!("archetype_node_id=\"{expected}\"")),
        "archetype_node_id must be an XML attribute: {xml}"
    );
    assert!(
        !xml.contains("<archetype_node_id>"),
        "archetype_node_id must NOT be a child element: {xml}"
    );
}

#[test]
fn ehr_status_archetype_node_id_is_attribute() {
    let json = r#"{
        "_type": "EHR_STATUS",
        "name": {"value": "EHR Status"},
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "subject": {"_type": "PARTY_SELF"},
        "is_queryable": true,
        "is_modifiable": true
    }"#;
    let status: EhrStatus =
        openehr_its::json::from_canonical_json(json).expect("deserialize EHR_STATUS JSON");
    let xml = to_canonical_xml(&status, "ehr_status").expect("serialize EHR_STATUS");
    assert_archetype_node_id_is_attribute(&xml, "openEHR-EHR-EHR_STATUS.generic.v1");
    // FromXml must read the attribute back (same XSD-driven attr classification).
    let back: EhrStatus = from_xml(&xml).expect("parse EHR_STATUS");
    assert_eq!(back.archetype_node_id, "openEHR-EHR-EHR_STATUS.generic.v1");
    assert_eq!(
        to_canonical_xml(&back, "ehr_status").expect("re-serialize"),
        xml,
        "round-trip stable"
    );
}

#[test]
fn ehr_access_archetype_node_id_is_attribute() {
    let json = r#"{
        "_type": "EHR_ACCESS",
        "name": {"value": "EHR Access"},
        "archetype_node_id": "at0000"
    }"#;
    let access: EhrAccess =
        openehr_its::json::from_canonical_json(json).expect("deserialize EHR_ACCESS JSON");
    let xml = to_canonical_xml(&access, "ehr_access").expect("serialize EHR_ACCESS");
    assert_archetype_node_id_is_attribute(&xml, "at0000");
    let back: EhrAccess = from_xml(&xml).expect("parse EHR_ACCESS");
    assert_eq!(back.archetype_node_id, "at0000");
}

#[test]
fn demographic_person_archetype_node_id_is_attribute() {
    // PERSON → ACTOR → PARTY → LOCATABLE (v2 Demographic.xsd); previously
    // uncovered by the v1 bundle (no demographic schema).
    // `PARTY.identities` is `1..*`
    // (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.demographic.party.adoc`
    // §Attributes), so a PERSON states at least one identity.
    let json = r#"{
        "_type": "PERSON",
        "name": {"value": "Patient"},
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "identities": [
            {
                "_type": "PARTY_IDENTITY",
                "name": {"value": "legal identity"},
                "archetype_node_id": "at0001",
                "details": {
                    "_type": "ITEM_TREE",
                    "name": {"value": "identity"},
                    "archetype_node_id": "at0002"
                }
            }
        ]
    }"#;
    let person: Person =
        openehr_its::json::from_canonical_json(json).expect("deserialize PERSON JSON");
    let xml = to_canonical_xml(&person, "person").expect("serialize PERSON");
    assert_archetype_node_id_is_attribute(&xml, "openEHR-DEMOGRAPHIC-PERSON.person.v1");
    let back: Person = from_xml(&xml).expect("parse PERSON");
    assert_eq!(
        back.archetype_node_id,
        "openEHR-DEMOGRAPHIC-PERSON.person.v1"
    );
}

#[test]
fn extract_generic_content_item_archetype_node_id_is_attribute() {
    // GENERIC_CONTENT_ITEM → EXTRACT_CONTENT_ITEM → EXTRACT_ITEM → LOCATABLE
    // (v2 EhrExtract.xsd). The v1 `Extract.xsd` carried a stale EXTRACT_ITEM with
    // no LOCATABLE base; the emitter now draws the extract family from v2.
    let json = r#"{
        "_type": "GENERIC_CONTENT_ITEM",
        "name": {"value": "item"},
        "archetype_node_id": "at0001",
        "is_primary": true
    }"#;
    let item: GenericContentItem = openehr_its::json::from_canonical_json(json)
        .expect("deserialize GENERIC_CONTENT_ITEM JSON");
    let xml = to_canonical_xml(&item, "content_item").expect("serialize GENERIC_CONTENT_ITEM");
    assert_archetype_node_id_is_attribute(&xml, "at0001");
    let back: GenericContentItem = from_xml(&xml).expect("parse GENERIC_CONTENT_ITEM");
    assert_eq!(back.archetype_node_id, "at0001");
}
