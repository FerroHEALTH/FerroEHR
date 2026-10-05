// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![expect(clippy::expect_used, reason = "test assertions/diagnostics/fixtures")]
//! The abstract-root serialization path (`to_canonical_xml_declared`).
//!
//! `ALL/Version.xsd` publishes exactly one global element for the change-control
//! package — `<xs:element name="version" type="VERSION"/>` — and its type is
//! `<xs:complexType name="VERSION" abstract="true">`. XML Schema Part 1 forbids
//! an element instance from using an abstract type directly: the instance must
//! select a non-abstract derived type with `xsi:type`
//! (<https://www.w3.org/TR/xmlschema-1/#xsi_type>, §2.6.1 + §3.4.6). So an
//! `ORIGINAL_VERSION` written under the published `<version>` root is
//! schema-valid only with `xsi:type="ORIGINAL_VERSION"`, which the plain
//! `to_canonical_xml` entry point (declared type `None` at the root) cannot
//! emit.

use openehr_its::xml::runtime::Namespace;
use openehr_its::xml::{from_canonical_xml, to_canonical_xml, to_canonical_xml_declared};
use openehr_rm::prelude::{Composition, OriginalVersion};
use serde_json::json;

/// An `ORIGINAL_VERSION<COMPOSITION>` over a corpus composition.
fn fixture() -> OriginalVersion<Composition> {
    let data: Composition = openehr_its::json::from_canonical_json(include_str!(
        "../vendor/openehr_sdk/composition/canonical_json/minimal_evaluation.json"
    ))
    .expect("typed composition");
    openehr_its::json::from_canonical_value(&json!({
        "_type": "ORIGINAL_VERSION",
        "contribution": {
            "_type": "OBJECT_REF", "namespace": "local", "type": "CONTRIBUTION",
            "id": { "_type": "HIER_OBJECT_ID", "value": "0198f4a5-9df1-7d1e-8b6f-2b8c00000abc" }
        },
        "commit_audit": {
            "_type": "AUDIT_DETAILS",
            "system_id": "ferroehr.local",
            "time_committed": { "_type": "DV_DATE_TIME", "value": "2026-07-29T10:11:12Z" },
            "change_type": {
                "_type": "DV_CODED_TEXT", "value": "creation",
                "defining_code": {
                    "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                    "code_string": "249"
                }
            },
            "committer": { "_type": "PARTY_IDENTIFIED", "name": "conformance tester" }
        },
        "uid": {
            "_type": "OBJECT_VERSION_ID",
            "value": "0198f4a5-9df1-7d1e-8b6f-2b8c00000001::ferroehr.local::1"
        },
        "lifecycle_state": {
            "_type": "DV_CODED_TEXT", "value": "complete",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "532"
            }
        },
        "data": openehr_its::json::to_canonical_value(&data)
    }))
    .expect("typed ORIGINAL_VERSION")
}

/// Under the published `<version>` root the concrete subtype is named, so the
/// document satisfies the abstract-type constraint.
#[test]
fn an_abstract_root_names_its_concrete_subtype() {
    let xml = to_canonical_xml_declared(&fixture(), "version", "VERSION", Namespace::V1)
        .expect("serialize under the published <version> root");
    assert!(xml.starts_with("<version "), "root element, got:\n{xml}");
    assert!(
        xml.contains(r#"xsi:type="ORIGINAL_VERSION""#),
        "an abstract declared type must be resolved by xsi:type, got:\n{xml}"
    );
    assert!(
        xml.contains(r#"xmlns="http://schemas.openehr.org/v1""#),
        "the v1 lineage"
    );
    // `VERSION.data` is `xs:anyType` in the XSD, so the payload names its type
    // by the same mechanism.
    assert!(
        xml.contains(r#"<data xsi:type="COMPOSITION""#),
        "the anyType data slot names its concrete type, got:\n{xml}"
    );
}

/// A root whose declared type IS the value's concrete type carries no
/// `xsi:type` — the attribute is dispatch, never decoration.
#[test]
fn a_matching_declared_root_type_emits_no_attribute() {
    let xml = to_canonical_xml_declared(
        &fixture(),
        "original_version",
        "ORIGINAL_VERSION",
        Namespace::V1,
    )
    .expect("serialize under a concrete root");
    assert!(
        !xml.contains("xsi:type=\"ORIGINAL_VERSION\""),
        "no xsi:type when the concrete type equals the declared one, got:\n{xml}"
    );
}

/// Reading is namespace- and `xsi:type`-agnostic at the root (the reader
/// dispatches on the Rust target type), so the abstract-root document
/// round-trips.
#[test]
fn the_abstract_root_document_round_trips() {
    let xml = to_canonical_xml_declared(&fixture(), "version", "VERSION", Namespace::V1)
        .expect("serialize");
    let back: OriginalVersion<Composition> = from_canonical_xml(&xml).expect("re-read");
    assert_eq!(
        to_canonical_xml(&back, "original_version").expect("re-serialize"),
        to_canonical_xml(&fixture(), "original_version").expect("serialize the fixture"),
        "the abstract-root document must carry the whole ORIGINAL_VERSION"
    );
}
