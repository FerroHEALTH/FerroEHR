// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions/diagnostics/fixtures"
)]
//! The **wire-boundary RM class-invariant dispatcher** gate
//! (`openehr_its::wire_validate`).
//!
//! Both validation tiers are RM model semantics and live in `openehr-rm`: the
//! fast path (`openehr_rm::v1_2::validate::try_fast_validate`, untyped) and the typed
//! dispatch (`openehr_rm::v1_2::validate::typed_dispatch::dispatch_typed`, which
//! decodes through the emitted canonical-JSON `serde` impls). What this crate
//! keeps — and why the gate runs HERE — is the composition: the GENERATED
//! five-crate structural fallthrough
//! (`openehr_its::json_codec::generated::structural`, which spans every spec
//! crate at once and can only be emitted downstream of all of them) and the thin
//! wire entry points that fix the order the tiers and the orthogonal layers run
//! in. The assertions are unchanged: the two-tier entry point must produce
//! byte-identical violations to the authoritative typed oracle over the whole
//! corpus, and the fast path must vouch only when it is provably equivalent
//! (declining to the typed path otherwise).

use openehr_base::validate::InvariantViolation;
use openehr_its::wire_validate::{
    validate_rm_invariants, validate_rm_value, validate_rm_value_typed,
};
use openehr_rm::v1_2::validate::terminology::validate_rm_terminology;
use openehr_rm::v1_2::validate::try_fast_validate;
use serde_json::{Value, json};

/// Run only the typed (fallback) dispatch — the oracle.
fn typed(value: &Value) -> Vec<InvariantViolation> {
    let mut out = Vec::new();
    if let Some(ty) = value.get("_type").and_then(Value::as_str) {
        validate_rm_value_typed(ty, value, &mut out);
    }
    out
}

/// Run the two-tier **core** (fast + typed) entry point — the tier the
/// fast-vs-typed equivalence property pins. The terminology-backed layer
/// ([`validate_rm_terminology`]) is orthogonal and is exercised separately, so
/// the equivalence assertions here compare core against core.
fn two_tier(value: &Value) -> Vec<InvariantViolation> {
    let mut out = Vec::new();
    validate_rm_invariants(value, &mut out);
    out
}

/// Run the full unified public entry point (core + terminology).
fn full(value: &Value) -> Vec<InvariantViolation> {
    let mut out = Vec::new();
    validate_rm_value(value, &mut out);
    out
}

/// Whether the fast path handled the node (nothing appended on `false`).
fn fast_handled(value: &Value) -> bool {
    let Some(ty) = value.get("_type").and_then(Value::as_str) else {
        return false;
    };
    let mut out = Vec::new();
    try_fast_validate(ty, value, &mut out)
}

// ── dispatch smoke tests (moved from openehr-rm's validate.rs) ────────────────

#[test]
fn dispatch_unknown_or_untyped_is_noop() {
    let mut out = Vec::new();
    validate_rm_value(&json!({"value": "x"}), &mut out);
    validate_rm_value(&json!({"_type": "NOT_A_REAL_TYPE"}), &mut out);
    assert!(out.is_empty());
}

#[test]
fn dispatch_code_phrase_invalid() {
    // CODE_PHRASE with an empty code_string violates Code_string_valid.
    let node = json!({
        "_type": "CODE_PHRASE",
        "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "local"},
        "code_string": ""
    });
    let mut out = Vec::new();
    validate_rm_value(&node, &mut out);
    assert!(
        out.iter()
            .any(|v| v.message == "Invariant Code_string_valid failed on type CODE_PHRASE"),
        "got {out:?}"
    );
}

#[test]
fn dispatch_dv_proportion_valid_and_invalid() {
    let valid = json!({
        "_type": "DV_PROPORTION", "numerator": 1.0, "denominator": 100.0, "type": 2
    });
    let mut out = Vec::new();
    validate_rm_value(&valid, &mut out);
    assert!(
        out.is_empty(),
        "expected valid percent proportion, got {out:?}"
    );

    // percent kind (2) requires denominator == 100.
    let invalid = json!({
        "_type": "DV_PROPORTION", "numerator": 1.0, "denominator": 3.0, "type": 2
    });
    let mut out = Vec::new();
    validate_rm_value(&invalid, &mut out);
    assert!(
        out.iter()
            .any(|v| v.message == "Invariant Percent_validity failed on type DV_PROPORTION"),
        "got {out:?}"
    );
}

// ── fast/typed equivalence (moved from openehr-rm's validate/fast.rs) ─────────

