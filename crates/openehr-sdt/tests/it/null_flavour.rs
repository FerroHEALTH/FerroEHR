// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![expect(clippy::panic, reason = "test assertions/diagnostics/fixtures")]
//! An ELEMENT's `null_flavour` constraint is an attribute of the ELEMENT, never
//! a value alternative (#5769).
//!
//! RM `data_structures` §ELEMENT gives the ELEMENT `value` and `null_flavour`
//! as separate attributes (`Inv_null_flavour_indicated`: `is_null() xor
//! null_flavour = Void`). ITS-REST `simplified_formats` master05 §ELEMENT
//! addresses the null flavour as `/_null_flavour`, and master04 §"Flat
//! format" makes an ELEMENT its value ("elements ARE their values"), so an
//! ELEMENT with one `value` type is promoted to it whatever its `null_flavour`
//! constraint says. The carrier is `Demonstration.v1`, whose "Boolean"
//! (`at0016`) constrains `value` to `DV_BOOLEAN` and `null_flavour` to four
//! openEHR codes, and whose "Choice" (`at0025`) allows `DV_QUANTITY` or
//! `DV_CODED_TEXT`.

use openehr_its::opt14;
use openehr_rm::prelude::Composition;
use openehr_sdt::flat::convert::{
    composition_from_flat, composition_from_structured, composition_to_flat,
    composition_to_structured,
};
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::{WebTemplate, WebTemplateNode};
use openehr_sdt::rm_instance::{ValidationKind, validate_composition};
use serde_json::{Map, Value, json};

const HEADING: &str = "encounter/demonstration:0/any_event/heading1:0";
const NOW: &str = "2026-11-16T10:00:00Z";

fn web_template() -> WebTemplate {
    let xml = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/demonstration/Demonstration.v1.opt"
    ))
    .unwrap_or_else(|e| panic!("read Demonstration.v1 OPT: {e}"));
    let opt = opt14::from_xml(&xml).unwrap_or_else(|e| panic!("parse OPT: {e}"));
    build_web_template(&opt).unwrap_or_else(|e| panic!("build WT: {e}"))
}

/// Every node of the tree whose `aqlPath` ends with `suffix`.
fn nodes_ending<'a>(node: &'a WebTemplateNode, suffix: &str, out: &mut Vec<&'a WebTemplateNode>) {
    if node.aql_path.ends_with(suffix) {
        out.push(node);
    }
    for child in &node.children {
        nodes_ending(child, suffix, out);
    }
}

/// A FLAT `Demonstration.v1` body: the context, the one mandatory heading
/// element, and `extra` under the heading.
fn flat(extra: &[(&str, Value)]) -> Map<String, Value> {
    let mut doc = Map::new();
    doc.insert("ctx/language".to_owned(), json!("en"));
    doc.insert("ctx/territory".to_owned(), json!("NL"));
    doc.insert("ctx/composer_name".to_owned(), json!("Dr. Ada Synthetic"));
    doc.insert(
        "encounter/demonstration:0/any_event/time".to_owned(),
        json!(NOW),
    );
    let external = format!("{HEADING}/text_that_is_sourced_from_an_external_terminology");
    doc.insert(format!("{external}|code"), json!("38341003"));
    doc.insert(format!("{external}|value"), json!("Hypertension"));
    doc.insert(format!("{external}|terminology"), json!("SNOMED-CT"));
    for (k, v) in extra {
        doc.insert(format!("{HEADING}/{k}"), v.clone());
    }
    doc
}

fn null_flavour_271() -> [(&'static str, Value); 3] {
    [
        ("boolean/_null_flavour|code", json!("271")),
        ("boolean/_null_flavour|value", json!("no information")),
        ("boolean/_null_flavour|terminology", json!("openehr")),
    ]
}

/// The ELEMENT `code` under the heading CLUSTER of a built composition.
fn element<'a>(composition: &'a Value, code: &str) -> &'a Value {
    composition
        .pointer("/content/0/data/events/0/data/items")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|i| i.get("archetype_node_id") == Some(&json!("at0004")))
        })
        .and_then(|cluster| cluster.get("items"))
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|i| i.get("archetype_node_id") == Some(&json!(code)))
        })
        .unwrap_or_else(|| panic!("no element {code} in {composition:#}"))
}

