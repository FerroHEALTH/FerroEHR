// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![expect(clippy::panic, reason = "test assertions/diagnostics/fixtures")]
//! The §Validation checklist audit.
//!
//! A point-by-point pin of the eight SHOULD-validate bullets the Simplified
//! Formats specification lists under
//! `docs/specs/openehr/ITS-REST/docs/simplified_formats/master04-basic_concepts.adoc`
//! §Validation ("Implementations SHOULD validate:") against the `openehr_sdt::flat`
//! ingest validators. There is exactly one test per bullet; each quotes its
//! bullet verbatim, cites `master04 §Validation`, and drives the rule through a
//! real public seam of the crate (`sim::flat::parse_flat`,
//! `validation::validate_context`, `validation::validate_composition`, and the
//! `convert::composition_*` conversion seam) with a positive case (the rule is
//! satisfied → clean / accepted) and a negative probe (a minimal violation →
//! the expected `ValidationKind` / `FlatError`).
//!
//! The end-to-end corpus + per-rule seam tests live in `tests/validation.rs`
//! and `tests/flat.rs`; this file is the dedicated audit that maps each spec
//! bullet to the seam that enforces it, so "substantively covered" becomes
//! pinned proof.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use openehr_its::opt14;
use openehr_sdt::flat::convert::{composition_from_flat, composition_to_flat};
use openehr_sdt::flat::error::FlatError;
use openehr_sdt::flat::sim::flat::parse_flat;
use openehr_sdt::flat::validation::validate_context;
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::{
    WebTemplate, WebTemplateCardinality, WebTemplateClosedAttribute, WebTemplateCodedValue,
    WebTemplateInput, WebTemplateInputType, WebTemplateNode,
};
use openehr_sdt::rm_instance::{ValidationKind, ValidationMessage, validate_composition};
use serde_json::{Map, Value, json};

/// Fixed `ctx/time` default for the FLAT build direction (master04 §Context) so
/// the conversion seam is deterministic under test.
const NOW: &str = "2024-01-01T00:00:00Z";

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

// ── shared helpers ───────────────────────────────────────────────────────────

/// A FLAT input map from key/value pairs (the wire shape `parse_flat` reads).
fn flat_doc(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

/// A bare `WebTemplateNode` with the given RM type / aql path.
fn node(rm: &str, path: &str) -> WebTemplateNode {
    WebTemplateNode::new(rm.to_owned(), path.to_owned())
}

/// Wrap a hand-shaped root node in a minimal `WebTemplate`.
fn wt_of(tree: WebTemplateNode) -> WebTemplate {
    WebTemplate {
        template_id: "t".to_owned(),
        sem_ver: None,
        version: "2.3".to_owned(),
        default_language: "en".to_owned(),
        languages: vec!["en".to_owned()],
        tree,
        other_details: indexmap::IndexMap::new(),
    }
}

fn kinds(msgs: &[ValidationMessage]) -> Vec<ValidationKind> {
    msgs.iter().map(|m| m.kind).collect()
}

// ── OPT-corpus loaders (mirrors tests/flat.rs) — for the conversion-seam bullet ─

fn composition_dir() -> PathBuf {
    manifest_dir().join("../openehr-its/tests/vendor/openehr_sdk/composition/canonical_json")
}

fn opt_dirs() -> Vec<PathBuf> {
    vec![
        manifest_dir().join("../openehr-its/tests/fixtures/sdk"),
        manifest_dir().join("tests/fixtures/better"),
        crate::common::service_corpus_dir(),
    ]
}

fn opt_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            opt_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "opt") {
            out.push(path);
        }
    }
}

/// `templateId → WebTemplate` for every OPT the `opt14` parser can read.
fn web_templates() -> BTreeMap<String, WebTemplate> {
    let mut out = BTreeMap::new();
    for dir in opt_dirs() {
        let mut files = Vec::new();
        opt_files(&dir, &mut files);
        files.sort();
        for path in files {
            let Ok(xml) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(opt) = opt14::from_xml(&xml) else {
                continue;
            };
            if let Ok(wt) = build_web_template(&opt) {
                out.entry(wt.template_id.clone()).or_insert(wt);
            }
        }
    }
    out
}

fn load_composition(name: &str) -> Value {
    let text = std::fs::read_to_string(crate::common::twinned(&composition_dir().join(name)))
        .unwrap_or_else(|e| panic!("read {name}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {name}: {e}"))
}

// ═══ the eight §Validation bullets ═══════════════════════════════════════════

