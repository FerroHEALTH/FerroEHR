// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! AOM2 **model-form** archetype XML codec gate — the `Archetype.xsd`
//! counterpart of the persistent-form gate in `aom2_xml`.
//!
//! # Corpus ceiling (stated, not implied)
//!
//! There is NO upstream instance corpus for this serialization. All 8 documents
//! openEHR ships in the vendored ITS-XML bundle
//! (`schemas/xml/its-xml-1.0.2-nsv1/AOM2/examples/`) declare
//! `xsi:schemaLocation="… ../P_Archetype.xsd"` and are therefore PERSISTENT-form
//! (`P_AUTHORED_ARCHETYPE`), covered by `aom2_xml`; the upstream
//! `openEHR/adl-archetypes` library publishes ADL text only (`.adl`/`.adls`), and
//! `openEHR/specifications-ITS-XML` has just three branches — `Release-1.0.2`,
//! `Release-2.0.0` and `master`, the last two identical and both already pinned
//! here. So no further upstream corpus exists to vendor.
//!
//! The gate is therefore **self-consistency**, which is what is provable without
//! a corpus: construct a minimal conforming `AUTHORED_ARCHETYPE` in Rust,
//! serialize it, parse the result back, and require the value to be equal — the
//! codec must be symmetric, not merely lenient on input. The wire envelope (root
//! element name + the ITS-XML v1 namespace) is asserted on the serialized text.
//!
//! # Why the root is `AUTHORED_ARCHETYPE`
//!
//! `Archetype.xsd` declares the entry point as
//! `<xs:element name="archetype" type="ARCHETYPE"/>`, but `ARCHETYPE` is
//! `abstract="true"` and nothing in the closure derives from it —
//! `AUTHORED_ARCHETYPE` extends `AUTHORED_RESOURCE` (`Resource.xsd`) and re-uses
//! the archetype body through `<xs:group ref="ARCHETYPE"/>` rather than extending
//! the type. `AUTHORED_ARCHETYPE` is thus the only instantiable archetype root
//! the schema offers, and the generated entry points are typed to it.

#![expect(
    clippy::panic_in_result_fn,
    reason = "the Book ch11 test shape: `?` propagates the codec plumbing while the assertions ARE the test — an assertion panic is how these tests fail"
)]

use openehr_its::aom2_model::types::{
    ArchetypeHrid, ArchetypeTerm, ArchetypeTerminology, AuthoredArchetype, CAttribute,
    CComplexObject, CObject, CString, Cardinality, Codedefinitionset, Multiplicityinterval,
    TerminologyCode,
};
use openehr_its::xml::runtime::XmlError;

/// A `MultiplicityInterval` (`BaseTypes.xsd`) over `lower..upper`, both bounds
/// included and bounded.
fn interval(lower: i32, upper: i32) -> Multiplicityinterval {
    Multiplicityinterval {
        lower_included: Some("true".to_owned()),
        upper_included: Some("true".to_owned()),
        lower_unbounded: Some("false".to_owned()),
        upper_unbounded: Some("false".to_owned()),
        lower: Some(lower),
        upper: Some(upper),
    }
}

