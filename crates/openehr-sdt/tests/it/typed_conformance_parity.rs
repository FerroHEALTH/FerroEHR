// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The archetype-conformance pass over typed RM reports the same multiset as
//! the canonical-JSON pass, on every template example and defective variants.

use openehr_its::opt14;
use openehr_rm::prelude::Composition;
use openehr_sdt::flat::example::{DetailLevel, example_composition};
use openehr_sdt::flat::validation::{
    validate_archetype_conformance, validate_archetype_conformance_typed,
};
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::rm_instance::ValidationMessage;
use serde_json::Value;

fn opts() -> Vec<std::path::PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = Vec::new();
    let mut stack = vec![
        root.join("corpus/templates"),
        root.join("crates/openehr-sdt/tests/fixtures"),
    ];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "opt") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// Apply `f` to the first node (depth first) it accepts; whether one did.
fn mutate_first(v: &mut Value, f: &mut dyn FnMut(&mut Value) -> bool) -> bool {
    if f(v) {
        return true;
    }
    match v {
        Value::Object(o) => o.values_mut().any(|c| mutate_first(c, f)),
        Value::Array(a) => a.iter_mut().any(|c| mutate_first(c, f)),
        _ => false,
    }
}

fn variants(doc: &Value) -> Vec<(&'static str, Value)> {
    let mut out = vec![("as built", doc.clone())];
    let mut push = |label: &'static str, f: &mut dyn FnMut(&mut Value) -> bool| {
        let mut v = doc.clone();
        if mutate_first(&mut v, f) {
            out.push((label, v));
        }
    };
    push(
        "magnitude out of range",
        &mut |n| match n.get_mut("magnitude") {
            Some(m) if m.is_number() => {
                *m = Value::from(1.0e12);
                true
            }
            _ => false,
        },
    );
    push(
        "code outside its list",
        &mut |n| match n.pointer_mut("/defining_code/code_string") {
            Some(c) => {
                *c = Value::from("zzz-not-a-code");
                true
            }
            None => false,
        },
    );
    push(
        "unexpected node",
        &mut |n| match n.get_mut("items").and_then(Value::as_array_mut) {
            Some(items) => match items
                .first_mut()
                .and_then(|i| i.get_mut("archetype_node_id"))
            {
                Some(id) if id.as_str().is_some_and(|s| s.starts_with("at")) => {
                    *id = Value::from("at9999");
                    true
                }
                _ => false,
            },
            None => false,
        },
    );
    push(
        "a member left out",
        &mut |n| match n.get_mut("items").and_then(Value::as_array_mut) {
            Some(items) if items.len() > 1 => {
                items.remove(0);
                true
            }
            _ => false,
        },
    );
    push(
        "a member repeated",
        &mut |n| match n.get_mut("items").and_then(Value::as_array_mut) {
            Some(items) => match items.first().cloned() {
                Some(first) => {
                    items.push(first);
                    true
                }
                None => false,
            },
            None => false,
        },
    );
    out
}

fn sorted(v: Vec<ValidationMessage>) -> Vec<(String, String, String)> {
    let mut out: Vec<_> = v
        .into_iter()
        .map(|m| (m.path, m.message, format!("{:?}", m.kind)))
        .collect();
    out.sort();
    out
}

#[test]
fn typed_conformance_reports_what_the_json_pass_reports() {
    let mut compared = 0;
    let mut with_findings = 0;
    let mut mismatches = Vec::new();
    for path in opts() {
        let Ok(xml) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(opt) = opt14::from_xml(&xml) else {
            continue;
        };
        let Ok(wt) = build_web_template(&opt) else {
            continue;
        };
        for level in [DetailLevel::Required, DetailLevel::Complete] {
            let Ok(example) = example_composition(&wt, level) else {
                continue;
            };
            for (label, variant) in variants(&example) {
                let Ok(typed) = openehr_its::json::from_canonical_value::<Composition>(&variant)
                else {
                    continue;
                };
                let canonical = openehr_its::json::to_canonical_value(&typed);
                let json_report = validate_archetype_conformance(&canonical, &wt);
                let typed_report = validate_archetype_conformance_typed(&typed, &wt);
                compared += 1;
                if !json_report.is_empty() {
                    with_findings += 1;
                }
                if sorted(json_report.clone()) != sorted(typed_report.clone()) {
                    mismatches.push(format!(
                        "{} {level:?} ({label}):\n  json:  {json_report:?}\n  typed: {typed_report:?}",
                        path.display()
                    ));
                }
            }
        }
    }
    assert!(compared > 500, "compared only {compared} documents");
    assert!(
        with_findings > 100,
        "only {with_findings} documents carried a finding"
    );
    assert!(
        mismatches.is_empty(),
        "{} of {compared} reports differ:\n{}",
        mismatches.len(),
        mismatches
            .iter()
            .take(4)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
