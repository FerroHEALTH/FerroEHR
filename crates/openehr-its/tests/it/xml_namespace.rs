// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![expect(clippy::expect_used, reason = "test assertions/diagnostics/fixtures")]
//! The two published ITS-XML wire lineages over ONE generated codec.
//!
//! `docs/specs/openehr/ITS-XML/README.adoc` §"Releases and IM Versions"
//! describes the `Release-2.0.0` change to the DOCUMENTS: it is "a major
//! version" in which "the internal namespace used in the schemas is also
//! changed to `http://schemas.openehr.org/v2`", with the older STABLE bundle
//! still published at `Release-1.0.2`. This gate pins exactly that much and no
//! more: our SERIALIZER's two outputs are byte-identical apart from the root
//! `xmlns`, the default entry point emits v1, the parameterized one emits
//! whichever lineage it is handed, and the reader accepts both.
//!
//! **That is a statement about the codec, NOT about the schemas.** The two
//! published XSD bundles do NOT differ by namespace alone: the same 2.0.0
//! release restructured the repository into per-component, per-RM-release
//! folders and the flat `Release-1.0.2v2` bundle was never re-issued against a
//! newer RM, so it is frozen at an RM generation older than the RM 1.2.0 model
//! this codec writes — 50 concrete RM classes have no complexType there at all
//! and 23 attributes over 17 more are undeclared, `FOLDER.details` among them.
//! The sweep, its per-attribute adjudications and the wire proof live in
//! `xml_xsd_validity`; the assertions below deliberately
//! stay scoped to the serializer, which is all they ever proved.

use openehr_its::xml::runtime::Namespace;
use openehr_its::xml::{from_canonical_xml, to_canonical_xml, to_canonical_xml_ns};
use openehr_rm::prelude::Composition;

const V1_NS: &str = "xmlns=\"http://schemas.openehr.org/v1\"";
const V2_NS: &str = "xmlns=\"http://schemas.openehr.org/v2\"";

fn fixture() -> Composition {
    let json =
        include_str!("../vendor/openehr_sdk/composition/canonical_json/minimal_evaluation.json");
    openehr_its::json::from_canonical_json(json).expect("deserialize composition JSON")
}

/// The default entry point emits the v2 lineage — the only vendored bundle
/// whose schemas describe every RM 1.2.0 class this model emits, matching the
/// served default (#2453, aligning with the #1666 ruling).
#[test]
fn default_entry_point_emits_the_v2_lineage() {
    let xml = to_canonical_xml(&fixture(), "composition").expect("serialize default");
    assert!(xml.contains(V2_NS), "default lineage is v2: {xml:.200}");
    assert!(!xml.contains(V1_NS), "default lineage is not v1");
    assert_eq!(
        xml,
        to_canonical_xml_ns(&fixture(), "composition", Namespace::V2).expect("serialize v2"),
        "to_canonical_xml is to_canonical_xml_ns(.., V2)"
    );
}

/// The v2 selection changes the SERIALIZED DOCUMENT's root namespace and
/// nothing else — the README's own statement of the 2.0.0 delta, applied to
/// our output. (What the two bundles' SCHEMAS accept differs; see the module
/// docs and `xml_xsd_validity`.)
#[test]
fn v2_selection_changes_only_the_root_namespace() {
    let v1 = to_canonical_xml_ns(&fixture(), "composition", Namespace::V1).expect("serialize v1");
    let v2 = to_canonical_xml_ns(&fixture(), "composition", Namespace::V2).expect("serialize v2");

    assert!(v2.starts_with("<composition"), "root element: {v2:.80}");
    assert!(v2.contains(V2_NS), "v2 namespace declared");
    assert!(!v2.contains(V1_NS), "v1 namespace not declared");
    assert!(
        v2.contains("xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\""),
        "the xsi namespace is declared in both lineages"
    );
    assert_eq!(
        v1.replace(V1_NS, V2_NS),
        v2,
        "the two lineages differ by the root xmlns alone"
    );
}

/// The reader is namespace-agnostic: a v2 document round-trips through the
/// same generated codec and re-serializes to the same v2 bytes.
#[test]
fn v2_document_round_trips() {
    let v2 = to_canonical_xml_ns(&fixture(), "composition", Namespace::V2).expect("serialize v2");
    let back: Composition = from_canonical_xml(&v2).expect("parse the v2 document");
    assert_eq!(
        to_canonical_xml_ns(&back, "composition", Namespace::V2).expect("re-serialize v2"),
        v2,
        "v2 → parse → v2 is lossless"
    );
    // …and the SAME parsed value re-serializes into the v1 lineage, which is
    // what makes namespace selection a pure presentation choice.
    assert!(
        to_canonical_xml_ns(&back, "composition", Namespace::V1)
            .expect("re-serialize v1")
            .contains(V1_NS),
        "a v2-parsed value can be emitted as v1"
    );
}

/// #1775 — the published-roots table (the ONE statement of the
/// published-document-element fact) agrees with the vendored schemas: each
/// entry's element + declared type + abstractness is re-derived from the XSD
/// text on every run.
#[test]
fn published_roots_table_matches_the_vendored_schemas() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas/xml/its-xml-1.0.2-nsv1/ALL");
    let read = |f: &str| std::fs::read_to_string(dir.join(f)).expect("vendored schema");
    let composition = read("Composition.xsd");
    let version = read("Version.xsd");
    let structure = read("Structure.xsd");
    for root in openehr_its::xml::PUBLISHED_ROOTS {
        let decl = format!(
            r#"<xs:element name="{}" type="{}"/>"#,
            root.element, root.declared_type
        );
        let (schema, name) = match root.element {
            "composition" => (&composition, "Composition.xsd"),
            "version" => (&version, "Version.xsd"),
            "items" => (&structure, "Structure.xsd"),
            other => panic!("no vendored schema mapped for published root {other:?}"),
        };
        assert!(schema.contains(&decl), "{name} must declare {decl}");
        let abstract_decl = format!(
            r#"<xs:complexType name="{}" abstract="true">"#,
            root.declared_type
        );
        assert_eq!(
            schema.contains(&abstract_decl),
            root.type_is_abstract,
            "abstractness of {} in {name} must match the table",
            root.declared_type
        );
    }
}
