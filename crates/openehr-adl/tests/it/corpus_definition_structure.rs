// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Structural spot-checks of the cADL parser against real vendored corpus
//! files: whole `.adls` sources (outer parse + the definition span, exactly
//! what every caller of the entry points does), with the deep AOM2 tree
//! asserted against expectations hand-derived by reading each file.
//!
//! These read `tests/corpus/**` through [`include_str!`], which is why they
//! live here rather than beside the parser: `tests/` is outside the crate's
//! published package (`include = ["src/**", …]`), so the same code under
//! `src/` compiles for us and fails for anyone running `cargo test` inside the
//! published `.crate` (#2385).

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::{
    CComplexObject, CComplexObjectData,
};
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_object::CPrimitiveObject;
use openehr_base::prelude::{Interval, ProperInterval};

use openehr_adl::parse::{Dialect, parse_definition_body};
use openehr_adl::source::parse_source;

/// A plain complex object, refusing the archetype-root arm.
fn data(cco: &CComplexObject) -> &CComplexObjectData {
    match cco {
        CComplexObject::CComplexObject(d) => d,
        CComplexObject::CArchetypeRoot(_) => panic!("expected plain complex object"),
    }
}

/// The root artefact's `definition` section of a whole ADL2 source: the
/// outer parse of [`parse_source`] plus
/// [`parse_definition_body`] over the definition span.
fn parse_source_definition(src: &str) -> CComplexObject {
    let artefact =
        parse_source(src, Dialect::Adl2).unwrap_or_else(|e| panic!("outer parse: {e:?}"));
    let def = artefact
        .definition
        .as_ref()
        .unwrap_or_else(|| panic!("no definition section"));
    let body = src.get(def.bytes.clone()).unwrap_or_default();
    parse_definition_body(body, Dialect::Adl2).unwrap_or_else(|e| panic!("parse: {e:?}"))
}

/// Fetch the named attribute of a complex object.
fn attr<'a>(d: &'a CComplexObjectData, name: &str) -> &'a CAttribute {
    d.attributes
        .iter()
        .flatten()
        .find(|a| a.rm_attribute_name == name)
        .unwrap_or_else(|| panic!("no attribute {name:?}"))
}

/// The first child of an attribute, as a plain complex object.
fn first_cco(a: &CAttribute) -> &CComplexObjectData {
    match &a.children.as_deref().unwrap_or_default()[0] {
        CObject::CComplexObject(CComplexObject::CComplexObject(d)) => d,
        other => panic!("expected complex object child, got {other:?}"),
    }
}

#[test]
fn corpus_ordinal_tuple_structure() {
    let src = include_str!(
        "../corpus/adl2-reference/features/aom_structures/tuples/openEHR-EHR-OBSERVATION.ordinal_tuple.v1.0.0.adls"
    );
    let cco = parse_source_definition(src);
    let root = data(&cco);
    assert_eq!(root.rm_type_name, "OBSERVATION");
    assert_eq!(root.node_id, "id1");

    // OBSERVATION/data/HISTORY[id3]/events/POINT_EVENT[id4]/data/
    // ITEM_LIST[id2]/items/ELEMENT[id10]/value/DV_ORDINAL[id11].
    let history = first_cco(attr(root, "data"));
    assert_eq!(history.node_id, "id3");
    let point_event = first_cco(attr(history, "events"));
    assert_eq!(point_event.node_id, "id4");
    let pe_occ = point_event
        .occurrences
        .as_ref()
        .expect("POINT_EVENT occurrences");
    assert_eq!(pe_occ.lower, Some(0));
    assert_eq!(pe_occ.upper, Some(1));
    let item_list = first_cco(attr(point_event, "data"));
    assert_eq!(item_list.node_id, "id2");
    // `events` is an ordered/unordered container with a cardinality.
    let events = attr(history, "events");
    assert!(events.cardinality.is_some());
    let element = first_cco(attr(item_list, "items"));
    assert_eq!(element.node_id, "id10");
    // `items` cardinality {1..6; ordered}.
    let items = attr(item_list, "items");
    let card = items.cardinality.as_ref().expect("items cardinality");
    assert_eq!(card.interval.lower, Some(1));
    assert_eq!(card.interval.upper, Some(6));
    assert!(card.is_ordered);
    let ordinal = first_cco(attr(element, "value"));
    assert_eq!(ordinal.rm_type_name, "DV_ORDINAL");
    assert_eq!(ordinal.node_id, "id11");

    // The `[value, symbol]` tuple with three ordinal rows.
    assert_eq!(ordinal.attribute_tuples.as_ref().map_or(0, Vec::len), 1);
    let tuple = &ordinal.attribute_tuples.as_deref().unwrap_or_default()[0];
    assert_eq!(tuple.members.as_ref().map_or(0, Vec::len), 2);
    assert_eq!(
        tuple.members.as_deref().unwrap_or_default()[0].rm_attribute_name,
        "value"
    );
    assert_eq!(
        tuple.members.as_deref().unwrap_or_default()[1].rm_attribute_name,
        "symbol"
    );
    assert_eq!(tuple.tuples.as_ref().map_or(0, Vec::len), 3);
    match &tuple.tuples.as_deref().unwrap_or_default()[0].members[0] {
        CPrimitiveObject::CInteger(ci) => match &ci.constraint.as_deref().unwrap_or_default()[0] {
            Interval::PointInterval(p) => assert_eq!(p.lower, Some(0)),
            Interval::ProperInterval(_) => panic!("expected point 0"),
        },
        other => panic!("expected CInteger, got {other:?}"),
    }
    match &tuple.tuples.as_deref().unwrap_or_default()[0].members[1] {
        CPrimitiveObject::CTerminologyCode(t) => assert_eq!(t.constraint, "at11"),
        other => panic!("expected CTerminologyCode, got {other:?}"),
    }
}