#[test]
fn fast_path_matches_typed_on_simple_nodes() {
    let cases = [
        json!({"_type": "CODE_PHRASE",
               "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "openehr"},
               "code_string": "433"}),
        json!({"_type": "CODE_PHRASE",
               "terminology_id": {"value": "openehr"}, "code_string": ""}),
        json!({"_type": "DV_TEXT", "value": "hello"}),
        json!({"_type": "DV_TEXT", "value": "", "formatting": ""}),
        json!({"_type": "DV_CODED_TEXT", "value": "event",
               "defining_code": {"terminology_id": {"value": "openehr"},
                                 "code_string": "433"}}),
        json!({"_type": "DV_QUANTITY", "magnitude": 120.0, "units": "mm[Hg]",
               "accuracy": 0.0, "accuracy_is_percent": true}),
        json!({"_type": "DV_COUNT", "magnitude": 3}),
        json!({"_type": "DV_PROPORTION", "numerator": 1.0, "denominator": 3.0,
               "type": 2}),
        json!({"_type": "DV_DURATION", "value": "P1DT2H"}),
        json!({"_type": "DV_DURATION", "value": "nonsense"}),
        json!({"_type": "DV_DATE", "value": "2021-02-31"}),
        json!({"_type": "DV_DATE_TIME", "value": "2021-05-17T10:30:00Z",
               "magnitude_status": "??"}),
        json!({"_type": "DV_TIME", "value": "10:30:00"}),
        json!({"_type": "DV_IDENTIFIER", "id": ""}),
        json!({"_type": "DV_PARSABLE", "value": "x", "formalism": ""}),
        json!({"_type": "TERM_MAPPING", "match": "=",
               "target": {"terminology_id": {"value": "x"}, "code_string": "1"}}),
        json!({"_type": "TERM_MAPPING", "match": "q",
               "target": {"terminology_id": {"value": "x"}, "code_string": "1"}}),
        json!({"_type": "DV_URI", "value": ""}),
        json!({"_type": "DV_EHR_URI", "value": "http://not-ehr"}),
        json!({"_type": "TERMINOLOGY_ID", "value": "SNOMED CT "}),
        json!({"_type": "ARCHETYPE_ID",
               "value": "openEHR-EHR-OBSERVATION.blood_pressure.v1"}),
        json!({"_type": "ARCHETYPE_ID", "value": "not-an-archetype-id"}),
        json!({"_type": "ARCHETYPED",
               "archetype_id": {"value": "openEHR-EHR-COMPOSITION.x.v1"},
               "rm_version": ""}),
        json!({"_type": "PARTY_IDENTIFIED"}),
        json!({"_type": "PARTY_IDENTIFIED", "name": ""}),
        json!({"_type": "EVENT_CONTEXT",
               "start_time": {"value": "2021-05-17T10:00:00Z"},
               "setting": {"value": "home",
                           "defining_code": {"terminology_id": {"value": "openehr"},
                                             "code_string": "225"}},
               "location": ""}),
    ];
    for node in &cases {
        assert!(
            fast_handled(node),
            "expected the fast path to handle {node}"
        );
        assert_eq!(two_tier(node), typed(node), "divergence on {node}");
    }
}

#[test]
fn element_xor_matches_typed() {
    let name = json!({"value": "systolic"});
    let value = json!({"_type": "DV_QUANTITY", "magnitude": 120.0, "units": "mm[Hg]"});
    let nf = json!({"_type": "DV_CODED_TEXT", "value": "unknown",
                    "defining_code": {"terminology_id": {"value": "openehr"},
                                      "code_string": "253"}});
    for element in [
        json!({"_type": "ELEMENT", "name": name, "archetype_node_id": "at0001",
               "value": value}),
        json!({"_type": "ELEMENT", "name": name, "archetype_node_id": "at0001",
               "null_flavour": nf}),
        json!({"_type": "ELEMENT", "name": name, "archetype_node_id": "at0001",
               "value": value, "null_flavour": nf}),
        json!({"_type": "ELEMENT", "name": name, "archetype_node_id": "",
               "value": value, "null_reason": {"value": "why"}}),
    ] {
        assert!(fast_handled(&element), "not handled: {element}");
        assert_eq!(two_tier(&element), typed(&element), "on {element}");
    }
}

/// A minimal valid `ELEMENT` for the `ITEM_STRUCTURE` fixtures below.
fn element() -> Value {
    json!({"_type": "ELEMENT", "name": {"value": "e"},
           "archetype_node_id": "at0002",
           "value": {"_type": "DV_TEXT", "value": "x"}})
}

/// The `ITEM_STRUCTURE` family + `EHR_STATUS` with the given `archetype_node_id`.
fn locatables_without_a_typed_impl(node_id: &str) -> Vec<Value> {
    vec![
        json!({"_type": "ITEM_TREE", "name": {"value": "tree"},
               "archetype_node_id": node_id, "items": [element()]}),
        json!({"_type": "ITEM_LIST", "name": {"value": "list"},
               "archetype_node_id": node_id, "items": [element()]}),
        json!({"_type": "ITEM_SINGLE", "name": {"value": "single"},
               "archetype_node_id": node_id, "item": element()}),
        json!({"_type": "EHR_STATUS", "name": {"value": "status"},
               "archetype_node_id": node_id, "subject": {"_type": "PARTY_SELF"},
               "is_queryable": true, "is_modifiable": true}),
    ]
}