// Bullet 1 (master04 §Validation): "Get the WT for the target template and map
// input fields to the identifiers".
//
// `parse_flat` maps each FLAT input key onto its hierarchical field-identifier
// path (the `SimNode` segment tree the WT is later matched against).
#[test]
fn bullet1_map_input_fields_to_identifiers() {
    // POSITIVE: a valid input maps its field onto the identifier tree, reachable
    // segment by segment (master04 §Flat format example).
    let doc = flat_doc(&[
        ("ctx/language", json!("en")),
        (
            "vital_signs/body_temperature:0/any_event:0/temperature|magnitude",
            json!(37.5),
        ),
    ]);
    let parsed = parse_flat(&doc).expect("a valid FLAT input parses");
    let temperature = parsed
        .child("vital_signs")
        .and_then(|n| n.child("body_temperature"))
        .and_then(|n| n.child("any_event"))
        .and_then(|n| n.child("temperature"))
        .expect("the input field is mapped onto its identifier path");
    assert!(
        temperature.attrs.contains_key("magnitude"),
        "the leaf datum is keyed under the mapped identifier"
    );

    // NEGATIVE: a syntactically invalid identifier is rejected, not silently
    // mismapped (master04 §Field Identifiers: no empty path segment).
    let bad = flat_doc(&[("vital_signs//temperature", json!(1))]);
    assert!(
        matches!(parse_flat(&bad), Err(FlatError::MalformedPath { .. })),
        "an invalid field identifier must be a MalformedPath"
    );
}

// Bullet 2 (master04 §Validation): "Check the final segment for the pipe to
// identify attribute suffix".
//
// `parse_flat` (via `path::FlatKey::parse`) splits the `|`-separated attribute
// suffix off the final segment, keying each suffix as its own datum part.
#[test]
fn bullet2_pipe_identifies_attribute_suffix() {
    // POSITIVE: two pipe suffixes on one leaf are each identified as a distinct
    // datum part (`magnitude` / `unit` — master05 §DV_QUANTITY).
    let doc = flat_doc(&[
        ("obs/temperature|magnitude", json!(37.5)),
        ("obs/temperature|unit", json!("°C")),
    ]);
    let parsed = parse_flat(&doc).unwrap();
    let leaf = parsed
        .child("obs")
        .and_then(|n| n.child("temperature"))
        .expect("the suffixed leaf");
    assert!(
        leaf.attrs.contains_key("magnitude") && leaf.attrs.contains_key("unit"),
        "each pipe suffix is identified as its own datum part: {:?}",
        leaf.attrs.keys().collect::<Vec<_>>()
    );
    assert!(
        !leaf.attrs.contains_key(""),
        "a suffixed leaf carries no bare (suffix-less) value"
    );

    // NEGATIVE: a trailing pipe with no suffix name is malformed (master04 §Flat
    // format syntax: no empty attribute suffix).
    let bad = flat_doc(&[("obs/temperature|", json!(1))]);
    assert!(
        matches!(parse_flat(&bad), Err(FlatError::MalformedPath { .. })),
        "an empty pipe suffix must be a MalformedPath"
    );
}

// Bullet 3 (master04 §Validation): "Mandatory context fields (language,
// territory) are present".
//
// `validation::validate_context` reports the absent mandatory `ctx/` fields on
// a parsed simplified document.
#[test]
fn bullet3_mandatory_context_fields_present() {
    // POSITIVE: both language and territory present → clean.
    let ok = parse_flat(&flat_doc(&[
        ("ctx/language", json!("en")),
        ("ctx/territory", json!("US")),
    ]))
    .unwrap();
    assert!(
        validate_context(&ok).is_empty(),
        "language + territory present must be clean"
    );

    // NEGATIVE: territory absent → a single Required violation keyed
    // `ctx/territory`.
    let missing = parse_flat(&flat_doc(&[("ctx/language", json!("en"))])).unwrap();
    let msgs = validate_context(&missing);
    assert!(
        msgs.iter()
            .any(|m| m.kind == ValidationKind::Required && m.path == "ctx/territory"),
        "an absent mandatory context field must be a Required violation: {msgs:?}"
    );
}

