// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![expect(clippy::panic, reason = "test assertions/diagnostics/fixtures")]
//! A `C_STRING` on the RM String attribute `ACTIVITY.action_archetype_id` is
//! checked, and the example meets it (#5913).
//!
//! ADL 1.4 `master05-cadl.adoc` §Constraints on String: a string attribute can
//! be constrained by a regular expression; AOM 1.4 `C_STRING.valid_value`
//! tests the value against it. `minimal_instruction.opt` constrains
//! `action_archetype_id` to `openEHR-EHR-ACTION\.minimal\.v1`.

use std::path::Path;

use openehr_its::opt14;
use openehr_sdt::flat::example::{DetailLevel, example_composition};
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::WebTemplate;
use openehr_sdt::rm_instance::validate_composition;
use serde_json::Value;

fn minimal_instruction() -> WebTemplate {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/fixtures/opt/valid/minimal_instruction.opt");
    let xml = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read: {e}"));
    let opt = opt14::from_xml(&xml).unwrap_or_else(|e| panic!("parse: {e}"));
    build_web_template(&opt).unwrap_or_else(|e| panic!("build: {e}"))
}

/// The first `ACTIVITY` of `v`, mutably.
fn activity(v: &mut Value) -> Option<&mut Value> {
    if v.get("_type").and_then(Value::as_str) == Some("ACTIVITY") {
        return Some(v);
    }
    match v {
        Value::Object(m) => m.values_mut().find_map(activity),
        Value::Array(a) => a.iter_mut().find_map(activity),
        _ => None,
    }
}

fn pattern_findings(comp: &Value, wt: &WebTemplate) -> Vec<String> {
    validate_composition(comp, wt)
        .into_iter()
        .filter(|m| m.path.ends_with("/action_archetype_id"))
        .map(|m| format!("{}: {}", m.path, m.message))
        .collect()
}

#[test]
fn the_example_meets_the_pattern_and_another_value_is_refused() {
    let wt = minimal_instruction();
    let mut comp = example_composition(&wt, DetailLevel::Medium).expect("the example builds");
    let act = activity(&mut comp).expect("the example carries the ACTIVITY");
    assert_eq!(
        act.get("action_archetype_id"),
        Some(&Value::from("openEHR-EHR-ACTION.minimal.v1")),
        "the example writes a value the pattern admits"
    );
    assert!(
        pattern_findings(&comp, &wt).is_empty(),
        "the example validates"
    );
    for refused in ["/.*/", "openEHR-EHR-ACTION.other.v1", "not-an-archetype-id"] {
        let mut edited = comp.clone();
        let act = activity(&mut edited).expect("the ACTIVITY");
        act["action_archetype_id"] = Value::from(refused);
        let findings = pattern_findings(&edited, &wt);
        assert!(
            findings
                .iter()
                .any(|f| f.contains("does not match pattern")),
            "{refused}: refused, got {findings:?}"
        );
    }
}