/// `LOCATABLE.Archetype_node_id_valid` (`not archetype_node_id.is_empty`,
/// `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.locatable.adoc`
/// §Invariants) fires with FULL fast-vs-typed equivalence on four
/// representative untyped-impl fixtures (`ITEM_TREE`, `ITEM_LIST`,
/// `ITEM_SINGLE`, `EHR_STATUS`) — the totality claim over the whole concrete
/// closure lives in `node_id_totality_is_model_derived` (#2442), which
/// derives its membership from the generated model.
#[test]
fn inherited_node_id_invariant_reaches_every_concrete_locatable() {
    for node in locatables_without_a_typed_impl("") {
        let ty = node["_type"].as_str().unwrap();
        assert!(!fast_handled(&node), "fast path must not vouch for {node}");
        let t = typed(&node);
        assert_eq!(two_tier(&node), t, "fallback divergence on {node}");
        assert!(
            t.iter()
                .any(|v| v.message
                    == format!("Invariant Archetype_node_id_valid failed on type {ty}")),
            "expected the inherited Archetype_node_id_valid violation on {ty}, got {t:?}"
        );
    }
}

/// The accepting twin of [`inherited_node_id_invariant_reaches_every_concrete_locatable`]:
/// the same nodes with a non-empty `archetype_node_id` carry no violation, so
/// the closeout cannot over-reject.
#[test]
fn valid_locatables_without_a_typed_impl_stay_clean() {
    for node in locatables_without_a_typed_impl("at0001") {
        let t = typed(&node);
        assert!(t.is_empty(), "expected a clean node, got {t:?} on {node}");
        assert_eq!(two_tier(&node), t, "fallback divergence on {node}");
    }
}

/// The inherited invariant is reported **once** per node: the classes whose
/// typed impl already realizes it must not gain a duplicate from the closeout.
#[test]
fn inherited_node_id_invariant_is_reported_once() {
    for node in [
        json!({"_type": "CLUSTER", "name": {"value": "c"},
               "archetype_node_id": "", "items": [element()]}),
        json!({"_type": "SECTION", "name": {"value": "s"}, "archetype_node_id": ""}),
        json!({"_type": "ITEM_TREE", "name": {"value": "t"},
               "archetype_node_id": "", "items": [element()]}),
    ] {
        let ty = node["_type"].as_str().unwrap();
        let expected = format!("Invariant Archetype_node_id_valid failed on type {ty}");
        let n = typed(&node)
            .iter()
            .filter(|v| v.message == expected)
            .count();
        assert_eq!(n, 1, "expected exactly one {expected}, got {n} on {node}");
    }
}

/// An **absent** `archetype_node_id` is a structural defect of a mandatory
/// attribute (reported by the decode), not an `Archetype_node_id_valid`
/// violation — the two must not double-report the same missing value.
#[test]
fn absent_node_id_is_structural_not_an_invariant_violation() {
    let node = json!({"_type": "ITEM_TREE", "name": {"value": "tree"},
                      "items": [element()]});
    let t = typed(&node);
    assert!(
        !t.iter()
            .any(|v| v.message.contains("Archetype_node_id_valid")),
        "an absent archetype_node_id must not raise the invariant: {t:?}"
    );
    assert!(
        t.iter()
            .any(|v| v.message.contains("does not conform to RM type ITEM_TREE")),
        "an absent mandatory attribute must be refused structurally: {t:?}"
    );
}

#[test]
fn nonconforming_nodes_fall_back_with_identical_output() {
    // Each of these fails the typed deserialize → the fast path must decline and
    // the two-tier output must equal the typed output.
    let cases = [
        json!({"_type": "DV_QUANTITY", "units": "kg"}),
        json!({"_type": "DV_TEXT", "value": null}),
        json!({"_type": "DV_TEXT", "value": 42}),
        json!({"_type": "DV_COUNT", "magnitude": 3.5}),
        json!({"_type": "DV_PROPORTION", "numerator": 1.0, "denominator": 1.0,
               "type": 4_000_000_000_i64}),
        json!({"_type": "CODE_PHRASE",
               "terminology_id": {"_type": "DV_TEXT", "value": "x"},
               "code_string": "1"}),
        json!({"_type": "ELEMENT", "name": {"value": "n"},
               "archetype_node_id": "at0001", "value": {"value": "x"}}),
        json!({"_type": "DV_TEXT", "value": "ok", "mappings": null}),
        json!({"_type": "TERM_MAPPING", "match": "==",
               "target": {"terminology_id": {"value": "x"}, "code_string": "1"}}),
    ];
    for node in &cases {
        assert!(!fast_handled(node), "fast path must not vouch for {node}");
        let t = typed(node);
        assert_eq!(two_tier(node), t, "fallback divergence on {node}");
        assert!(
            !t.is_empty(),
            "the typed oracle should reject {node}, got clean"
        );
    }
}