// Bullet 4 (master04 §Validation): "Field identifiers match WT metadata
// structure".
//
// `validation::validate_composition` walks the instance against the WebTemplate;
// an archetyped child under a closed attribute whose node id is not admitted by
// the WT structure is reported `ValidationKind::Unexpected`.
#[test]
fn bullet4_field_identifiers_match_wt_structure() {
    // A COMPOSITION whose `content` attribute the template closes to a single
    // archetype node id (`at0001`).
    let mut root = node("COMPOSITION", "");
    root.closed_attributes = vec![WebTemplateClosedAttribute {
        path: "/content".to_owned(),
        allowed_ids: vec!["at0001".to_owned()],
        slots: Vec::new(),
    }];
    let wt = wt_of(root);
    let section = |nid: &str| {
        json!({"_type": "SECTION", "archetype_node_id": nid,
               "name": {"_type": "DV_TEXT", "value": "s"}})
    };

    // POSITIVE: the admitted node id matches the WT structure → no Unexpected.
    let ok = json!({"_type": "COMPOSITION", "archetype_node_id": "x",
                    "content": [section("at0001")]});
    assert!(
        !kinds(&validate_composition(&ok, &wt)).contains(&ValidationKind::Unexpected),
        "an admitted field identifier matches the WT structure"
    );

    // NEGATIVE: a foreign node id is absent from the WT structure → Unexpected,
    // keyed at the closed attribute.
    let bad = json!({"_type": "COMPOSITION", "archetype_node_id": "x",
                     "content": [section("at0002")]});
    let msgs = validate_composition(&bad, &wt);
    assert!(
        msgs.iter()
            .any(|m| m.kind == ValidationKind::Unexpected && m.path == "/content"),
        "a field identifier absent from the WT structure must be Unexpected: {msgs:?}"
    );
}

// Bullet 5 (master04 §Validation): "Data types match expected types from the
// Operational Template".
//
// `validation::validate_composition` checks each matched node's RM type against
// the WebTemplate node's `rmType`; a non-conforming datum type is
// `ValidationKind::WrongType`.
#[test]
fn bullet5_data_types_match_operational_template() {
    let wt = wt_of(node("DV_QUANTITY", "/q"));

    // POSITIVE: a DV_QUANTITY where the OPT expects DV_QUANTITY → no WrongType.
    let ok = json!({"_type": "DV_QUANTITY", "magnitude": 1.0, "units": "kg"});
    assert!(
        !kinds(&validate_composition(&ok, &wt)).contains(&ValidationKind::WrongType),
        "a conforming datum type matches the OPT"
    );

    // NEGATIVE: a DV_TEXT where the OPT expects DV_QUANTITY → WrongType.
    let bad = json!({"_type": "DV_TEXT", "value": "x"});
    assert!(
        validate_composition(&bad, &wt)
            .iter()
            .any(|m| m.kind == ValidationKind::WrongType),
        "a datum type that does not match the OPT must be WrongType"
    );
}

// Bullet 6 (master04 §Validation): "Cardinality constraints are satisfied".
//
// `validation::validate_composition` counts the children under each constrained
// container attribute and reports `ValidationKind::Cardinality` when the count
// falls outside the interval.
#[test]
fn bullet6_cardinality_constraints_satisfied() {
    let mut root = node("COMPOSITION", "");
    root.card_all = vec![WebTemplateCardinality {
        min: Some(1),
        max: 2,
        ids: None,
        path: "/content".to_owned(),
    }];
    let wt = wt_of(root);
    let entry = json!({"_type": "OBSERVATION", "archetype_node_id": "a"});

    // POSITIVE: two children, within `1..2` → no Cardinality.
    let ok = json!({"_type": "COMPOSITION", "archetype_node_id": "x",
                    "content": [entry.clone(), entry.clone()]});
    assert!(
        !kinds(&validate_composition(&ok, &wt)).contains(&ValidationKind::Cardinality),
        "a container within its cardinality interval is clean"
    );

    // NEGATIVE: three children exceeds `max = 2` → Cardinality.
    let bad = json!({"_type": "COMPOSITION", "archetype_node_id": "x",
                     "content": [entry.clone(), entry.clone(), entry]});
    assert!(
        validate_composition(&bad, &wt)
            .iter()
            .any(|m| m.kind == ValidationKind::Cardinality),
        "a container exceeding its cardinality maximum must be Cardinality"
    );
}

