// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The RM-instance passes over typed RM report the same multiset as the
//! canonical-JSON passes, on every COMPOSITION and defective variants.

use openehr_rm::prelude::Composition;
use openehr_sdt::rm_instance::{
    ValidationMessage, validate_rm_and_terminology, validate_rm_and_terminology_typed,
};
use serde_json::Value;

fn compositions() -> Vec<(String, Value)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = Vec::new();
    let mut stack: Vec<std::path::PathBuf> = ["corpus", "crates", "docs/specs/openehr/CNF"]
        .iter()
        .map(|d| root.join(d))
        .collect();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n != "target") {
                    stack.push(path);
                }
                continue;
            }
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if !text.contains("\"COMPOSITION\"") {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if value.get("_type").and_then(Value::as_str) == Some("COMPOSITION") {
                out.push((path.display().to_string(), value));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Defective variants: a coded slot out of its group and its code set, an
/// empty `archetype_node_id`, and `archetype_details` on a non-root node.
fn variants(doc: &Value) -> Vec<(&'static str, Value)> {
    let mut out = vec![("as committed", doc.clone())];
    let mut v = doc.clone();
    if let Some(code) = v.pointer_mut("/category/defining_code/code_string") {
        *code = Value::from("999");
        out.push(("category out of its group", v));
    }
    let mut v = doc.clone();
    if let Some(code) = v.pointer_mut("/language/code_string") {
        *code = Value::from("zz-not-a-language");
        out.push(("language out of its code set", v));
    }
    let mut v = doc.clone();
    if let Some(id) = v.pointer_mut("/content/0/archetype_node_id") {
        *id = Value::from("");
        out.push(("empty archetype_node_id", v));
    }
    let mut v = doc.clone();
    let details = doc.get("archetype_details").cloned();
    if let (Some(name), Some(details)) = (v.pointer_mut("/content/0/name"), details) {
        let _ = name;
        if let Some(Value::Object(first)) = v.pointer_mut("/content/0") {
            first.insert("archetype_node_id".into(), Value::from("at0000"));
            first.insert("archetype_details".into(), details);
            out.push(("archetype_details on a term-code node", v));
        }
    }
    out
}

fn sorted(mut v: Vec<ValidationMessage>) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = v
        .drain(..)
        .map(|m| (m.path, m.message, format!("{:?}", m.kind)))
        .collect();
    out.sort();
    out
}

#[test]
fn typed_passes_report_what_the_json_passes_report() {
    let docs = compositions();
    assert!(
        docs.len() > 100,
        "the corpus should hold many compositions, found {}",
        docs.len()
    );
    let mut compared = 0;
    let mut with_findings = 0;
    let mut mismatches = Vec::new();
    for (path, doc) in &docs {
        for (label, variant) in variants(doc) {
            // A document the reader refuses never reaches validation.
            let Ok(typed) = openehr_its::json::from_canonical_value::<Composition>(&variant) else {
                continue;
            };
            let json_report = validate_rm_and_terminology(&variant, "COMPOSITION");
            let typed_report = validate_rm_and_terminology_typed(&typed);
            compared += 1;
            if !json_report.is_empty() {
                with_findings += 1;
            }
            if sorted(json_report.clone()) != sorted(typed_report.clone()) {
                mismatches.push(format!(
                    "{path} ({label}):\n  json:  {json_report:?}\n  typed: {typed_report:?}"
                ));
            }
        }
    }
    assert!(compared > 300, "compared only {compared} documents");
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
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