#[test]
fn unmodelled_shapes_fall_back() {
    let with_range = json!({"_type": "DV_COUNT", "magnitude": 1,
        "normal_status": {"terminology_id": {"value": "openehr"},
                          "code_string": "N"},
        "normal_range": {"lower": {"_type": "DV_COUNT", "magnitude": 0},
                         "upper": {"_type": "DV_COUNT", "magnitude": 5},
                         "lower_unbounded": false, "upper_unbounded": false,
                         "lower_included": true, "upper_included": true}});
    assert!(!fast_handled(&with_range));
    assert_eq!(two_tier(&with_range), typed(&with_range));

    let periodic = json!({"_type": "HISTORY", "name": {"value": "h"},
        "archetype_node_id": "at0001",
        "origin": {"value": "2021-05-17T10:00:00Z"},
        "period": {"value": "PT1H"},
        "events": [{"_type": "POINT_EVENT", "name": {"value": "e"},
                    "archetype_node_id": "at0002",
                    "time": {"value": "2021-05-17T10:30:00Z"},
                    "data": {"_type": "ITEM_TREE", "name": {"value": "d"},
                             "archetype_node_id": "at0003", "items": []}}]});
    assert!(!fast_handled(&periodic));
    assert_eq!(two_tier(&periodic), typed(&periodic));

    let multimedia = json!({"_type": "DV_MULTIMEDIA",
        "media_type": {"terminology_id": {"value": "IANA_media-types"},
                       "code_string": "image/png"},
        "size": 100});
    assert!(!fast_handled(&multimedia));
    assert_eq!(two_tier(&multimedia), typed(&multimedia));
}

#[test]
fn shallow_collections_mirror_the_prune() {
    let mixed = json!({"_type": "CLUSTER", "name": {"value": "c"},
        "archetype_node_id": "at0001",
        "items": [{"_type": "ELEMENT", "name": {"value": "e"},
                   "archetype_node_id": "", "value": {"_type": "DV_COUNT",
                                                      "magnitude": 1}}, "stray"]});
    assert!(fast_handled(&mixed));
    assert_eq!(two_tier(&mixed), typed(&mixed));

    let scalars = json!({"_type": "CLUSTER", "name": {"value": "c"},
        "archetype_node_id": "at0001", "items": ["not-an-item"]});
    assert!(!fast_handled(&scalars));
    assert_eq!(two_tier(&scalars), typed(&scalars));
}

// ── corpus equivalence: the load-bearing oracle ──────────────────────────────

/// Every `_type`-bearing object node in `v`, depth-first.
fn collect_nodes<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(map) => {
            if map.get("_type").is_some_and(Value::is_string) {
                out.push(v);
            }
            for child in map.values() {
                collect_nodes(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_nodes(item, out);
            }
        }
        _ => {}
    }
}

/// The documents this gate (and its mutation battery) may use as a base.
///
/// Routed through the single exclusion registry: a document
/// [`crate::common::excluded`] has adjudicated NOT a canonical RM 1.2 object
/// (a raw-DB/ITS-REST shape, a deliberately-invalid fixture, a defective
/// vendored one) is a spec-forbidden state, so asserting validation behaviour
/// on it — or MUTATING it — asserts behaviour on a document the spec already
/// refuses. Where the adjudication produced a repo-authored VALID TWIN,
/// [`crate::common::twinned`] substitutes it, so honouring the exclusions
/// costs the battery no coverage; an excluded document with no twin is
/// dropped.
/// Records one corpus JSON file, substituting an excluded document's valid twin.
fn push_corpus_file(path: std::path::PathBuf, files: &mut Vec<std::path::PathBuf>) {
    if crate::common::excluded(&crate::common::corpus_rel(&path)).is_none() {
        files.push(path);
        return;
    }
    let twin = crate::common::twinned(&path);
    if twin != path {
        files.push(twin);
    }
}

fn corpus_files() -> Vec<std::path::PathBuf> {
    let mut roots = vec![std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vendor/openehr_sdk"
    ))];
    // The benchmark CKM examples exercise the exact hot commit shapes.
    roots.push(std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../corpus/templates/ckm"
    )));
    let mut files = Vec::new();
    while let Some(dir) = roots.pop() {
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("read corpus dir {}: {e}", dir.display()));
        for entry in entries {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                roots.push(path);
            } else if path.extension().and_then(std::ffi::OsStr::to_str) == Some("json") {
                push_corpus_file(path, &mut files);
            }
        }
    }
    files.sort();
    files.dedup();
    assert!(
        files.len() >= 50,
        "corpus went missing? found only {} json files",
        files.len()
    );
    files
}

/// Every corpus node must produce byte-identical violations through the two-tier
/// entry point and the typed oracle.
#[test]
fn corpus_equivalence_valid_nodes() {
    let mut total = 0usize;
    let mut fast = 0usize;
    for path in corpus_files() {
        let text =
            std::fs::read_to_string(crate::common::twinned(&path)).expect("read corpus file");
        let Ok(doc) = serde_json::from_str::<Value>(&text) else {
            continue; // non-RM json (e.g. web templates) — skip unparseable
        };
        let mut nodes = Vec::new();
        collect_nodes(&doc, &mut nodes);
        for node in nodes {
            total += 1;
            if fast_handled(node) {
                fast += 1;
            }
            assert_eq!(
                two_tier(node),
                typed(node),
                "divergence in {} on {node}",
                path.display()
            );
        }
    }
    eprintln!("corpus equivalence: {total} nodes, {fast} fast-handled");
    assert!(total > 3_000, "expected a real corpus, saw {total} nodes");
}