#[test]
fn corpus_slot_structure() {
    let src = include_str!(
        "../corpus/adl2-reference/validity/slots/openEHR-EHR-SECTION.slot_parent.v1.0.0.adls"
    );
    let cco = parse_source_definition(src);
    let root = data(&cco);
    assert_eq!(root.rm_type_name, "SECTION");
    assert_eq!(root.node_id, "id1");

    // SECTION/items cardinality {1..*; unordered} matches { allow_archetype
    // OBSERVATION[id2] occurrences {0..1} matches { include… exclude… } }.
    let items = attr(root, "items");
    assert!(items.is_multiple);
    let card = items.cardinality.as_ref().expect("items cardinality");
    assert_eq!(card.interval.lower, Some(1));
    assert!(card.interval.upper_unbounded);
    assert!(!card.is_ordered);

    match &items.children.as_deref().unwrap_or_default()[0] {
        CObject::ArchetypeSlot(s) => {
            assert_eq!(s.rm_type_name, "OBSERVATION");
            assert_eq!(s.node_id, "id2");
            let occ = s.occurrences.as_ref().expect("slot occurrences");
            assert_eq!(occ.lower, Some(0));
            assert_eq!(occ.upper, Some(1));
            assert_eq!(s.includes.as_ref().map_or(0, Vec::len), 1);
            assert!(
                s.includes.as_deref().unwrap_or_default()[0]
                    .string_expression
                    .as_deref()
                    .unwrap_or_default()
                    .contains("archetype_id/value")
            );
            assert_eq!(s.excludes.as_ref().map_or(0, Vec::len), 1);
            assert!(!s.is_closed);
        }
        other => panic!("expected ArchetypeSlot, got {other:?}"),
    }
}

#[test]
fn corpus_primitive_types_structure() {
    let src = include_str!(
        "../corpus/adl2-reference/features/aom_structures/primitive_types/openehr-TEST_PKG-WHOLE.primitive_types.v1.0.0.adls"
    );
    let cco = parse_source_definition(src);
    let root = data(&cco);
    assert_eq!(root.node_id, "id1");
    // integer_attr3 == {|0..100|}.
    match &attr(root, "integer_attr3")
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CInteger(ci) => match &ci.constraint.as_deref().unwrap_or_default()[0] {
            Interval::ProperInterval(ProperInterval::ProperInterval(pi)) => {
                assert_eq!(pi.lower, Some(0));
                assert_eq!(pi.upper, Some(100));
            }
            _ => panic!("expected proper interval"),
        },
        _ => panic!("expected CInteger"),
    }
    // date_attr3 == {yyyy-mm-??} (a pattern).
    match &attr(root, "date_attr3")
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CDate(c) => assert_eq!(c.pattern_constraint.as_deref(), Some("yyyy-mm-??")),
        _ => panic!("expected CDate pattern"),
    }
    // duration_attr22 == {PWD/PT0S} (pattern + value).
    match &attr(root, "duration_attr22")
        .children
        .as_deref()
        .unwrap_or_default()[0]
    {
        CObject::CDuration(c) => {
            assert_eq!(c.pattern_constraint.as_deref(), Some("PWD"));
            assert_eq!(c.constraint.as_ref().map_or(0, Vec::len), 1);
        }
        _ => panic!("expected CDuration pattern+value"),
    }
}