/// Builds `doc` and asserts it commits with no finding.
fn commits(doc: &Map<String, Value>, wt: &WebTemplate) -> Value {
    let composition =
        composition_from_flat(doc, wt, NOW).unwrap_or_else(|e| panic!("FLAT builds: {e}"));
    let findings = validate_composition(&composition, wt);
    assert!(findings.is_empty(), "{doc:#?} commits: {findings:#?}");
    composition
}

fn under_heading<'a>(flat: &'a Map<String, Value>, element: &str) -> Vec<(&'a str, &'a Value)> {
    let prefix = format!("{HEADING}/{element}");
    flat.iter()
        .filter(|(k, _)| {
            k.strip_prefix(&prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '|']))
        })
        .map(|(k, v)| (k.as_str(), v))
        .collect()
}

#[test]
fn the_web_template_promotes_a_single_value_element_beside_its_null_flavour() {
    let wt = web_template();
    let mut null_flavours = Vec::new();
    nodes_ending(&wt.tree, "/null_flavour", &mut null_flavours);
    assert!(
        null_flavours.is_empty(),
        "no node presents a null flavour as a value: {:?}",
        null_flavours
            .iter()
            .map(|n| &n.aql_path)
            .collect::<Vec<_>>()
    );
    let mut booleans = Vec::new();
    nodes_ending(&wt.tree, "/items[at0016]/value", &mut booleans);
    assert_eq!(booleans.len(), 4, "one Boolean per event");
    for boolean in booleans {
        assert_eq!(boolean.id, "boolean");
        assert_eq!(boolean.rm_type, "DV_BOOLEAN");
        assert!(boolean.children.is_empty());
    }
    let document = serde_json::to_string(&wt).expect("the web template serializes");
    assert!(!document.contains("/null_flavour"), "{document}");
}

#[test]
fn a_boolean_commits_on_the_element_key() {
    let wt = web_template();
    let composition = commits(&flat(&[("boolean", json!(true))]), &wt);
    let boolean = element(&composition, "at0016");
    assert_eq!(
        boolean.get("value"),
        Some(&json!({"_type": "DV_BOOLEAN", "value": true}))
    );
    assert!(boolean.get("null_flavour").is_none(), "{boolean:#}");
}

#[test]
fn a_null_flavour_commits_as_the_element_attribute() {
    let wt = web_template();
    let composition = commits(&flat(&null_flavour_271()), &wt);
    let boolean = element(&composition, "at0016");
    assert!(boolean.get("value").is_none(), "{boolean:#}");
    assert_eq!(
        boolean.pointer("/null_flavour/defining_code/code_string"),
        Some(&json!("271"))
    );
}

#[test]
fn a_flat_read_of_a_null_flavoured_element_emits_only_the_null_flavour() {
    let wt = web_template();
    let composition = commits(&flat(&null_flavour_271()), &wt);
    let back = composition_to_flat(&composition, &wt).expect("FLAT emits");
    let mut keys: Vec<&str> = under_heading(&back, "boolean")
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    keys.sort_unstable();
    let expected: Vec<String> = ["code", "terminology", "value"]
        .iter()
        .map(|s| format!("{HEADING}/boolean/_null_flavour|{s}"))
        .collect();
    assert_eq!(keys, expected);
}

#[test]
fn a_structured_read_of_a_null_flavoured_element_emits_only_the_null_flavour() {
    let wt = web_template();
    let composition = commits(&flat(&null_flavour_271()), &wt);
    let structured = composition_to_structured(&composition, &wt).expect("STRUCTURED emits");
    let boolean =
        &structured["encounter"]["demonstration"][0]["any_event"][0]["heading1"][0]["boolean"][0];
    let members: Vec<&String> = boolean
        .as_object()
        .unwrap_or_else(|| panic!("a boolean object: {structured:#}"))
        .keys()
        .collect();
    assert_eq!(members, ["_null_flavour"], "{boolean:#}");
    assert_eq!(boolean["_null_flavour"][0]["|code"], json!("271"));
    let rebuilt = composition_from_structured(&structured, &wt, NOW)
        .unwrap_or_else(|e| panic!("STRUCTURED builds: {e}"));
    assert_eq!(element(&rebuilt, "at0016"), element(&composition, "at0016"));
}

