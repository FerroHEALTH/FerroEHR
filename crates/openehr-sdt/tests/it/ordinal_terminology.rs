// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![expect(clippy::panic, reason = "test assertions/diagnostics/fixtures")]
//! An ordinal symbol matches a constrained entry only by its whole code phrase
//! (#5914).
//!
//! The symbol is a `DV_CODED_TEXT` whose `defining_code` is a `CODE_PHRASE`,
//! whose `code_string` means something only inside its `terminology_id` (RM
//! `data_types` §`CODE_PHRASE`), so `external::at0003` is not the `local::at0003`
//! the template's `C_DV_ORDINAL` lists (AOM 1.4 §ORDINAL).

use std::path::Path;

use openehr_its::opt14;
use openehr_sdt::flat::example::{DetailLevel, example_composition};
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::WebTemplate;
use openehr_sdt::rm_instance::validate_composition;
use serde_json::Value;

fn minimal_admin() -> WebTemplate {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/fixtures/opt/valid/minimal_admin.opt");
    let xml = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read: {e}"));
    let opt = opt14::from_xml(&xml).unwrap_or_else(|e| panic!("parse: {e}"));
    build_web_template(&opt).unwrap_or_else(|e| panic!("build: {e}"))
}

/// The first `DV_ORDINAL` of `v`, mutably.
fn ordinal(v: &mut Value) -> Option<&mut Value> {
    if v.get("_type").and_then(Value::as_str) == Some("DV_ORDINAL") {
        return Some(v);
    }
    match v {
        Value::Object(m) => m.values_mut().find_map(ordinal),
        Value::Array(a) => a.iter_mut().find_map(ordinal),
        _ => None,
    }
}

/// The ordinal findings of `comp`.
fn ordinal_findings(comp: &Value, wt: &WebTemplate) -> Vec<String> {
    validate_composition(comp, wt)
        .into_iter()
        .filter(|m| m.message.contains("ordinal"))
        .map(|m| format!("{}: {}", m.path, m.message))
        .collect()
}

#[test]
fn an_ordinal_symbol_of_another_terminology_is_refused() {
    let wt = minimal_admin();
    let comp = example_composition(&wt, DetailLevel::Medium).expect("the example builds");
    assert!(
        ordinal_findings(&comp, &wt).is_empty(),
        "the listed local symbol validates"
    );
    for terminology in ["external", "SNOMED-CT"] {
        let mut edited = comp.clone();
        let symbol = ordinal(&mut edited).expect("the example carries a DV_ORDINAL");
        assert_eq!(
            symbol.pointer("/symbol/defining_code/terminology_id/value"),
            Some(&Value::from("local")),
            "the template lists local codes"
        );
        symbol["symbol"]["defining_code"]["terminology_id"]["value"] = Value::from(terminology);
        let findings = ordinal_findings(&edited, &wt);
        assert!(
            findings
                .iter()
                .any(|f| f.contains(&format!("'{terminology}::"))
                    && f.contains("not in the constrained value set")),
            "{terminology}: refused naming the terminology, got {findings:?}"
        );
    }
}