/// Mutation equivalence: for the first-seen (`_type`, key) pair in the corpus,
/// mutate that key through a battery of shape changes and assert the two-tier
/// output still equals the typed oracle.
#[test]
fn corpus_equivalence_mutated_nodes() {
    let mutations: &[Value] = &[
        Value::Null,
        json!(42),
        json!(3.5),
        json!("mutated"),
        json!(""),
        json!(true),
        json!([]),
        json!({}),
        json!([42]),
        json!([{}]),
        json!({"_type": "DV_QUANTITY"}),
    ];
    let mut seen = std::collections::HashSet::new();
    let mut checked = 0usize;
    for path in corpus_files() {
        let text =
            std::fs::read_to_string(crate::common::twinned(&path)).expect("read corpus file");
        let Ok(doc) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let mut nodes = Vec::new();
        collect_nodes(&doc, &mut nodes);
        for node in nodes {
            let Value::Object(map) = node else { continue };
            let ty = map
                .get("_type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                if key == "_type" || !seen.insert((ty.clone(), key.clone())) {
                    continue;
                }
                checked += assert_key_equivalence(map, &ty, &key, mutations);
            }
            // An unknown key must stay ignored on both paths.
            if seen.insert((ty.clone(), "__unknown__".into())) {
                let mut extra = map.clone();
                extra.insert("__unknown_key__".into(), json!(42));
                let extra = Value::Object(extra);
                assert_eq!(
                    two_tier(&extra),
                    typed(&extra),
                    "divergence adding unknown key on {ty}"
                );
                checked += 1;
            }
        }
    }
    eprintln!("mutation equivalence: {checked} mutated nodes checked");
    assert!(checked > 500, "mutation battery too small: {checked}");
}

/// Asserts the two paths agree on one key's removal and on every mutated shape,
/// returning how many comparisons were made.
fn assert_key_equivalence(
    map: &serde_json::Map<String, Value>,
    ty: &str,
    key: &str,
    mutations: &[Value],
) -> usize {
    let mut removed = map.clone();
    removed.shift_remove(key);
    let removed = Value::Object(removed);
    assert_eq!(
        two_tier(&removed),
        typed(&removed),
        "divergence removing {ty}.{key}"
    );
    for m in mutations {
        let mut mutated = map.clone();
        mutated.insert(key.to_owned(), m.clone());
        let mutated = Value::Object(mutated);
        assert_eq!(
            two_tier(&mutated),
            typed(&mutated),
            "divergence mutating {ty}.{key} to {m}"
        );
    }
    1 + mutations.len()
}

/// The hot commit shape must actually ride the fast path: on the populated IPS
/// example the overwhelming majority of dispatched nodes are handled without a
/// typed deserialize. Guards the perf property against silent coverage
/// regressions.
#[test]
fn ips_nodes_ride_the_fast_path() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../corpus/templates/ckm/international-patient-summary.example.json"
    );
    let doc: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("read IPS example"))
            .expect("parse IPS example");
    let mut nodes = Vec::new();
    collect_nodes(&doc, &mut nodes);
    let mut dispatched = 0usize;
    let mut fast = 0usize;
    for node in nodes {
        let mut out = Vec::new();
        let ty = node.get("_type").and_then(Value::as_str).unwrap_or("");
        validate_rm_value_typed(ty, node, &mut out);
        let mut fast_out = Vec::new();
        let handled = try_fast_validate(ty, node, &mut fast_out);
        if handled {
            fast += 1;
            assert_eq!(fast_out, out, "IPS divergence on {node}");
        }
        if matches!(
            ty,
            "CODE_PHRASE"
                | "DV_TEXT"
                | "DV_CODED_TEXT"
                | "DV_QUANTITY"
                | "DV_COUNT"
                | "DV_DATE_TIME"
                | "ELEMENT"
                | "CLUSTER"
                | "OBSERVATION"
                | "SECTION"
                | "COMPOSITION"
                | "TERMINOLOGY_ID"
        ) {
            dispatched += 1;
        }
    }
    assert!(
        fast * 10 >= dispatched * 9,
        "fast-path coverage regressed: {fast} fast of {dispatched} hot nodes"
    );
}

// ── terminology-backed invariants ────────────────────────────────────────────
//
// The terminology binding table
// (`openehr_rm::v1_2::validate::terminology::validate_rm_terminology`, run by
// `validate_rm_value` as a post-core check). Two properties: the corpus-audit
// safety property (no valid corpus document is newly rejected) and the
// per-vocabulary enforcement property (an out-of-vocabulary code IS rejected).
// Spec: the RM class invariants resolved against the TERM 3.1.0 bundle.

/// A bare `CODE_PHRASE` for an external (ISO/IANA) code-set slot.
fn code_phrase(terminology: &str, code: &str) -> Value {
    json!({
        "_type": "CODE_PHRASE",
        "terminology_id": {"_type": "TERMINOLOGY_ID", "value": terminology},
        "code_string": code,
    })
}

/// A `DV_CODED_TEXT` for an openEHR-group slot.
fn coded(terminology: &str, code: &str) -> Value {
    json!({
        "_type": "DV_CODED_TEXT", "value": "x",
        "defining_code": code_phrase(terminology, code),
    })
}

fn terminology_of(ty: &str, value: &Value) -> Vec<InvariantViolation> {
    let mut out = Vec::new();
    validate_rm_terminology(ty, value, &mut out);
    out
}

