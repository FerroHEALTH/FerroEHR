// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![expect(clippy::panic, reason = "test assertions/diagnostics/fixtures")]
//! Generated examples pass the server's own validation.
//!
//! ITS-REST Release 1.1.0, Definition API `definition_template_adl1.4_example_get`,
//! `detail_level`: the `required` and `medium` examples are "intended to be
//! committable". The commit path validates a COMPOSITION with exactly the
//! passes [`validate_composition`] composes (RM invariants + terminology, then
//! archetype conformance against the template), so every vendored template the
//! suite loads must generate examples that pass it at both levels.
//!
//! Two generator rules are pinned on their own as well:
//!
//! * an attribute the template makes mandatory (AOM 1.4
//!   `master04-constraint_model_package.adoc` §existence) is present even when
//!   its structural child carries no leaf content (an EVENT `state`, an ENTRY
//!   `protocol`), as the node its structural stub constrains;
//! * an ELEMENT never carries `value` and `null_flavour` together, and a
//!   value-less ELEMENT carries its `null_flavour` (RM `data_structures`
//!   §ELEMENT `Inv_null_flavour_indicated`: `is_null() xor null_flavour = Void`).

use std::path::{Path, PathBuf};

use openehr_its::opt14;
use openehr_rm::v1_2::paths::{PathSegment, RmPath};
use openehr_sdt::flat::example::{DetailLevel, example_composition};
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::{
    WebTemplate, WebTemplateExistence, WebTemplateNode, WebTemplateStructuralStub,
};
use openehr_sdt::rm_instance::validate_composition;
use serde_json::Value;

use crate::common::{opt_dirs, opt_files};

/// The two levels ITS-REST declares committable.
const COMMITTABLE: [DetailLevel; 2] = [DetailLevel::Required, DetailLevel::Medium];

fn ckm_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/templates/ckm")
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("?")
        .to_owned()
}

fn web_template(path: &Path) -> WebTemplate {
    let name = path.display();
    let xml = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {name}: {e}"));
    let opt = opt14::from_xml(&xml).unwrap_or_else(|e| panic!("parse {name}: {e}"));
    build_web_template(&opt).unwrap_or_else(|e| panic!("build {name}: {e}"))
}

/// The `.opt` files directly under `dir` (not its subdirectories), sorted.
fn top_level_opts(dir: &Path) -> Vec<PathBuf> {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "opt"))
        .collect();
    out.sort();
    out
}

/// The vendored templates whose examples cannot pass the commit path, each
/// with its reason.
const NOT_COMMITTABLE: &[(&str, &str)] = &[
    (
        "pregnancy-ecosystem-testing.opt",
        "the template pins COMPOSITION.category to 435, a code the openEHR \
         terminology's composition category group (431, 433, 451, 815) does not hold",
    ),
    // TODO(#3779): same-archetype siblings told apart by name merge in the example.
    (
        "covid-19-pneumonia-diagnosis-and-treatment-7th-edition.opt",
        "two name-distinguished imaging_exam_result siblings",
    ),
];

/// Generates every committable level of `path`'s template and returns the
/// validation messages, each prefixed with the file and level.
///
/// Only a COMPOSITION commits, so a template rooted in another class is outside
/// the committable claim (#3780).
fn committable_findings(path: &Path) -> Vec<String> {
    let name = file_name(path);
    if NOT_COMMITTABLE.iter().any(|(file, _)| *file == name) {
        return Vec::new();
    }
    let xml = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {name}: {e}"));
    // A template the conformant reader refuses is pinned by the reader's own
    // gates; there is no example to generate for it.
    let Ok(opt) = opt14::from_xml(&xml) else {
        return Vec::new();
    };
    let Ok(wt) = build_web_template(&opt) else {
        return Vec::new();
    };
    if wt.tree.rm_type != "COMPOSITION" {
        return Vec::new();
    }
    let mut out = Vec::new();
    for level in COMMITTABLE {
        let example = example_composition(&wt, level);
        for m in validate_composition(&example, &wt) {
            out.push(format!("{name}@{level:?}: {}: {}", m.path, m.message));
        }
    }
    out
}

/// Every template of the curated CKM journey pack and of the fixture
/// directories the Simplified Formats gates pair compositions with generates
/// `required` and `medium` examples the commit-path validation accepts.
#[test]
fn loaded_templates_generate_committable_examples() {
    let mut files = top_level_opts(&ckm_dir());
    for dir in opt_dirs() {
        files.extend(opt_files(&dir));
    }
    assert!(
        files.len() >= 16,
        "expected the template corpus, found {}",
        files.len()
    );
    let findings: Vec<String> = files.iter().flat_map(|p| committable_findings(p)).collect();
    assert!(
        findings.is_empty(),
        "generated examples refused by the commit-path validation:\n{}",
        findings.join("\n")
    );
}

/// The full vendored CKM template library (`corpus/templates/ckm/full/`)
/// generates committable examples too.
#[test]
fn full_ckm_library_generates_committable_examples() {
    let files = top_level_opts(&ckm_dir().join("full"));
    assert!(
        files.len() >= 300,
        "expected the full library, found {}",
        files.len()
    );
    let findings: Vec<String> = files.iter().flat_map(|p| committable_findings(p)).collect();
    assert!(
        findings.is_empty(),
        "generated examples refused by the commit-path validation:\n{}",
        findings.join("\n")
    );
}

