// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![expect(clippy::panic, reason = "test assertions/diagnostics/fixtures")]
//! An `ELEMENT.value` constrained by several alternative objects (#4226).
//!
//! ADL 1.4 `master05-cadl.adoc` §"Single-valued Attributes": the children of a
//! single-valued attribute are "alternative constraints, only one of which
//! needs to be matched by the data". The carrier is `value_alternatives.en.v1`,
//! a synthetic template whose four elements each carry two alternatives:
//!
//! * "Nucleic acid": `DV_TEXT` beside a local `DV_CODED_TEXT`, the shape of
//!   the reported `vcf_report` template. The web template presents it as one
//!   coded node with an open list and an `|other` input (ITS-REST
//!   `simplified_formats` master04 §"Open Value-Sets and the `|other` Suffix"),
//!   and `|other` commits a `DV_TEXT`;
//! * "Amount": `DV_QUANTITY` or `DV_COUNT`;
//! * "Recorded": `DV_DATE` or `DV_DATE_TIME`;
//! * "Speed": two `DV_QUANTITY` alternatives, `[mi_i]/h` up to 55 and `km/h`
//!   up to 100, the cADL example of the same section.

use openehr_its::opt14;
use openehr_rm::prelude::Composition;
use openehr_sdt::flat::convert::{
    composition_from_flat, composition_from_structured, composition_to_flat,
    composition_to_structured, flat_to_structured,
};
use openehr_sdt::flat::example::{DetailLevel, example_composition};
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::{WebTemplate, WebTemplateNode};
use openehr_sdt::rm_instance::{ValidationKind, ValidationMessage, validate_composition};
use serde_json::{Map, Value, json};

const OBS: &str = "value_alternatives/value_alternatives_observation:0";
const NOW: &str = "2026-10-27T10:00:00Z";

fn web_template() -> WebTemplate {
    let xml = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/value_alternatives/value_alternatives.opt"
    ))
    .unwrap_or_else(|e| panic!("read value_alternatives OPT: {e}"));
    let opt = opt14::from_xml(&xml).unwrap_or_else(|e| panic!("parse OPT: {e}"));
    build_web_template(&opt).unwrap_or_else(|e| panic!("build WT: {e}"))
}

fn find<'a>(node: &'a WebTemplateNode, id: &str) -> Option<&'a WebTemplateNode> {
    if node.id == id {
        return Some(node);
    }
    node.children.iter().find_map(|c| find(c, id))
}

fn flat(extra: &[(&str, Value)]) -> Map<String, Value> {
    let mut doc = Map::new();
    doc.insert("ctx/language".to_owned(), json!("en"));
    doc.insert("ctx/territory".to_owned(), json!("NL"));
    doc.insert("ctx/composer_name".to_owned(), json!("Dr. Ada Synthetic"));
    for (k, v) in extra {
        doc.insert(format!("{OBS}/{k}"), v.clone());
    }
    doc
}

/// The ELEMENT value at `items[code]` in a built composition.
fn element_value<'a>(composition: &'a Value, code: &str) -> &'a Value {
    composition
        .pointer("/content/0/data/events/0/data/items")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|i| i.get("archetype_node_id") == Some(&json!(code)))
        })
        .and_then(|element| element.get("value"))
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

/// Builds `doc` and returns its findings.
fn findings_of(doc: &Map<String, Value>, wt: &WebTemplate) -> Vec<ValidationMessage> {
    let composition =
        composition_from_flat(doc, wt, NOW).unwrap_or_else(|e| panic!("FLAT builds: {e}"));
    validate_composition(&composition, wt)
}

/// The canonical JSON route: the composition decoded into the typed RM,
/// encoded back to canonical JSON, and validated.
fn canonical_round_trip(composition: &Value) -> Value {
    let typed: Composition = openehr_its::json::from_canonical_value(composition)
        .unwrap_or_else(|e| panic!("canonical JSON decodes: {e}"));
    openehr_its::json::to_canonical_value(&typed)
}