/// The minimal conforming model-form archetype: every XSD-mandatory member of
/// `AUTHORED_ARCHETYPE` and of the types it reaches, and nothing optional beyond
/// what is needed to exercise one `C_ATTRIBUTE` → `C_OBJECT` child (so the
/// polymorphic `xsi:type` slot is covered, not just the scalar envelope).
fn minimal_archetype() -> AuthoredArchetype {
    AuthoredArchetype {
        // AUTHORED_RESOURCE (Resource.xsd).
        original_language: TerminologyCode {
            terminology_id: "ISO_639-1".to_owned(),
            code_string: "en".to_owned(),
        },
        is_controlled: None,
        description: None,
        translations: Vec::new(),
        uid: None,
        annotations: Vec::new(),
        // The `xs:group ref="ARCHETYPE"` body (Archetype.xsd).
        archetype_id: ArchetypeHrid {
            namespace: Some("org.openehr".to_owned()),
            rm_publisher: Some("openEHR".to_owned()),
            rm_package: Some("EHR".to_owned()),
            rm_class: Some("CLUSTER".to_owned()),
            release_version: Some("1.0.0".to_owned()),
            version_status: Some("released".to_owned()),
            build_count: Some("0".to_owned()),
            concept_id: Some("aom2_model_gate".to_owned()),
        },
        parent_archetype_id: None,
        definition: CComplexObject {
            is_deprecated: None,
            node_id: Some("id1".to_owned()),
            rm_type_name: Some("CLUSTER".to_owned()),
            sibling_order: Vec::new(),
            occurrences: Some(interval(1, 1)),
            is_frozen: None,
            attributes: vec![CAttribute {
                rm_attribute_name: "items".to_owned(),
                existence: interval(1, 1),
                differential_path: None,
                is_multiple: true,
                cardinality: Cardinality {
                    is_ordered: true,
                    is_unique: false,
                    interval: interval(1, 1),
                },
                children: vec![CObject::CString(CString {
                    is_deprecated: None,
                    node_id: Some("id2".to_owned()),
                    rm_type_name: Some("ELEMENT".to_owned()),
                    sibling_order: Vec::new(),
                    occurrences: Some(interval(0, 1)),
                    is_frozen: None,
                    assumed_value: None,
                    constraint: "the_only_value".to_owned(),
                    default_value: None,
                })],
            }],
        },
        rules: Vec::new(),
        terminology: ArchetypeTerminology {
            term_definitions: vec![Codedefinitionset {
                id: "en".to_owned(),
                items: vec![ArchetypeTerm {
                    id: "id1".to_owned(),
                    text: "AOM2 model-form gate".to_owned(),
                    description: None,
                }],
            }],
            term_bindings: Vec::new(),
            value_sets: Vec::new(),
        },
        is_specialised: None,
        // AUTHORED_ARCHETYPE's own additions.
        build_uid: None,
        adl_version: Some("2.4.0".to_owned()),
        is_generated: None,
        other_metadata: None,
    }
}

/// A minimal conforming model-form archetype serializes, re-parses, and compares
/// equal — the generated `ToXml`/`FromXml` pair is symmetric over every mandatory
/// member, including the polymorphic `C_OBJECT` child slot.
#[test]
fn model_form_archetype_round_trips() -> Result<(), XmlError> {
    let first = minimal_archetype();
    let xml = openehr_its::aom2_model::to_xml(&first)?;
    let second = openehr_its::aom2_model::from_xml(&xml)?;
    assert_eq!(
        second, first,
        "model-form archetype did not survive to_xml → from_xml; serialized:\n{xml}"
    );
    // Re-serializing the parsed value must reproduce the same bytes, so the
    // element order the emitter writes is also the order the reader accepts.
    let reprinted = openehr_its::aom2_model::to_xml(&second)?;
    assert_eq!(
        reprinted, xml,
        "re-serializing the parsed model-form archetype changed the wire bytes"
    );
    Ok(())
}

/// The serialized envelope is the one `Archetype.xsd` declares: root element
/// `<archetype>` in the ITS-XML v1 target namespace
/// (`targetNamespace="http://schemas.openehr.org/v1"`).
#[test]
fn model_form_wire_envelope() -> Result<(), XmlError> {
    let xml = openehr_its::aom2_model::to_xml(&minimal_archetype())?;
    assert!(
        xml.contains("<archetype"),
        "root element is not `<archetype>`:\n{xml}"
    );
    assert!(
        xml.contains("xmlns=\"http://schemas.openehr.org/v1\""),
        "root element does not carry the ITS-XML v1 namespace:\n{xml}"
    );
    Ok(())
}

/// The archetype body reached through `<xs:group ref="ARCHETYPE"/>` survives the
/// round trip with its content intact.
///
/// This is the load-bearing assertion of this gate: the whole model-form
/// archetype body (`archetype_id`, `definition`, `terminology`, …) is declared
/// behind an `xs:group` reference, so a codec that ignored group references would
/// still round-trip — vacuously, over an empty envelope.
#[test]
fn model_form_group_body_survives() -> Result<(), XmlError> {
    let xml = openehr_its::aom2_model::to_xml(&minimal_archetype())?;
    let parsed = openehr_its::aom2_model::from_xml(&xml)?;
    assert_eq!(
        parsed.archetype_id.concept_id.as_deref(),
        Some("aom2_model_gate")
    );
    assert_eq!(parsed.definition.rm_type_name.as_deref(), Some("CLUSTER"));
    assert_eq!(parsed.definition.attributes.len(), 1);
    let attribute =
        parsed.definition.attributes.first().ok_or_else(|| {
            XmlError::Parse("the round-tripped definition lost its attribute".into())
        })?;
    assert_eq!(attribute.rm_attribute_name, "items");
    assert_eq!(attribute.children.len(), 1);
    assert_eq!(parsed.terminology.term_definitions.len(), 1);
    Ok(())
}