// ── mandatory structural attributes without leaf content ─────────────────────

/// Depth-first search for the first web-template node `pred` accepts.
fn find_node<'a>(
    node: &'a WebTemplateNode,
    pred: &dyn Fn(&WebTemplateNode) -> bool,
) -> Option<&'a WebTemplateNode> {
    if pred(node) {
        return Some(node);
    }
    node.children.iter().find_map(|c| find_node(c, pred))
}

/// Mutable twin of [`find_node`], addressed by `aql_path`.
fn node_at_mut<'a>(
    node: &'a mut WebTemplateNode,
    aql_path: &str,
) -> Option<&'a mut WebTemplateNode> {
    if node.aql_path == aql_path {
        return Some(node);
    }
    node.children
        .iter_mut()
        .find_map(|c| node_at_mut(c, aql_path))
}

/// The first structural stub for `attr` anywhere in the tree, with the
/// `aql_path` of the node holding it (the owner itself, or the ancestor the
/// compactor re-homed it onto).
fn first_stub(wt: &WebTemplate, attr: &str) -> (String, WebTemplateStructuralStub) {
    let holder = find_node(&wt.tree, &|n| {
        n.structural_stubs.iter().any(|s| s.attr == attr)
    })
    .unwrap_or_else(|| panic!("a node holding a {attr} stub"));
    let stub = holder
        .structural_stubs
        .iter()
        .find(|s| s.attr == attr)
        .cloned()
        .unwrap_or_else(|| panic!("the {attr} stub"));
    (holder.aql_path.clone(), stub)
}

/// Makes the stub's attribute mandatory and strips every leaf beneath it, the
/// shape of a template that constrains a content-less mandatory structure.
fn make_content_less_mandatory(
    wt: &mut WebTemplate,
    holder_path: &str,
    stub: &WebTemplateStructuralStub,
) {
    let holder = node_at_mut(&mut wt.tree, holder_path)
        .unwrap_or_else(|| panic!("node {holder_path} in the web template"));
    holder
        .children
        .retain(|c| !c.aql_path.starts_with(stub.path.as_str()));
    holder.existence.push(WebTemplateExistence {
        min: 1,
        max: 1,
        path: stub.path.clone(),
    });
}

/// The segments of the stub's owner path: the attribute's absolute path
/// without the attribute itself.
fn owner_segments(stub: &WebTemplateStructuralStub) -> Vec<PathSegment> {
    let owner = stub
        .path
        .strip_suffix(stub.attr.as_str())
        .and_then(|p| p.strip_suffix('/'))
        .unwrap_or_else(|| panic!("owner of {}", stub.path));
    let path: RmPath = owner
        .parse()
        .unwrap_or_else(|e| panic!("owner path {owner}: {e}"));
    path.segments
}

/// Every canonical-JSON object `segments` reach from `node`, each step selecting
/// the values whose node id and name its predicate matches.
fn objects_at<'a>(node: &'a Value, segments: &[PathSegment], out: &mut Vec<&'a Value>) {
    let Some((segment, rest)) = segments.split_first() else {
        out.push(node);
        return;
    };
    match node.get(&segment.attribute) {
        Some(Value::Array(items)) => items
            .iter()
            .filter(|item| segment.predicate.matches(item))
            .for_each(|item| objects_at(item, rest, out)),
        Some(single) if segment.predicate.matches(single) => objects_at(single, rest, out),
        _ => {}
    }
}

/// Every canonical-JSON object whose `archetype_node_id` is `id`.
fn objects_with_id<'a>(v: &'a Value, id: &str, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(m) => {
            if m.get("archetype_node_id").and_then(Value::as_str) == Some(id) {
                out.push(v);
            }
            m.values().for_each(|c| objects_with_id(c, id, out));
        }
        Value::Array(a) => a.iter().for_each(|e| objects_with_id(e, id, out)),
        _ => {}
    }
}

/// The messages naming a missing mandatory `attr`.
fn missing_attribute(comp: &Value, wt: &WebTemplate, attr: &str) -> Vec<String> {
    let needle = format!("mandatory attribute '{attr}' is missing");
    validate_composition(comp, wt)
        .into_iter()
        .filter(|m| m.message.contains(&needle))
        .map(|m| format!("{}: {}", m.path, m.message))
        .collect()
}

