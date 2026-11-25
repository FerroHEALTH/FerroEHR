// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![expect(clippy::panic, reason = "test assertions/diagnostics/fixtures")]
//! A coded `LOCATABLE.name` keeps to the code list the template constrains it
//! to (RM common `master03-archetyped_package.adoc` §"The `LOCATABLE` class";
//! AOM 1.4 §`C_CODED_TEXT`): the CKM `exclusion_global` element's name is one
//! of `local::at0003` to `at0007`.

use std::path::Path;

use openehr_its::opt14;
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::WebTemplate;
use openehr_sdt::rm_instance::validate_composition;
use serde_json::Value;

fn ips(file: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/templates/ckm")
        .join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn web_template() -> WebTemplate {
    let opt = opt14::from_xml(&ips("international-patient-summary.opt"))
        .unwrap_or_else(|e| panic!("parse: {e}"));
    build_web_template(&opt).unwrap_or_else(|e| panic!("build: {e}"))
}

/// The example composition with the first exclusion statement's name coded
/// `code`.
fn with_exclusion_name(code: &str) -> Value {
    let mut comp: Value = serde_json::from_str(&ips("international-patient-summary.example.json"))
        .unwrap_or_else(|e| panic!("example: {e}"));
    let name = comp
        .pointer_mut("/content/0/items/1/data/items/0/name/defining_code/code_string")
        .unwrap_or_else(|| panic!("the example's first exclusion statement has a coded name"));
    *name = Value::from(code);
    comp
}

fn name_findings(comp: &Value, wt: &WebTemplate) -> Vec<String> {
    validate_composition(comp, wt)
        .into_iter()
        .filter(|m| m.path.ends_with("/name"))
        .map(|m| format!("{}: {}", m.path, m.message))
        .collect()
}

#[test]
fn a_coded_name_outside_its_code_list_is_refused() {
    let wt = web_template();
    assert_eq!(
        name_findings(&with_exclusion_name("at0005"), &wt),
        Vec::<String>::new(),
        "a listed code validates"
    );
    let findings = name_findings(&with_exclusion_name("at0099"), &wt);
    assert!(
        findings
            .iter()
            .any(|f| f.contains("'at0099'") && f.contains("not in the constrained value set")),
        "the unlisted code is refused at the name: {findings:?}"
    );
}