/// **Corpus audit (the enforce-safety property).** Running the terminology hook
/// over every `_type` node of the whole valid corpus produces ZERO violations:
/// enforcing these invariants at the dispatcher never rejects a document the
/// repository already accepts. (This is the per-invariant AUDIT-mode sweep that
/// clears every wired invariant for enforcement — a corpus-breaking invariant
/// would surface here as a concrete finding rather than being enforced.)
#[test]
fn corpus_terminology_audit_is_clean() {
    let mut total = 0usize;
    let mut findings: Vec<String> = Vec::new();
    for path in corpus_files() {
        let text =
            std::fs::read_to_string(crate::common::twinned(&path)).expect("read corpus file");
        let Ok(doc) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let mut nodes = Vec::new();
        collect_nodes(&doc, &mut nodes);
        for node in nodes {
            let ty = node.get("_type").and_then(Value::as_str).unwrap_or("");
            let v = terminology_of(ty, node);
            total += 1;
            for iv in v {
                findings.push(format!(
                    "{}: {ty} {} — {}",
                    path.file_name()
                        .and_then(std::ffi::OsStr::to_str)
                        .unwrap_or("?"),
                    iv.path,
                    iv.message,
                ));
            }
        }
    }
    assert!(total > 3_000, "expected a real corpus, saw {total} nodes");
    assert!(
        findings.is_empty(),
        "terminology enforcement would reject {} valid corpus node(s) — \
         adjudicate each as EXCLUDED-PENDING before enforcing:\n{}",
        findings.len(),
        findings.join("\n"),
    );
    eprintln!("terminology corpus audit: {total} nodes, all clean");
}

/// The full unified entry (`validate_rm_value`) equals core + terminology on a
/// coded node: a valid code adds nothing beyond the core invariants, an invalid
/// code adds exactly the terminology violation.
#[test]
fn full_entry_is_core_plus_terminology() {
    // A valid DV_TEXT.encoding adds no terminology violation over the core.
    let good = json!({"_type": "DV_TEXT", "value": "x",
                      "encoding": code_phrase("IANA_character-sets", "UTF-8")});
    assert_eq!(full(&good), two_tier(&good), "valid encoding: core == full");

    // An invalid encoding adds exactly one terminology violation over the core.
    let bad = json!({"_type": "DV_TEXT", "value": "x",
                     "encoding": code_phrase("IANA_character-sets", "NOT-A-CHARSET")});
    let core = two_tier(&bad);
    let extra: Vec<_> = full(&bad)
        .into_iter()
        .filter(|iv| !core.contains(iv))
        .collect();
    assert_eq!(
        extra.len(),
        1,
        "one extra terminology violation, got {extra:?}"
    );
    assert!(
        extra[0].message.contains("Encoding_valid"),
        "expected Encoding_valid, got {extra:?}"
    );
}

/// Group bindings (`has_code_for_group_id`): an out-of-group openEHR code is
/// rejected, a valid one is clean, and a non-`openehr` terminology binding is
/// out of scope (skipped).
#[test]
fn terminology_group_enforcement() {
    // COMPOSITION.category (composition_category).
    assert!(
        terminology_of(
            "COMPOSITION",
            &json!({"_type": "COMPOSITION", "category": coded("openehr", "99999")})
        )
        .iter()
        .any(|iv| iv.message.contains("Category_validity")),
        "bad category must be rejected"
    );
    assert!(
        terminology_of(
            "COMPOSITION",
            &json!({"_type": "COMPOSITION", "category": coded("openehr", "433")})
        )
        .is_empty(),
        "valid category 433 must be clean"
    );
    // A non-openehr terminology binding is out of scope for the group check.
    assert!(
        terminology_of(
            "COMPOSITION",
            &json!({"_type": "COMPOSITION", "category": coded("local", "99999")})
        )
        .is_empty(),
        "non-openehr terminology is out of scope for the openEHR-group check"
    );
    // PARTY_RELATED.relationship, AUDIT_DETAILS.change_type, ATTESTATION.reason,
    // ISM_TRANSITION.current_state, TERM_MAPPING.purpose, VERSION.lifecycle_state.
    for (ty, field, invariant) in [
        ("PARTY_RELATED", "relationship", "Relationship_valid"),
        ("AUDIT_DETAILS", "change_type", "Change_type_valid"),
        ("ATTESTATION", "reason", "Reason_valid"),
        ("ISM_TRANSITION", "current_state", "Current_state_valid"),
        ("TERM_MAPPING", "purpose", "Purpose_valid"),
        (
            "ORIGINAL_VERSION",
            "lifecycle_state",
            "Lifecycle_state_valid",
        ),
        ("PARTICIPATION", "function", "Function_valid"),
        ("PARTICIPATION", "mode", "Mode_valid"),
    ] {
        let node = json!({"_type": ty, field: coded("openehr", "99999")});
        assert!(
            terminology_of(ty, &node)
                .iter()
                .any(|iv| iv.message.contains(invariant)),
            "{ty}.{field} bad code must trip {invariant}"
        );
    }
}

