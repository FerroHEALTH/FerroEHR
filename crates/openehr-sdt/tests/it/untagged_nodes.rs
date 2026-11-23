// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Untagged canonical-JSON nodes run the full validation walk: an untagged
//! node's type is its parent attribute's declared type
//! (`openehr_rm::v1_2::model::declared_concrete_type`), so verdicts do not depend
//! on an omittable `_type`.

use openehr_rm::v1_2::model::declared_concrete_type;
use openehr_sdt::rm_instance::{ValidationKind, validate_rm_and_terminology};
use serde_json::{Value, json};

use crate::common::corpus_files;

/// A minimal event COMPOSITION whose `context` node carries NO `_type` tag —
/// legal canonical JSON (`EVENT_CONTEXT` is the concretely-declared type of
/// `COMPOSITION.context`).
fn composition_with_untagged_context(setting_code: &str) -> Value {
    json!({
        "_type": "COMPOSITION",
        "name": {"_type": "DV_TEXT", "value": "Minimal"},
        "archetype_node_id": "openEHR-EHR-COMPOSITION.minimal.v1",
        "language": {"terminology_id": {"value": "ISO_639-1"}, "code_string": "en"},
        "territory": {"terminology_id": {"value": "ISO_3166-1"}, "code_string": "UY"},
        "category": {"value": "event", "defining_code": {
            "terminology_id": {"value": "openehr"}, "code_string": "433"}},
        "composer": {"_type": "PARTY_IDENTIFIED", "name": "Dr. House"},
        "context": {
            // deliberately untagged
            "start_time": {"value": "2021-09-21T21:52:31.927-03:00"},
            "setting": {"value": "primary medical care", "defining_code": {
                "terminology_id": {"value": "openehr"}, "code_string": setting_code}},
        },
        "content": [],
    })
}

/// An out-of-group setting on an untagged `context` is a terminology violation
/// (RM ehr `EVENT_CONTEXT` `Setting_valid`); the valid twin is clean.
#[test]
fn untagged_context_setting_is_enforced() {
    let bad = composition_with_untagged_context("999");
    let violations = validate_rm_and_terminology(&bad, "COMPOSITION");
    assert!(
        violations
            .iter()
            .any(|m| m.kind == ValidationKind::Terminology && m.path.contains("context/setting")),
        "an out-of-group setting on an untagged EVENT_CONTEXT must be a \
         terminology violation, got: {violations:?}"
    );

    let good = composition_with_untagged_context("228");
    let violations = validate_rm_and_terminology(&good, "COMPOSITION");
    assert!(
        !violations
            .iter()
            .any(|m| m.path.contains("context/setting")),
        "the valid twin (setting 228) must raise nothing on the setting slot, \
         got: {violations:?}"
    );
}

/// Core (non-terminology) invariants reach untagged nodes too: an empty
/// `EVENT_CONTEXT.location` violates `Location_valid` (RM ehr `EVENT_CONTEXT`
/// §Invariants) whether or not the context node is tagged.
#[test]
fn untagged_context_runs_core_invariants() {
    let mut comp = composition_with_untagged_context("228");
    comp["context"]["location"] = json!("");
    let violations = validate_rm_and_terminology(&comp, "COMPOSITION");
    assert!(
        violations
            .iter()
            .any(|m| m.kind == ValidationKind::Invariant && m.path.contains("context")),
        "an empty location on an untagged EVENT_CONTEXT must violate a core \
         invariant, got: {violations:?}"
    );
}

/// Strip every legally omittable `_type`: where the parent's declared type is
/// concrete and names the tag's class.
fn strip_omittable_tags(v: &mut Value, declared: Option<&str>) {
    let Some(obj) = v.as_object_mut() else { return };
    let this_type: Option<String> = obj
        .get("_type")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| declared.map(str::to_owned));
    if let (Some(tag), Some(decl)) = (obj.get("_type").and_then(Value::as_str), declared)
        && tag == decl
    {
        // `shift_remove` keeps key order under `serde_json/preserve_order`
        // (<https://docs.rs/serde_json/1/serde_json/struct.Map.html#method.remove>).
        obj.shift_remove("_type");
    }
    let Some(this_type) = this_type else { return };
    let keys: Vec<String> = obj
        .keys()
        .filter(|k| !k.starts_with('_'))
        .cloned()
        .collect();
    for k in keys {
        let child_declared = declared_concrete_type(&this_type, &k);
        match obj.get_mut(&k) {
            Some(Value::Array(items)) => {
                for item in items {
                    strip_omittable_tags(item, child_declared);
                }
            }
            Some(child @ Value::Object(_)) => strip_omittable_tags(child, child_declared),
            _ => {}
        }
    }
}

/// Stripping every omittable `_type` from a valid COMPOSITION changes no
/// verdict.
#[test]
fn corpus_verdicts_are_tag_presence_independent() {
    let mut checked = 0usize;
    for path in corpus_files() {
        let text = std::fs::read_to_string(&path).expect("read corpus file");
        let Ok(doc) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if doc.get("_type").and_then(Value::as_str) != Some("COMPOSITION") {
            continue;
        }
        let tagged: Vec<String> = validate_rm_and_terminology(&doc, "COMPOSITION")
            .into_iter()
            .map(|m| format!("{}|{:?}|{}", m.path, m.kind, m.message))
            .collect();
        let mut stripped = doc.clone();
        strip_omittable_tags(&mut stripped, Some("COMPOSITION"));
        let untagged: Vec<String> = validate_rm_and_terminology(&stripped, "COMPOSITION")
            .into_iter()
            .map(|m| format!("{}|{:?}|{}", m.path, m.kind, m.message))
            .collect();
        assert_eq!(
            tagged,
            untagged,
            "verdicts changed after stripping omittable tags in {}",
            path.display()
        );
        checked += 1;
    }
    assert!(
        checked > 20,
        "expected a real corpus, checked {checked} compositions"
    );
}