#[test]
fn a_null_flavoured_element_round_trips_through_flat() {
    let wt = web_template();
    let composition = commits(&flat(&null_flavour_271()), &wt);
    let back = composition_to_flat(&composition, &wt).expect("FLAT emits");
    let rebuilt = commits(&back, &wt);
    assert_eq!(element(&rebuilt, "at0016"), element(&composition, "at0016"));
}

#[test]
fn a_boolean_round_trips_through_flat_structured_and_canonical_json() {
    let wt = web_template();
    let composition = commits(&flat(&[("boolean", json!(false))]), &wt);
    let back = composition_to_flat(&composition, &wt).expect("FLAT emits");
    assert_eq!(
        under_heading(&back, "boolean"),
        [(format!("{HEADING}/boolean").as_str(), &json!(false))]
    );
    let structured = composition_to_structured(&composition, &wt).expect("STRUCTURED emits");
    let from_structured = composition_from_structured(&structured, &wt, NOW)
        .unwrap_or_else(|e| panic!("STRUCTURED builds: {e}"));
    let typed: Composition = openehr_its::json::from_canonical_value(&composition)
        .unwrap_or_else(|e| panic!("canonical JSON decodes: {e}"));
    let canonical = openehr_its::json::to_canonical_value(&typed);
    for rebuilt in [commits(&back, &wt), from_structured, canonical] {
        assert_eq!(element(&rebuilt, "at0016"), element(&composition, "at0016"));
        let findings = validate_composition(&rebuilt, &wt);
        assert!(findings.is_empty(), "{findings:#?}");
    }
}

#[test]
fn a_null_flavour_outside_the_template_list_is_refused() {
    let wt = web_template();
    let mut composition = commits(&flat(&null_flavour_271()), &wt);
    let items = composition
        .pointer_mut("/content/0/data/events/0/data/items/0/items")
        .and_then(Value::as_array_mut)
        .unwrap_or_else(|| panic!("heading items"));
    let boolean = items
        .iter_mut()
        .find(|i| i.get("archetype_node_id") == Some(&json!("at0016")))
        .unwrap_or_else(|| panic!("the Boolean element"));
    boolean["null_flavour"]["defining_code"]["code_string"] = json!("999");
    let findings = validate_composition(&composition, &wt);
    assert!(
        findings.iter().any(|f| f.kind == ValidationKind::CodedValue
            && f.path.ends_with("/items[at0016]/null_flavour")),
        "the template's null flavour list is enforced: {findings:#?}"
    );
}

#[test]
fn a_choice_keeps_its_alternatives_and_takes_a_null_flavour_on_the_element() {
    let wt = web_template();
    let mut choices = Vec::new();
    nodes_ending(&wt.tree, "/items[at0025]", &mut choices);
    let first = choices.first().unwrap_or_else(|| panic!("a choice node"));
    let ids: Vec<&str> = first.children.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["quantity_value", "coded_text_value"]);
    let composition = commits(
        &flat(&[
            ("choice/_null_flavour|code", json!("253")),
            ("choice/_null_flavour|value", json!("unknown")),
            ("choice/_null_flavour|terminology", json!("openehr")),
        ]),
        &wt,
    );
    let choice = element(&composition, "at0025");
    assert!(choice.get("value").is_none(), "{choice:#}");
    assert_eq!(
        choice.pointer("/null_flavour/defining_code/code_string"),
        Some(&json!("253"))
    );
    let back = composition_to_flat(&composition, &wt).expect("FLAT emits");
    let rebuilt = commits(&back, &wt);
    assert_eq!(element(&rebuilt, "at0025"), choice);
}