#[test]
fn the_web_template_shows_the_coded_and_the_free_text_form() {
    let wt = web_template();
    let node = find(&wt.tree, "nucleic_acid").expect("nucleic_acid node");
    assert_eq!(node.rm_type, "DV_CODED_TEXT");
    let code = node
        .inputs
        .iter()
        .find(|i| i.suffix.as_deref() == Some("code"))
        .expect("a |code input");
    assert_eq!(code.list_open, Some(true), "the coded list is open");
    let codes: Vec<&str> = code.list.iter().map(|v| v.value.as_str()).collect();
    assert_eq!(codes, ["at0022", "at0023", "at0024"]);
    assert!(
        node.inputs
            .iter()
            .any(|i| i.suffix.as_deref() == Some("other")),
        "an |other input carries the free text"
    );
    let document = serde_json::to_string(&wt).expect("the web template serializes");
    assert!(
        !document.contains("\"alternatives\""),
        "the folded alternative stays out of the metadata document"
    );
}

#[test]
fn other_commits_a_dv_text_through_flat() {
    let wt = web_template();
    let doc = flat(&[("nucleic_acid|other", json!("DNA"))]);
    let composition = commits(&doc, &wt);
    assert_eq!(
        element_value(&composition, "at0004"),
        &json!({"_type": "DV_TEXT", "value": "DNA"})
    );
    let back = composition_to_flat(&composition, &wt).expect("FLAT emits");
    assert_eq!(
        back.get(&format!("{OBS}/nucleic_acid|other")),
        Some(&json!("DNA"))
    );
    assert!(!back.contains_key(&format!("{OBS}/nucleic_acid|code")));
}

#[test]
fn a_code_commits_a_dv_coded_text_through_flat() {
    let wt = web_template();
    let doc = flat(&[("nucleic_acid|code", json!("at0023"))]);
    let composition = commits(&doc, &wt);
    let value = element_value(&composition, "at0004");
    assert_eq!(value["_type"], json!("DV_CODED_TEXT"));
    assert_eq!(value["defining_code"]["code_string"], json!("at0023"));
    let back = composition_to_flat(&composition, &wt).expect("FLAT emits");
    assert_eq!(
        back.get(&format!("{OBS}/nucleic_acid|code")),
        Some(&json!("at0023"))
    );
    assert!(!back.contains_key(&format!("{OBS}/nucleic_acid|other")));
}

#[test]
fn both_forms_commit_through_structured() {
    let wt = web_template();
    for (suffix, value, rm_type) in [
        ("other", json!("RNA"), "DV_TEXT"),
        ("code", json!("at0024"), "DV_CODED_TEXT"),
    ] {
        let doc = flat(&[(&format!("nucleic_acid|{suffix}"), value)]);
        let structured = flat_to_structured(&doc).expect("FLAT to STRUCTURED");
        let leaf = &structured["value_alternatives"]["value_alternatives_observation"][0]["nucleic_acid"]
            [0];
        assert!(
            leaf.get(format!("|{suffix}")).is_some(),
            "STRUCTURED carries |{suffix}: {leaf:#}"
        );
        let composition = composition_from_structured(&structured, &wt, NOW)
            .unwrap_or_else(|e| panic!("STRUCTURED builds: {e}"));
        assert_eq!(
            element_value(&composition, "at0004")["_type"],
            json!(rm_type)
        );
        let findings = validate_composition(&composition, &wt);
        assert!(findings.is_empty(), "|{suffix} commits: {findings:#?}");
        let back = composition_to_structured(&composition, &wt).expect("STRUCTURED emits");
        assert_eq!(
            back["value_alternatives"]["value_alternatives_observation"][0]["nucleic_acid"][0]
                [format!("|{suffix}")],
            leaf[format!("|{suffix}")]
        );
    }
}

#[test]
fn both_forms_commit_through_canonical_json() {
    let wt = web_template();
    for value in [
        json!({"_type": "DV_TEXT", "value": "mtDNA"}),
        json!({
            "_type": "DV_CODED_TEXT",
            "value": "DNA",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "local"},
                "code_string": "at0022"
            }
        }),
    ] {
        let mut composition = commits(&flat(&[("nucleic_acid|code", json!("at0022"))]), &wt);
        composition["content"][0]["data"]["events"][0]["data"]["items"][0]["value"] = value.clone();
        let canonical = canonical_round_trip(&composition);
        assert_eq!(element_value(&canonical, "at0004"), &value);
        let findings = validate_composition(&canonical, &wt);
        assert!(findings.is_empty(), "{value} commits: {findings:#?}");
    }
}

