// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![expect(clippy::panic, reason = "test assertions/diagnostics/fixtures")]
//! A FLAT or STRUCTURED value is never dropped on write (#5768).
//!
//! ITS-REST `simplified_formats` master05 puts a value on a data value node,
//! the nodes its per-type tables map, and master04 §Validation checks that
//! "field identifiers match WT metadata structure". A key that gives a value
//! on a node with no value of its own (an ELEMENT whose `value` admits several
//! types, or a structural node) is therefore refused with the key and, for an
//! ELEMENT, the alternatives the value belongs on. The carrier is
//! `Demonstration.v1`, whose "Choice" (`at0025`) allows `DV_QUANTITY` or
//! `DV_CODED_TEXT`.

use openehr_its::opt14;
use openehr_sdt::flat::convert::{
    composition_from_flat, composition_from_structured, composition_to_flat,
    composition_to_structured, flat_to_structured,
};
use openehr_sdt::flat::error::FlatError;
use openehr_sdt::flat::example::{DetailLevel, example_composition};
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::{WebTemplate, WebTemplateNode};
use openehr_sdt::rm_instance::validate_composition;
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

/// The refusal a FLAT document must meet.
fn refused(doc: &Map<String, Value>, wt: &WebTemplate) -> FlatError {
    match composition_from_flat(doc, wt, NOW) {
        Ok(composition) => panic!("{doc:#?} must be refused, built {composition:#}"),
        Err(e) => e,
    }
}

fn choice_value_refusal(e: &FlatError) {
    let FlatError::ValueWithoutInput {
        path,
        rm_type,
        alternatives,
    } = e
    else {
        panic!("a value without an input: {e:?}");
    };
    assert!(
        path.contains("/heading1:0/choice"),
        "the key is named: {path}"
    );
    assert_eq!(rm_type, "ELEMENT", "the node is the ELEMENT");
    assert_eq!(
        alternatives,
        &["quantity_value", "coded_text_value"],
        "the refusal names where the value belongs"
    );
    let message = e.to_string();
    assert!(
        message.contains("quantity_value") && message.contains("coded_text_value"),
        "{message}"
    );
}

#[test]
fn a_value_on_an_element_with_alternatives_is_refused() {
    let wt = web_template();
    choice_value_refusal(&refused(&flat(&[("choice", json!(5))]), &wt));
}

#[test]
fn a_value_beside_a_null_flavour_is_refused_and_never_stored() {
    let wt = web_template();
    let doc = flat(&[
        ("choice", json!(5)),
        ("choice/_null_flavour|code", json!("271")),
        ("choice/_null_flavour|value", json!("no information")),
        ("choice/_null_flavour|terminology", json!("openehr")),
    ]);
    choice_value_refusal(&refused(&doc, &wt));
}

#[test]
fn a_structured_value_on_an_element_with_alternatives_is_refused() {
    let wt = web_template();
    let structured = flat_to_structured(&flat(&[("choice", json!(5))]))
        .unwrap_or_else(|e| panic!("FLAT to STRUCTURED: {e}"));
    let e = match composition_from_structured(&structured, &wt, NOW) {
        Ok(composition) => panic!("must be refused, built {composition:#}"),
        Err(e) => e,
    };
    choice_value_refusal(&e);
}

#[test]
fn a_value_on_an_alternative_commits_and_round_trips() {
    let wt = web_template();
    let doc = flat(&[
        ("choice/quantity_value|magnitude", json!(5.0)),
        ("choice/quantity_value|unit", json!("g")),
    ]);
    let composition =
        composition_from_flat(&doc, &wt, NOW).unwrap_or_else(|e| panic!("FLAT builds: {e}"));
    let findings = validate_composition(&composition, &wt);
    assert!(findings.is_empty(), "{findings:#?}");
    let back = composition_to_flat(&composition, &wt).expect("FLAT emits");
    for key in [
        "choice/quantity_value|magnitude",
        "choice/quantity_value|unit",
    ] {
        assert_eq!(
            back.get(&format!("{HEADING}/{key}")),
            doc.get(&format!("{HEADING}/{key}")),
            "{key}"
        );
    }
    let structured = composition_to_structured(&composition, &wt).expect("STRUCTURED emits");
    let rebuilt = composition_from_structured(&structured, &wt, NOW)
        .unwrap_or_else(|e| panic!("STRUCTURED builds: {e}"));
    assert_eq!(rebuilt, composition);
}

