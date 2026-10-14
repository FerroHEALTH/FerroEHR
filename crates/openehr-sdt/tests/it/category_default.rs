// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `COMPOSITION.category` a FLAT or TDD build supplies when the input names
//! none comes from the template's constraint on `/category`: its assumed
//! value or single admitted code, `433` (event) when the template leaves it
//! unconstrained, and a refusal naming the path when it admits several codes
//! and fixes no default.
//! `category` is mandatory (RM `UML/classes/org.openehr.rm.composition.composition.adoc`
//! §Attributes, `1..1`), so the supplied value must satisfy the template.
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_its::opt14;
use openehr_sdt::flat::convert::composition_from_flat;
use openehr_sdt::flat::error::FlatError;
use openehr_sdt::flat::tdd::from_tdd;
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::WebTemplate;
use serde_json::{Map, Value};

const CORPUS: &str = "../../docs/specs/openehr/CNF/tests/platform/robot/_resources/test_data_sets";
const NOW: &str = "2026-01-01T00:00:00Z";

/// The persistent-only template (`category` code list `[431]`) with its
/// category code list replaced by `codes` (empty: unconstrained) and, when
/// given, an `assumed_value` on the category's `C_CODE_PHRASE`.
fn web_template(codes: &[&str], assumed: Option<&str>) -> WebTemplate {
    let path = format!("{CORPUS}/valid_templates/minimal_persistent/persistent_minimal.opt");
    let xml = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let original = "<code_list>431</code_list>";
    assert_eq!(
        xml.matches(original).count(),
        1,
        "the fixture constrains category once"
    );
    let at = xml.find(original).expect("category code list");
    // The OPT XSD orders `C_CODE_PHRASE` as `assumed_value`, `terminology_id`,
    // `code_list`, so the assumed value goes before the terminology id.
    let terminology = xml
        .get(..at)
        .and_then(|head| head.rfind("<terminology_id>"))
        .expect("category terminology id");
    let replacement = codes.iter().fold(String::new(), |mut out, code| {
        out.push_str("<code_list>");
        out.push_str(code);
        out.push_str("</code_list>");
        out
    });
    let mut edited = xml.get(..terminology).expect("head").to_owned();
    if let Some(code) = assumed {
        edited.push_str(
            "<assumed_value><terminology_id><value>openehr</value></terminology_id>\
             <code_string>",
        );
        edited.push_str(code);
        edited.push_str("</code_string></assumed_value>");
    }
    edited.push_str(xml.get(terminology..at).expect("terminology id"));
    edited.push_str(&replacement);
    edited.push_str(xml.get(at + original.len()..).expect("tail"));
    let xml = edited;
    build_web_template(&opt14::from_xml(&xml).expect("parse OPT")).expect("build WebTemplate")
}

/// The CNF FLAT instance of the template.
fn flat_document() -> Map<String, Value> {
    let path = format!("{CORPUS}/compositions/FLAT/persistent_minimal.en.v1__full.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    serde_json::from_str(&text).expect("FLAT JSON")
}

/// The CNF FLAT instance of the template, without its category.
fn flat_without_category() -> Map<String, Value> {
    let mut doc = flat_document();
    doc.retain(|key, _| !key.starts_with("persistent_minimal/category"));
    doc
}

/// The CNF TDD instance of the template, without its category element.
fn tdd_without_category() -> String {
    let path = format!("{CORPUS}/compositions/TDD/persistent_minimal.en.v1__full.xml");
    let xml = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let start = xml.find("<category>").expect("the TDD carries a category");
    let end = xml.find("</category>").expect("closing category") + "</category>".len();
    let mut out = xml.get(..start).expect("prefix").to_owned();
    out.push_str(xml.get(end..).expect("suffix"));
    out
}

/// The built category's code, terminology and rubric.
fn category_code(comp: &Value) -> (&str, &str, &str) {
    let text = |pointer: &str| {
        comp.pointer(pointer)
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{pointer} in {comp}"))
    };
    (
        text("/category/defining_code/code_string"),
        text("/category/defining_code/terminology_id/value"),
        text("/category/value"),
    )
}

#[test]
fn persistent_only_template_defaults_to_431() {
    let wt = web_template(&["431"], None);
    let flat = composition_from_flat(&flat_without_category(), &wt, NOW).expect("FLAT build");
    assert_eq!(category_code(&flat), ("431", "openehr", "persistent"));
    let tdd = from_tdd(&tdd_without_category(), &wt).expect("TDD build");
    assert_eq!(category_code(&tdd), ("431", "openehr", "persistent"));
}

#[test]
fn unconstrained_category_defaults_to_433() {
    let wt = web_template(&[], None);
    let flat = composition_from_flat(&flat_without_category(), &wt, NOW).expect("FLAT build");
    assert_eq!(category_code(&flat), ("433", "openehr", "event"));
    let tdd = from_tdd(&tdd_without_category(), &wt).expect("TDD build");
    assert_eq!(category_code(&tdd), ("433", "openehr", "event"));
}

#[test]
fn several_admitted_codes_without_default_refuse_naming_the_path() {
    let wt = web_template(&["431", "433"], None);
    match composition_from_flat(&flat_without_category(), &wt, NOW) {
        Err(FlatError::MissingRequiredSuffix { key }) => {
            assert_eq!(key, "persistent_minimal/category|code");
        }
        other => panic!("FLAT build must refuse a missing category, got {other:?}"),
    }
    match from_tdd(&tdd_without_category(), &wt) {
        Err(FlatError::Conversion(message)) => {
            assert!(message.contains("/category"), "{message}");
            assert!(message.contains("431, 433"), "{message}");
        }
        other => panic!("TDD build must refuse a missing category, got {other:?}"),
    }
}

#[test]
fn an_explicit_category_is_kept() {
    let wt = web_template(&["431", "433"], None);
    let flat = composition_from_flat(&flat_document(), &wt, NOW).expect("FLAT build");
    assert_eq!(category_code(&flat).0, "431");
}

#[test]
fn several_admitted_codes_take_the_assumed_value() {
    let wt = web_template(&["431", "433"], Some("433"));
    let flat = composition_from_flat(&flat_without_category(), &wt, NOW).expect("FLAT build");
    assert_eq!(category_code(&flat), ("433", "openehr", "event"));
    let tdd = from_tdd(&tdd_without_category(), &wt).expect("TDD build");
    assert_eq!(category_code(&tdd), ("433", "openehr", "event"));
}