/// Code-set bindings (`code_set(id).has_code`): the code value is validated
/// against the set regardless of the stated terminology id; a member is clean.
#[test]
fn terminology_code_set_enforcement() {
    // COMPOSITION.language (ISO 639-1) + .territory (ISO 3166-1).
    let bad = json!({
        "_type": "COMPOSITION",
        "language": code_phrase("ISO_639-1", "zz"),
        "territory": code_phrase("ISO_3166-1", "ZZ"),
    });
    let v = terminology_of("COMPOSITION", &bad);
    assert!(v.iter().any(|iv| iv.message.contains("Language_valid")));
    assert!(v.iter().any(|iv| iv.message.contains("Territory_valid")));
    let good = json!({
        "_type": "COMPOSITION",
        "language": code_phrase("ISO_639-1", "en"),
        "territory": code_phrase("ISO_3166-1", "US"),
    });
    assert!(terminology_of("COMPOSITION", &good).is_empty());

    // DV_TEXT.encoding (IANA character sets), DV_QUANTITY.normal_status,
    // DV_MULTIMEDIA.media_type + compression/integrity algorithms.
    assert!(
        terminology_of(
            "DV_TEXT",
            &json!({"_type": "DV_TEXT", "value": "x",
                    "encoding": code_phrase("IANA_character-sets", "NOT-A-CHARSET")})
        )
        .iter()
        .any(|iv| iv.message.contains("Encoding_valid"))
    );
    assert!(
        terminology_of(
            "DV_QUANTITY",
            &json!({"_type": "DV_QUANTITY", "magnitude": 1.0, "units": "kg",
                    "normal_status": code_phrase("openehr", "X")})
        )
        .iter()
        .any(|iv| iv.message.contains("Normal_status_validity"))
    );
    assert!(
        terminology_of(
            "DV_MULTIMEDIA",
            &json!({"_type": "DV_MULTIMEDIA", "media_type": code_phrase("IANA_media-types", "no/such")})
        )
        .iter()
        .any(|iv| iv.message.contains("Media_type_valid"))
    );
}

/// An uncoded slot (plain `DV_TEXT` participation function) or an absent optional
/// slot is skipped — the `generating_type = DV_CODED_TEXT` / `/= Void`
/// antecedents guard the terminology check.
#[test]
fn terminology_uncoded_and_absent_slots_are_skipped() {
    // PARTICIPATION.function as a plain DV_TEXT (no defining_code): skipped.
    let plain = json!({
        "_type": "PARTICIPATION",
        "function": {"_type": "DV_TEXT", "value": "attending physician"},
    });
    assert!(terminology_of("PARTICIPATION", &plain).is_empty());
    // Absent optional slot (ISM_TRANSITION.transition): skipped.
    let no_transition =
        json!({"_type": "ISM_TRANSITION", "current_state": coded("openehr", "245")});
    assert!(terminology_of("ISM_TRANSITION", &no_transition).is_empty());
}

/// A half-open `DV_INTERVAL` is ACCEPTED whether or not its boundary flags
/// are present on the wire. BASE
/// `org.openehr.base.foundation_types.interval.adoc` declares `lower`/`upper`
/// as 0..1 with a CLOSED four-invariant set (`Limits_consistent`,
/// `Limits_comparable`, `Lower/Upper_included_valid`) — no invariant requires
/// a bound value when its `*_unbounded` flag is false; the guarded
/// implications are unevaluable then and skip.
/// The tolerant read defaults missing flags to `false`, which violates
/// nothing.
#[test]
fn half_open_interval_is_accepted_with_or_without_flags() {
    let flagless = json!({
        "_type": "DV_INTERVAL",
        "upper": {"_type": "DV_DURATION", "value": "PT1S"}
    });
    assert!(full(&flagless).is_empty(), "got {:?}", full(&flagless));

    let flagged = json!({
        "_type": "DV_INTERVAL",
        "upper": {"_type": "DV_DURATION", "value": "PT1S"},
        "lower_unbounded": true,
        "upper_unbounded": false,
        "lower_included": false,
        "upper_included": true
    });
    assert!(full(&flagged).is_empty(), "got {:?}", full(&flagged));
}

// ── ELEMENT Inv_null_flavour_indicated (RM data_structures §ELEMENT) ──────────

