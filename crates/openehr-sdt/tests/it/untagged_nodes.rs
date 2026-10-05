// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Untagged canonical-JSON nodes run the full validation walk.
//!
//! Canonical JSON requires `_type` only on polymorphic slots (the ITS-JSON
//! schema declares `COMPOSITION.context` as a bare `$ref` to the concrete
//! `EVENT_CONTEXT`, with `_type` not among its `required` members), so a node
//! under a concretely-declared attribute may legally omit its tag. A walk that
//! dispatches on the wire tag alone therefore silently skips every RM
//! invariant on such nodes — the escape the CNF red row
//! `create_composition-setting_invalid` exposed (adjudicated to the
//! application per `.claude/rules/cnf-triage.md`; RM ehr `EVENT_CONTEXT`
//! §Invariants `Setting_valid`). These tests pin the fix: the effective RM
//! type of an untagged node is its parent's declared attribute type
//! (`openehr_rm::v1_2::model::declared_concrete_type`), and validation
//! verdicts are tag-presence-independent wherever the tag is omittable.

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

/// The red-row regression: an out-of-group `EVENT_CONTEXT.setting` on an
/// UNTAGGED `context` node is a terminology violation (RM ehr `EVENT_CONTEXT`
/// §Invariants `Setting_valid`; TERM `SupportTerminology` §Vocabularies
/// `setting`) — and the valid twin stays clean.
#[test]
fn untagged_context_setting_is_enforced() {
    let bad = composition_with_untagged_context("999");
    let violations = validate_rm_and_terminology(&bad);
    assert!(
        violations
            .iter()
            .any(|m| m.kind == ValidationKind::Terminology && m.path.contains("context/setting")),
        "an out-of-group setting on an untagged EVENT_CONTEXT must be a \
         terminology violation, got: {violations:?}"
    );

    let good = composition_with_untagged_context("228");
    let violations = validate_rm_and_terminology(&good);
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
    let violations = validate_rm_and_terminology(&comp);
    assert!(
        violations
            .iter()
            .any(|m| m.kind == ValidationKind::Invariant && m.path.contains("context")),
        "an empty location on an untagged EVENT_CONTEXT must violate a core \
         invariant, got: {violations:?}"
    );
}

/// Strip every LEGALLY-omittable `_type` from a node tree: a tag is removed
/// exactly where the parent attribute's declared RM type is concrete AND names
/// the same class the tag carries (omission elsewhere would change the
/// instance's meaning, so those tags stay).
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
        // `shift_remove`, never `remove`: with `serde_json/preserve_order` the
        // plain `remove` is a swap-remove that moves the LAST key into the
        // removed slot
        // (<https://docs.rs/serde_json/1/serde_json/struct.Map.html#method.remove>),
        // which would reorder the object and make this test compare two
        // different traversal orders instead of two verdict lists.
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

/// **The tag-independence property over the whole valid corpus:** stripping
/// every legally-omittable `_type` from a valid COMPOSITION changes NO
/// validation verdict — the effective-type resolution makes the walk
/// tag-presence-independent, so a format or client that omits optional tags
/// gets the same conformance answer as one that writes them all.
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
        let tagged: Vec<String> = validate_rm_and_terminology(&doc)
            .into_iter()
            .map(|m| format!("{}|{:?}|{}", m.path, m.kind, m.message))
            .collect();
        let mut stripped = doc.clone();
        strip_omittable_tags(&mut stripped, Some("COMPOSITION"));
        let untagged: Vec<String> = validate_rm_and_terminology(&stripped)
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