#[test]
fn a_code_outside_the_open_list_matches_the_text_alternative() {
    // DV_CODED_TEXT conforms to DV_TEXT (RM data_types §DV_CODED_TEXT), so it
    // matches the unconstrained DV_TEXT alternative.
    let wt = web_template();
    let mut composition = commits(&flat(&[("nucleic_acid|code", json!("at0022"))]), &wt);
    composition["content"][0]["data"]["events"][0]["data"]["items"][0]["value"]["defining_code"]
        ["code_string"] = json!("at0099");
    let findings = validate_composition(&composition, &wt);
    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn a_value_no_alternative_admits_names_every_alternative() {
    let wt = web_template();
    let mut composition = commits(&flat(&[("nucleic_acid|other", json!("DNA"))]), &wt);
    composition["content"][0]["data"]["events"][0]["data"]["items"][0]["value"] =
        json!({"_type": "DV_COUNT", "magnitude": 3});
    let findings = validate_composition(&composition, &wt);
    let wrong: Vec<&ValidationMessage> = findings
        .iter()
        .filter(|m| m.kind == ValidationKind::WrongType)
        .collect();
    let [only] = wrong.as_slice() else {
        panic!("one wrong-type finding: {findings:#?}");
    };
    assert!(only.path.ends_with("/items[at0004]/value"), "{only:?}");
    assert!(
        only.message.contains("DV_CODED_TEXT") && only.message.contains("DV_TEXT"),
        "the refusal names both alternatives: {only:?}"
    );
}

#[test]
fn a_quantity_or_a_count_commits() {
    let wt = web_template();
    commits(
        &flat(&[
            ("amount/quantity_value|magnitude", json!(2.5)),
            ("amount/quantity_value|unit", json!("mg")),
        ]),
        &wt,
    );
    let composition = commits(&flat(&[("amount/count_value", json!(4))]), &wt);
    assert_eq!(
        element_value(&composition, "at0005")["_type"],
        json!("DV_COUNT")
    );
}

#[test]
fn a_date_or_a_date_time_commits() {
    let wt = web_template();
    let date = commits(&flat(&[("recorded/date_value", json!("2026-10-27"))]), &wt);
    assert_eq!(element_value(&date, "at0006")["_type"], json!("DV_DATE"));
    let date_time = commits(
        &flat(&[("recorded/date_time_value", json!("2026-10-27T10:00:00Z"))]),
        &wt,
    );
    assert_eq!(
        element_value(&date_time, "at0006")["_type"],
        json!("DV_DATE_TIME")
    );
}

#[test]
fn a_speed_matching_the_second_quantity_alternative_commits() {
    let wt = web_template();
    let composition = commits(
        &flat(&[
            ("speed/quantity_value_1|magnitude", json!(80.0)),
            ("speed/quantity_value_1|unit", json!("km/h")),
        ]),
        &wt,
    );
    let back = composition_to_flat(&composition, &wt).expect("FLAT emits");
    let speed: Vec<&String> = back.keys().filter(|k| k.contains("/speed/")).collect();
    assert_eq!(speed.len(), 2, "the value is emitted once: {speed:?}");
    let rebuilt = commits(&back, &wt);
    assert_eq!(
        element_value(&rebuilt, "at0007"),
        element_value(&composition, "at0007")
    );
}

#[test]
fn a_speed_matching_no_quantity_alternative_names_both() {
    let wt = web_template();
    // 80 [mi_i]/h is over the 55 of the first alternative and in the wrong
    // unit for the second.
    let findings = findings_of(
        &flat(&[
            ("speed/quantity_value|magnitude", json!(80.0)),
            ("speed/quantity_value|unit", json!("[mi_i]/h")),
        ]),
        &wt,
    );
    let summary = findings
        .iter()
        .find(|m| m.message.contains("matches none of the alternatives"))
        .unwrap_or_else(|| panic!("a summary finding: {findings:#?}"));
    assert!(
        summary.message.contains("[DV_QUANTITY, DV_QUANTITY]"),
        "{summary:?}"
    );
    assert!(
        findings
            .iter()
            .filter(|m| m.message.starts_with("alternative DV_QUANTITY: "))
            .count()
            >= 2,
        "each alternative's finding is kept: {findings:#?}"
    );
}

#[test]
fn the_examples_commit() {
    let wt = web_template();
    for level in [
        DetailLevel::Required,
        DetailLevel::Medium,
        DetailLevel::Complete,
    ] {
        let example = example_composition(&wt, level).expect("the example builds");
        let findings = validate_composition(&example, &wt);
        assert!(findings.is_empty(), "{level:?}: {findings:#?}");
    }
}