/// The XOR arms of RM `data_structures` §ELEMENT `Inv_null_flavour_indicated`:
///
/// > `Inv_null_flavour_indicated`: `is_null() xor null_flavour = Void`
/// > (`RM/docs/UML/classes/org.openehr.rm.data_structures.element.adoc`)
///
/// `is_null()` is "value is Void", so the invariant holds for exactly one of
/// `value` / `null_flavour` being present. BOTH arms fail: an ELEMENT carrying
/// both, and — the arm the vendored corpus actually contains — an ELEMENT
/// carrying neither. Pinned on both dispatcher tiers, because the fast path
/// may only skip the typed oracle when its result is identical.
#[test]
fn element_null_flavour_xor_fails_on_both_and_on_neither() {
    let element = |extra: Value| {
        let mut e = json!({
            "_type": "ELEMENT",
            "archetype_node_id": "at0002",
            "name": {"_type": "DV_TEXT", "value": "an element"},
        });
        if let (Some(o), Some(x)) = (e.as_object_mut(), extra.as_object()) {
            for (k, v) in x {
                o.insert(k.clone(), v.clone());
            }
        }
        e
    };
    let value = json!({"value": {"_type": "DV_TEXT", "value": "a datum"}});
    let null_flavour = json!({"null_flavour": {
        "_type": "DV_CODED_TEXT", "value": "no information",
        "defining_code": {"_type": "CODE_PHRASE",
            "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "openehr"},
            "code_string": "271"},
    }});
    let mut both = value.clone();
    if let (Some(o), Some(x)) = (both.as_object_mut(), null_flavour.as_object()) {
        for (k, v) in x {
            o.insert(k.clone(), v.clone());
        }
    }

    let violated = |v: &Value| {
        two_tier(v)
            .iter()
            .any(|iv| iv.message == "Invariant Inv_null_flavour_indicated failed on type ELEMENT")
    };

    // The two valid shapes.
    for ok in [element(value), element(null_flavour)] {
        assert!(!violated(&ok), "must be accepted: {:?}", two_tier(&ok));
    }
    // Both present.
    let both = element(both);
    assert!(violated(&both), "got {:?}", two_tier(&both));
    // Neither present — the corpus case.
    let neither = element(json!({}));
    assert!(violated(&neither), "got {:?}", two_tier(&neither));

    // Same verdict from the authoritative typed oracle, on the tier the fast
    // path claims to be equivalent to.
    for case in [&both, &neither] {
        assert!(fast_handled(case), "the fast path handles a plain ELEMENT");
        assert_eq!(two_tier(case), typed(case));
    }
}

/// The vendored openEHR-SDK corpus carries this violation for real: both
/// `all_types_systematic_tests.json` and
/// `all_types_systematic_tests_feeder_audit.json` hold one ELEMENT
/// (`content[1]/data/items[0]`, `at0002`) with neither `value` nor
/// `null_flavour` — RM-invalid per `Inv_null_flavour_indicated`.
///
/// Adjudication: the fixtures are NOT corrected. They exist to exercise the
/// canonical-JSON codec, and the only gates that consume them (`fidelity.rs` —
/// read, lossless re-serialize,
/// ITS-JSON schema validation) are codec gates that do not run RM class
/// invariants, so their verdicts are unaffected. The RM-invalid node is turned
/// into a positive assertion instead: the dispatcher must reject it, naming
/// the invariant. A fixture that stops carrying the violation fails here and
/// must be re-adjudicated, never silently dropped.
#[test]
fn corpus_elements_without_value_or_null_flavour_are_rejected() {
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vendor/openehr_sdk/composition/canonical_json"
    );
    for name in [
        "all_types_systematic_tests.json",
        "all_types_systematic_tests_feeder_audit.json",
    ] {
        let text = std::fs::read_to_string(format!("{dir}/{name}"))
            .unwrap_or_else(|e| panic!("read {name}: {e}"));
        let composition: Value =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {name}: {e}"));
        let offenders = value_less_elements(&composition);
        assert_eq!(
            offenders.len(),
            1,
            "{name}: expected the one known RM-invalid ELEMENT, found {}",
            offenders.len()
        );
        for element in offenders {
            let violations = two_tier(element);
            assert!(
                violations.iter().any(|iv| iv.message
                    == "Invariant Inv_null_flavour_indicated failed on type ELEMENT"),
                "{name}: an ELEMENT with neither value nor null_flavour must be \
                 rejected, got {violations:?}"
            );
        }
    }
}

/// Every ELEMENT in `value` carrying neither `value` nor `null_flavour`.
fn value_less_elements(value: &Value) -> Vec<&Value> {
    fn walk<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
        match v {
            Value::Object(map) => {
                let present = |k: &str| map.get(k).is_some_and(|x| !x.is_null());
                if map.get("_type").and_then(Value::as_str) == Some("ELEMENT")
                    && !present("value")
                    && !present("null_flavour")
                {
                    out.push(v);
                }
                for child in map.values() {
                    walk(child, out);
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(value, &mut out);
    out
}

/// `Archetype_node_id_valid` reaches EVERY concrete LOCATABLE descendant of
/// the generated model — the totality half the fixture-listed test above
/// cannot carry (#2442).
///
/// The membership is the generated closure itself
/// (`openehr_rm::v1_2::model::descendants("LOCATABLE")`), so an emitter
/// regression that drops a class from the closure — silently ending its
/// enforcement — fails here instead of going dark. The dispatcher's closeout
/// reads the node's own `_type` + `archetype_node_id` off the JSON value, so
/// a minimal node per class suffices; the class's other invariants firing
/// beside it are irrelevant to the assertion.
#[test]
fn node_id_totality_is_model_derived() {
    let concrete = openehr_rm::v1_2::model::descendants("LOCATABLE");
    assert!(
        concrete.len() >= 39,
        "the concrete LOCATABLE closure shrank to {} — emitter regression?",
        concrete.len()
    );
    for ty in concrete {
        let node = json!({"_type": ty, "archetype_node_id": ""});
        let violations = two_tier(&node);
        assert!(
            violations
                .iter()
                .any(|v| v.message
                    == format!("Invariant Archetype_node_id_valid failed on type {ty}")),
            "{ty}: an empty archetype_node_id must violate Archetype_node_id_valid, got {violations:?}"
        );
    }
}
