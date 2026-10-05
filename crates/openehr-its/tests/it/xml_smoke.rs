// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Smoke test for the generated canonical-XML `ToXml` path: load a
//! composition from the JSON corpus, serialize to XML, and check structure.
use openehr_its::xml::to_canonical_xml;
use openehr_rm::prelude::Composition;

#[test]
fn composition_serializes_to_canonical_xml() {
    let json =
        include_str!("../vendor/openehr_sdk/composition/canonical_json/minimal_evaluation.json");
    let compo: Composition =
        openehr_its::json::from_canonical_json(json).expect("deserialize composition JSON");
    let xml = to_canonical_xml(&compo, "composition").expect("serialize to XML");
    println!("{xml}");
    assert!(xml.starts_with("<composition"), "root element");
    assert!(
        xml.contains("xmlns=\"http://schemas.openehr.org/v2\""),
        "v1 namespace"
    );
    assert!(xml.contains("archetype_node_id="), "locatable attribute");
    assert!(xml.contains("</composition>"), "closed root");
}