/// Strips the leaf content under the first `attr` stub of the curated
/// vital-signs template, makes the attribute mandatory, and asserts that every
/// owner instance in the generated examples carries the constrained node.
fn assert_content_less_mandatory_is_materialised(attr: &str) {
    let mut wt = web_template(&ckm_dir().join("vital-signs.opt"));
    let (holder_path, stub) = first_stub(&wt, attr);
    make_content_less_mandatory(&mut wt, &holder_path, &stub);
    let segments = owner_segments(&stub);
    for level in [DetailLevel::Medium, DetailLevel::Complete] {
        let comp = example_composition(&wt, level);
        let mut owners = Vec::new();
        objects_at(&comp, &segments, &mut owners);
        assert!(
            !owners.is_empty(),
            "{level:?}: the example carries the owner of {}",
            stub.path
        );
        for owner in owners {
            let value = owner
                .get(attr)
                .unwrap_or_else(|| panic!("{level:?}: {} is present: {owner}", stub.path));
            assert_eq!(
                value.get("archetype_node_id").and_then(Value::as_str),
                Some(stub.node_id.as_str()),
                "{level:?}: {} carries the constrained node id",
                stub.path
            );
        }
        let missing = missing_attribute(&comp, &wt, attr);
        assert!(missing.is_empty(), "{level:?}: {missing:?}");
    }
}

/// An ENTRY whose `protocol` the template makes mandatory but fills with no
/// leaf content still carries its `protocol`, as the constrained `ITEM_TREE`.
#[test]
fn content_less_mandatory_protocol_is_materialised() {
    assert_content_less_mandatory_is_materialised("protocol");
}

/// An EVENT whose `state` the template makes mandatory but fills with no leaf
/// content still carries its `state`, as the constrained `ITEM_TREE` the
/// EVENT's structural stub records.
#[test]
fn content_less_mandatory_event_state_is_materialised() {
    assert_content_less_mandatory_is_materialised("state");
}

// ── ELEMENT value xor null_flavour ───────────────────────────────────────────

/// The vendored CKM template whose `at0004` ELEMENT constrains both `value`
/// (`DV_TEXT`) and `null_flavour` (`271|no information|`).
fn null_flavour_template() -> WebTemplate {
    web_template(&ckm_dir().join("full/odl-report-empower.opt"))
}

/// Every ELEMENT object in `v`.
fn elements<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(m) => {
            if m.get("_type").and_then(Value::as_str) == Some("ELEMENT") {
                out.push(v);
            }
            m.values().for_each(|c| elements(c, out));
        }
        Value::Array(a) => a.iter().for_each(|e| elements(e, out)),
        _ => {}
    }
}

/// The ELEMENTs of `comp` that break `Inv_null_flavour_indicated`.
fn null_flavour_violations(comp: &Value) -> Vec<String> {
    let mut found = Vec::new();
    elements(comp, &mut found);
    found
        .into_iter()
        .filter(|e| e.get("value").is_some() == e.get("null_flavour").is_some())
        .map(ToString::to_string)
        .collect()
}

/// At every level an ELEMENT whose template constrains a `null_flavour` beside
/// its `value` carries the value alone.
#[test]
fn element_with_a_value_carries_no_null_flavour() {
    let wt = null_flavour_template();
    for level in [
        DetailLevel::Required,
        DetailLevel::Medium,
        DetailLevel::Complete,
    ] {
        let comp = example_composition(&wt, level);
        let violations = null_flavour_violations(&comp);
        assert!(violations.is_empty(), "{level:?}: {violations:?}");
        let invariant: Vec<String> = validate_composition(&comp, &wt)
            .into_iter()
            .filter(|m| m.message.contains("Inv_null_flavour_indicated"))
            .map(|m| format!("{}: {}", m.path, m.message))
            .collect();
        assert!(invariant.is_empty(), "{level:?}: {invariant:?}");
    }
    let medium = example_composition(&wt, DetailLevel::Medium);
    let mut at0004 = Vec::new();
    objects_with_id(&medium, "at0004", &mut at0004);
    assert!(
        at0004
            .iter()
            .any(|e| e.get("value").is_some() && e.get("null_flavour").is_none()),
        "the medium example carries the at0004 ELEMENT with its value: {at0004:?}"
    );
}

/// A mandatory ELEMENT whose value the example cannot fill carries its
/// `null_flavour` instead, the one shape the invariant requires it on.
#[test]
fn value_less_mandatory_element_carries_its_null_flavour() {
    let mut wt = null_flavour_template();
    let element_path = find_node(&wt.tree, &|n| {
        n.rm_type == "ELEMENT"
            && n.children
                .iter()
                .any(|c| c.aql_path.ends_with("/null_flavour"))
    })
    .unwrap_or_else(|| panic!("an ELEMENT with a null_flavour child"))
    .aql_path
    .clone();
    let element =
        node_at_mut(&mut wt.tree, &element_path).unwrap_or_else(|| panic!("the ELEMENT node"));
    element.children.retain(|c| !c.aql_path.ends_with("/value"));
    element.min = Some(1);
    let node_id = element
        .node_id
        .clone()
        .unwrap_or_else(|| panic!("the ELEMENT node id"));

    let comp = example_composition(&wt, DetailLevel::Medium);
    let mut found = Vec::new();
    objects_with_id(&comp, &node_id, &mut found);
    assert!(
        found
            .iter()
            .any(|e| e.get("value").is_none() && e.get("null_flavour").is_some()),
        "the value-less ELEMENT carries its null_flavour: {found:?}"
    );
    let violations = null_flavour_violations(&comp);
    assert!(violations.is_empty(), "{violations:?}");
}
