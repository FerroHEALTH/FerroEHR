// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![expect(clippy::expect_used, reason = "test assertions/diagnostics/fixtures")]
//! Verdict invariance across the legal canonical-JSON spelling freedoms.
//!
//! Deleting a present empty collection from a valid document changes no
//! verdict, and neither does writing a whole Real as an integer token. Inserting
//! `[]` may, since invariants such as `Participations_validity` refuse it.

use openehr_sdt::rm_instance::validate_rm_and_terminology;
use serde_json::{Number, Value};

use crate::common::corpus_files;

fn verdicts(doc: &Value) -> Vec<String> {
    let mut out: Vec<String> = validate_rm_and_terminology(doc, "COMPOSITION")
        .into_iter()
        .map(|m| format!("{}|{:?}|{}", m.path, m.kind, m.message))
        .collect();
    out.sort();
    out
}

/// Every valid corpus COMPOSITION, with each spelling transform applied,
/// yields the same verdict set as the original.
fn corpus_invariance(label: &str, transform: impl Fn(&mut Value)) {
    let mut checked = 0usize;
    for path in corpus_files() {
        let text = std::fs::read_to_string(&path).expect("read corpus file");
        let Ok(doc) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if doc.get("_type").and_then(Value::as_str) != Some("COMPOSITION") {
            continue;
        }
        let baseline = verdicts(&doc);
        let mut respelled = doc.clone();
        transform(&mut respelled);
        assert_eq!(
            baseline,
            verdicts(&respelled),
            "{label}: verdicts changed for {}",
            path.display()
        );
        checked += 1;
    }
    assert!(
        checked > 20,
        "{label}: expected a real corpus, checked {checked}"
    );
}

/// Remove every present-but-EMPTY array member, recursively.
fn drop_empty_collections(v: &mut Value) {
    match v {
        Value::Object(obj) => {
            obj.retain(|_, child| !matches!(child, Value::Array(a) if a.is_empty()));
            for child in obj.values_mut() {
                drop_empty_collections(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                drop_empty_collections(item);
            }
        }
        _ => {}
    }
}

/// Re-spell every whole-valued float number as an integer token (`78.0` →
/// `78`), recursively — the tolerant-read twin of the canonical output.
fn integerize_whole_reals(v: &mut Value) {
    match v {
        Value::Object(obj) => {
            for child in obj.values_mut() {
                integerize_whole_reals(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                integerize_whole_reals(item);
            }
        }
        Value::Number(n) => {
            if let Some(f) = n.as_f64()
                && n.as_i64().is_none()
                && n.as_u64().is_none()
                && f.fract() == 0.0
                && f.is_finite()
                && f.abs() < 9.007_199_254_740_992e15
            {
                // Exactly representable both ways (inside 2^53).
                #[expect(
                    clippy::as_conversions,
                    clippy::cast_possible_truncation,
                    reason = "fract()==0 and |f| < 2^53 are checked on the lines above, so the cast is exact"
                )]
                let i = f as i64;
                *v = Value::Number(Number::from(i));
            }
        }
        _ => {}
    }
}

#[test]
fn omitting_empty_collections_changes_no_verdict() {
    corpus_invariance("drop-empty-collections", drop_empty_collections);
}

#[test]
fn integer_spelling_of_whole_reals_changes_no_verdict() {
    corpus_invariance("integerize-whole-reals", integerize_whole_reals);
}

/// A present empty `EVENT_CONTEXT.participations` is a violation, the omitted
/// spelling clean (RM ehr §`EVENT_CONTEXT` `Participations_validity`).
#[test]
fn present_but_empty_participations_is_a_violation_and_omitted_is_not() {
    let mut doc = serde_json::json!({
        "_type": "COMPOSITION",
        "name": {"_type": "DV_TEXT", "value": "Minimal"},
        "archetype_node_id": "openEHR-EHR-COMPOSITION.minimal.v1",
        "language": {"terminology_id": {"value": "ISO_639-1"}, "code_string": "en"},
        "territory": {"terminology_id": {"value": "ISO_3166-1"}, "code_string": "UY"},
        "category": {"value": "event", "defining_code": {
            "terminology_id": {"value": "openehr"}, "code_string": "433"}},
        "composer": {"_type": "PARTY_IDENTIFIED", "name": "Dr. House"},
        "context": {
            "start_time": {"value": "2021-09-21T21:52:31.927-03:00"},
            "setting": {"value": "primary medical care", "defining_code": {
                "terminology_id": {"value": "openehr"}, "code_string": "228"}},
        },
        "content": [],
    });
    let clean = verdicts(&doc);
    assert!(
        !clean.iter().any(|m| m.contains("participations")),
        "omitted participations must be clean, got: {clean:?}"
    );
    doc["context"]["participations"] = serde_json::json!([]);
    let dirty = verdicts(&doc);
    assert!(
        dirty
            .iter()
            .any(|m| m.contains("participations") && m.contains("at least one member")),
        "present-but-empty participations must violate \
         Participations_validity, got: {dirty:?}"
    );
}
