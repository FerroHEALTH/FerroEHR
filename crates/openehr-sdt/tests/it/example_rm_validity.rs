// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Regression: generated examples must be RM-valid — an `ELEMENT.value`
//! carries a `DATA_VALUE`, never a structure (RM `data_structures`
//! `ELEMENT.value: DATA_VALUE [0..1]`). The CKM `CCTA report` OPT (vendored
//! in the CNF runner's journey template pack) exposed a generator defect
//! where a deep `protocol` subtree produced `ELEMENT.value` = `ITEM_TREE`,
//! which conformant validation rejects (422 against the platform's own
//! commit surface).

use openehr_its::opt14;
use openehr_sdt::flat::example::{DetailLevel, example_composition};
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use serde_json::Value;

/// The RM structure types that must never appear as an `ELEMENT.value`.
const STRUCTURES: [&str; 5] = [
    "ITEM_TREE",
    "ITEM_LIST",
    "ITEM_TABLE",
    "ITEM_SINGLE",
    "CLUSTER",
];

fn assert_elements_carry_data_values(node: &Value, path: &str, findings: &mut Vec<String>) {
    match node {
        Value::Object(map) => {
            if map.get("_type").and_then(Value::as_str) == Some("ELEMENT")
                && let Some(value) = map.get("value")
                && let Some(value_type) = value.get("_type").and_then(Value::as_str)
                && STRUCTURES.contains(&value_type)
            {
                findings.push(format!("{path}: ELEMENT.value is {value_type}"));
            }
            for (key, child) in map {
                assert_elements_carry_data_values(child, &format!("{path}/{key}"), findings);
            }
        }
        Value::Array(items) => {
            for (i, child) in items.iter().enumerate() {
                assert_elements_carry_data_values(child, &format!("{path}[{i}]"), findings);
            }
        }
        _ => {}
    }
}

/// Every template of the CNF runner's vendored CKM journey pack generates
/// RM-shape-valid examples at every detail level (the pack is the measured
/// workload's committed payload ground — an invalid example poisons the
/// instrument).
#[test]
fn ckm_pack_examples_are_rm_shape_valid() {
    let pack = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/templates/ckm");
    let mut checked = 0;
    let mut findings: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&pack).expect("pack dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().is_none_or(|e| e != "opt") {
            continue;
        }
        let xml = std::fs::read_to_string(&path).expect("read OPT");
        let opt = opt14::from_xml(&xml).expect("parse OPT");
        let wt = build_web_template(&opt).expect("build web template");
        for level in [
            DetailLevel::Required,
            DetailLevel::Medium,
            DetailLevel::Complete,
        ] {
            let example = example_composition(&wt, level);
            assert_elements_carry_data_values(
                &example,
                &format!(
                    "{}@{level:?}",
                    path.file_name()
                        .and_then(std::ffi::OsStr::to_str)
                        .unwrap_or("?")
                ),
                &mut findings,
            );
        }
        checked += 1;
    }
    assert!(checked >= 15, "expected the full pack, checked {checked}");
    assert!(
        findings.is_empty(),
        "RM-invalid ELEMENT.value in generated examples:\n{}",
        findings.join("\n")
    );
}