/// The FLAT key of every node that takes no value of its own, with an index
/// on each repeating segment.
fn valueless_keys(node: &WebTemplateNode, prefix: &str, out: &mut Vec<String>) {
    for child in &node.children {
        let segment = if child.max == 1 {
            child.id.clone()
        } else {
            format!("{}:0", child.id)
        };
        let key = format!("{prefix}/{segment}");
        if !child.children.is_empty() && child.inputs.is_empty() {
            out.push(key.clone());
        }
        valueless_keys(child, &key, out);
    }
}

#[test]
fn a_value_on_any_node_without_an_input_is_refused() {
    let wt = web_template();
    let mut keys = Vec::new();
    valueless_keys(&wt.tree, &wt.tree.id, &mut keys);
    assert!(keys.len() > 10, "{keys:?}");
    for key in keys {
        let mut doc = flat(&[]);
        doc.insert(key.clone(), json!(5));
        match composition_from_flat(&doc, &wt, NOW) {
            Err(FlatError::ValueWithoutInput { path, .. }) => {
                let id = key.rsplit('/').next().and_then(|s| s.split(':').next());
                assert!(id.is_some_and(|id| path.contains(id)), "{key}: {path}");
            }
            Err(e) => panic!("{key}: refused for another reason: {e}"),
            Ok(_) => panic!("{key}: a value on a node without an input is accepted"),
        }
    }
}

/// `value` with one change a converter cannot read the same way.
fn perturbed(value: &Value) -> Value {
    match value {
        Value::Bool(b) => json!(!b),
        Value::Number(n) => n.as_i64().map_or_else(
            || json!(n.as_f64().unwrap_or_default() + 1.5),
            |i| json!(i + 1),
        ),
        Value::String(s) => json!(format!("{s}x")),
        other => other.clone(),
    }
}

#[test]
fn no_flat_key_is_silently_ignored_on_write() {
    let wt = web_template();
    let example = example_composition(&wt, DetailLevel::Complete)
        .unwrap_or_else(|e| panic!("the example builds: {e}"));
    let doc = composition_to_flat(&example, &wt).expect("FLAT emits");
    let base = composition_from_flat(&doc, &wt, NOW)
        .unwrap_or_else(|e| panic!("the example commits: {e}"));
    let again = composition_from_flat(&doc, &wt, NOW).unwrap_or_else(|e| panic!("rebuilds: {e}"));
    assert_eq!(base, again, "the build is deterministic");
    assert!(doc.len() > 50, "{}", doc.len());
    let mut ignored = Vec::new();
    for (key, value) in &doc {
        let mut changed = doc.clone();
        changed.insert(key.clone(), perturbed(value));
        if let Ok(built) = composition_from_flat(&changed, &wt, NOW)
            && built == base
        {
            ignored.push(key.clone());
        }
    }
    assert!(
        ignored.is_empty(),
        "keys whose value changes nothing: {ignored:#?}"
    );
}

fn proportion(magnitude: Value) -> Map<String, Value> {
    flat(&[
        ("proportion|numerator", json!(20.5)),
        ("proportion|denominator", json!(12.4)),
        ("proportion|type", json!(0)),
        ("proportion", magnitude),
    ])
}

#[test]
fn a_bare_proportion_value_that_disagrees_with_its_quotient_is_refused() {
    let wt = web_template();
    // master05 §DV_PROPORTION: the bare value is the magnitude, calculated
    // as numerator / denominator (20.5 / 12.4 = 1.6532258064516128). A value
    // agrees when it equals the quotient to the decimals it is written with.
    for agreeing in [
        json!(1.653_225_806_451_612_8),
        json!(1.65),
        json!(1.7),
        json!(2),
    ] {
        composition_from_flat(&proportion(agreeing.clone()), &wt, NOW)
            .unwrap_or_else(|e| panic!("{agreeing} agrees with the quotient: {e}"));
    }
    for disagreeing in [json!(1.8), json!(1.66), json!(3), json!("1.65")] {
        let e = refused(&proportion(disagreeing.clone()), &wt);
        assert!(
            matches!(&e, FlatError::InvalidValue { path, .. } if path.contains("/proportion")),
            "{disagreeing}: {e:?}"
        );
    }
}