// Bullet 7 (master04 §Validation): "Terminology bindings are valid".
//
// Two flavours of terminology binding, both via `validation::validate_composition`:
// an archetype coded-value list (`ValidationKind::CodedValue`) and the
// RM-mandated openEHR terminology groups (`ValidationKind::Terminology`).
#[test]
fn bullet7_terminology_bindings_valid() {
    // (a) archetype coded-list binding: a DV_CODED_TEXT leaf bound to a local
    // code list.
    let mut leaf = node("DV_CODED_TEXT", "/coded");
    let mut input = WebTemplateInput::new(WebTemplateInputType::CodedText, Some("code"));
    input.list = vec![WebTemplateCodedValue::new(
        "at0001",
        Some("at0001".to_owned()),
    )];
    leaf.inputs = vec![input];
    let coded_wt = wt_of(leaf);
    let coded = |code: &str| {
        json!({"_type": "DV_CODED_TEXT", "value": "x",
               "defining_code": {"_type": "CODE_PHRASE",
                   "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "local"},
                   "code_string": code}})
    };
    // POSITIVE: a code in the bound list → no CodedValue.
    assert!(
        !kinds(&validate_composition(&coded("at0001"), &coded_wt))
            .contains(&ValidationKind::CodedValue),
        "a coded value in its archetype binding is clean"
    );
    // NEGATIVE: a code outside the bound list → CodedValue.
    assert!(
        validate_composition(&coded("at0099"), &coded_wt)
            .iter()
            .any(|m| m.kind == ValidationKind::CodedValue),
        "a coded value outside its archetype binding must be CodedValue"
    );

    // (b) RM-mandated openEHR terminology binding: COMPOSITION.category must be a
    // valid code in the openEHR `composition category` group.
    let comp_wt = wt_of(node("COMPOSITION", ""));
    let comp = |code: &str| {
        json!({"_type": "COMPOSITION", "archetype_node_id": "x",
               "category": {"_type": "DV_CODED_TEXT", "value": "event",
                   "defining_code": {"_type": "CODE_PHRASE",
                       "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "openehr"},
                       "code_string": code}}})
    };
    // POSITIVE: 433 ("event") is valid in the group → no Terminology.
    assert!(
        !kinds(&validate_composition(&comp("433"), &comp_wt))
            .contains(&ValidationKind::Terminology),
        "a code valid in its RM-mandated terminology group is clean"
    );
    // NEGATIVE: 99999 is not in the group → Terminology.
    assert!(
        validate_composition(&comp("99999"), &comp_wt)
            .iter()
            .any(|m| m.kind == ValidationKind::Terminology),
        "a code outside its RM-mandated terminology group must be Terminology"
    );
}

// Bullet 8 (master04 §Validation): "RM attribute paths (underscore-prefixed) are
// valid".
//
// master04 §"RM Attributes prefix": optional RM attributes are addressed with a
// leading underscore (`_attributeName`); the valid families are the `_`-rows of
// the master05 per-type tables. The FLAT conversion seam
// (`convert::composition_from_flat`) rejects an underscore-prefixed segment
// naming no RM-attribute family as `FlatError::UnknownSuffix`.
#[test]
fn bullet8_underscore_rm_attribute_paths_valid() {
    let wts = web_templates();
    let wt = wts
        .get("Corona_Anamnese")
        .expect("Corona_Anamnese WebTemplate");
    let comp = load_composition("compo_corona.json");
    // A valid FLAT rendering of a clean composition — every field identifier in
    // it already resolves against the WT.
    let flat = composition_to_flat(&comp, wt).expect("render the clean composition to FLAT");
    let leaf_key = flat
        .keys()
        .find(|k| !k.starts_with("ctx/") && k.contains('|'))
        .expect("a suffixed data leaf")
        .clone();
    let leaf_path = leaf_key.split('|').next().unwrap_or(&leaf_key).to_owned();
    let container_path = leaf_path
        .rsplit_once('/')
        .map(|(parent, _)| parent.to_owned())
        .expect("the leaf sits under a container node");

    // POSITIVE: a valid underscore-prefixed RM attribute path (`_uid`, the
    // LOCATABLE.uid family — master05 §LOCATABLE) on a container node is
    // accepted and builds.
    let mut good = flat.clone();
    good.insert(
        format!("{container_path}/_uid"),
        json!("8849182c-82ad-4088-a07f-48ead4180515"),
    );
    assert!(
        composition_from_flat(&good, wt, NOW).is_ok(),
        "a valid `_`-prefixed RM attribute path must be accepted"
    );

    // NEGATIVE: an underscore-prefixed segment naming no RM-attribute family is
    // rejected (its path is not valid).
    let mut bad = flat;
    bad.insert(format!("{leaf_path}/_not_a_real_rm_attribute"), json!("x"));
    let err = composition_from_flat(&bad, wt, NOW)
        .expect_err("an invalid `_`-prefixed RM attribute path must be rejected");
    assert!(
        matches!(err, FlatError::UnknownSuffix { .. }),
        "an invalid RM-attribute path must be an UnknownSuffix, got: {err:?}"
    );
}
