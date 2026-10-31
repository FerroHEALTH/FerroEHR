// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The archetype-conformance pass over an ADL 2 operational template, driven
//! by the Veredictum catalogue's ADL 2 content cases (`corpus/veredictum/
//! adl2-content/`, vendored by `scripts/vendor/veredictum-adl2-content.sh`).
//!
//! Each case commits one composition against one template and names the
//! status: `201` where the template admits it, `422` (ITS-REST
//! `responses/422.yaml`: "the underlying template ... is not validating the
//! supplied resource") where it breaks one constraint. The compositions record
//! each inlined archetype by its complete identifier (BASE
//! `artefact_identification` `master07-referencing.adoc` §Reconstitutability),
//! and the pass reads them under the template's nodes. One test per constraint
//! kind, citing AOM2 `master04.5-constraint_model-class_definitions.adoc`.

#![expect(
    clippy::panic,
    clippy::expect_used,
    reason = "test assertions and fixture plumbing outside #[test] fns, which the \
              clippy.toml allow-*-in-tests scoping does not reach"
)]
#![expect(
    clippy::doc_markdown,
    reason = "the docs quote AOM2 section titles, which name classes as prose"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use openehr_adl::assemble::parse_artefact;
use openehr_adl::parse::Dialect;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
use openehr_sdt::flat::convert::{
    composition_from_flat, composition_from_structured, composition_to_flat,
    composition_to_structured,
};
use openehr_sdt::flat::validation::validate_archetype_conformance;
use openehr_sdt::flat::webtemplate::builder_v2_4::build_web_template_v2_4;
use openehr_sdt::flat::webtemplate::model::WebTemplate;
use openehr_sdt::rm_instance::{ValidationKind, ValidationMessage, validate_rm_and_terminology};
use serde_json::Value;

/// One commit step of a catalogue workflow.
struct Case {
    workflow: String,
    template: String,
    status: u16,
    body: Value,
}

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/veredictum/adl2-content")
}

fn yaml(path: &PathBuf) -> Value {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_norway::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn workflows(doc: &Value) -> &[Value] {
    doc.pointer("/workflows")
        .and_then(Value::as_array)
        .expect("workflows")
        .as_slice()
}

/// Every setup template, built into its Web Template and keyed by the upload
/// workflow a case names in `dependsOn`.
fn templates() -> BTreeMap<String, WebTemplate> {
    let setup = yaml(&corpus().join("_setup/content_templates_adl2.arazzo.yaml"));
    workflows(&setup)
        .iter()
        .map(|wf| {
            let id = wf
                .pointer("/workflowId")
                .and_then(Value::as_str)
                .expect("workflowId")
                .to_owned();
            let source = wf
                .pointer("/steps/0/requestBody/payload")
                .and_then(Value::as_str)
                .expect("ADL 2 payload");
            let archetype = parse_artefact(source, Dialect::Adl2)
                .unwrap_or_else(|e| panic!("parse {id}: {e:?}"));
            let Archetype::AuthoredArchetype(authored) = archetype else {
                panic!("{id} is not an authored archetype");
            };
            let AuthoredArchetype::OperationalTemplate(opt) = *authored else {
                panic!("{id} is not an operational template");
            };
            let wt = build_web_template_v2_4(&opt).unwrap_or_else(|e| panic!("build {id}: {e}"));
            (id, wt)
        })
        .collect()
}

/// Every commit step of every `content/adl2_*` document.
fn cases() -> Vec<Case> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(corpus().join("content"))
        .expect("content dir")
        .flatten()
        .map(|e| e.path())
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for path in &paths {
        let doc = yaml(path);
        for wf in workflows(&doc) {
            let template = wf
                .pointer("/dependsOn/0")
                .and_then(Value::as_str)
                .and_then(|d| d.rsplit('.').next())
                .expect("dependsOn")
                .to_owned();
            let Some(commit) = wf
                .pointer("/steps")
                .and_then(Value::as_array)
                .and_then(|s| {
                    s.iter()
                        .find(|s| s.get("stepId") == Some(&Value::from("commit")))
                })
            else {
                continue;
            };
            let condition = commit
                .pointer("/successCriteria/0/condition")
                .and_then(Value::as_str)
                .expect("status condition");
            let status = condition
                .strip_prefix("$statusCode == ")
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(|| panic!("status condition {condition}"));
            out.push(Case {
                workflow: wf
                    .pointer("/workflowId")
                    .and_then(Value::as_str)
                    .expect("workflowId")
                    .to_owned(),
                template,
                status,
                body: commit
                    .pointer("/requestBody/payload")
                    .cloned()
                    .expect("commit body"),
            });
        }
    }
    out
}

/// The template pass's findings for one case.
fn template_findings(
    case: &Case,
    templates: &BTreeMap<String, WebTemplate>,
) -> Vec<ValidationMessage> {
    let wt = templates
        .get(&case.template)
        .unwrap_or_else(|| panic!("{}: no template {}", case.workflow, case.template));
    validate_archetype_conformance(&case.body, wt)
}

/// Every `422` case whose workflow id ends in one of `kinds` is refused by the
/// template pass, and at least one such case exists per suffix.
fn assert_refused(kinds: &[&str]) {
    let templates = templates();
    let cases = cases();
    for kind in kinds {
        let rows: Vec<&Case> = cases
            .iter()
            .filter(|c| c.status == 422 && c.workflow.ends_with(kind))
            .collect();
        assert!(!rows.is_empty(), "no 422 case ends in {kind}");
        for case in rows {
            assert!(
                validate_rm_and_terminology(&case.body).is_empty(),
                "{}: the RM pass refuses it, so the template is not what is tested",
                case.workflow
            );
            assert!(
                !template_findings(case, &templates).is_empty(),
                "{}: a composition breaking the template's {kind} constraint was admitted",
                case.workflow
            );
        }
    }
}

/// The suffix of every constraint kind the `422` cases break.
const KINDS: [&str; 13] = [
    "-c_integer_range",
    "-c_integer_list",
    "-c_real_range",
    "-c_real_list",
    "-c_string_list",
    "-c_string_list_units",
    "-c_string_pattern",
    "-c_boolean_true_valid",
    "-c_boolean_false_valid",
    "-c_dv_quantity_tuple",
    "-c_dv_ordinal_tuple",
    "-c_terminology_code",
    "-c_multiple_attribute_cardinality",
];

#[test]
fn every_refusal_case_belongs_to_a_tested_constraint_kind() {
    let refusals: Vec<String> = cases()
        .into_iter()
        .filter(|c| c.status == 422)
        .map(|c| c.workflow)
        .collect();
    assert_eq!(refusals.len(), 30, "the 30 refusal rows of the catalogue");
    for workflow in &refusals {
        assert!(
            KINDS.iter().any(|k| workflow.ends_with(k)),
            "{workflow} breaks a constraint kind no test here covers"
        );
    }
}

/// AOM2 `master04.5` §"Conformance semantics: C_ORDERED": a `C_INTEGER` admits
/// a value inside one of its intervals, the point intervals of a list included.
#[test]
fn c_integer_range_and_list_are_enforced() {
    assert_refused(&["-c_integer_range", "-c_integer_list"]);
}

/// AOM2 `master04.5` §"Conformance semantics: C_ORDERED", for `C_REAL`.
#[test]
fn c_real_range_and_list_are_enforced() {
    assert_refused(&["-c_real_range", "-c_real_list"]);
}

/// AOM2 `master04.5` §"Conformance semantics: C_STRING": a list of literals or
/// a regular expression.
#[test]
fn c_string_list_and_pattern_are_enforced() {
    assert_refused(&[
        "-c_string_list",
        "-c_string_list_units",
        "-c_string_pattern",
    ]);
}

/// AOM2 `master04.5` §"Conformance semantics: C_BOOLEAN".
#[test]
fn c_boolean_is_enforced() {
    assert_refused(&["-c_boolean_true_valid", "-c_boolean_false_valid"]);
}

/// AOM2 `master04.5` §"Conformance semantics: C_SECOND_ORDER" (the
/// `C_ATTRIBUTE_TUPLE` of `DV_QUANTITY` and `DV_ORDINAL`).
#[test]
fn tuple_constraints_are_enforced() {
    assert_refused(&["-c_dv_quantity_tuple", "-c_dv_ordinal_tuple"]);
}

/// AOM2 `master04.5` §"Conformance semantics: C_TERMINOLOGY_NODE": the code
/// belongs to the value set the ac-code names.
#[test]
fn c_terminology_code_value_set_is_enforced() {
    assert_refused(&["-c_terminology_code"]);
}

/// AOM2 `master04.5` §"Conformance Semantics: C_ATTRIBUTE": the cardinality of
/// a container attribute.
#[test]
fn attribute_cardinality_is_enforced() {
    assert_refused(&["-c_multiple_attribute_cardinality"]);
}

/// Every `201` case passes both passes, so no refusal above is a blanket one.
#[test]
fn every_admitted_composition_passes() {
    let templates = templates();
    let admitted: Vec<Case> = cases().into_iter().filter(|c| c.status == 201).collect();
    assert!(!admitted.is_empty(), "the catalogue's 201 cases");
    for case in &admitted {
        let mut findings = validate_rm_and_terminology(&case.body);
        findings.extend(template_findings(case, &templates));
        assert!(
            findings.is_empty(),
            "{}: admitted by the template, refused with {findings:?}",
            case.workflow
        );
    }
}

/// The accepted `DV_COUNT` case with its inlined OBSERVATION retargeted by
/// `edit`.
fn count_case_with(edit: impl FnOnce(&mut Value)) -> (Value, WebTemplate) {
    let case = cases()
        .into_iter()
        .find(|c| c.workflow == "adl2-dv_count-validate_range-accepted")
        .expect("the DV_COUNT range control case");
    let wt = templates()
        .remove(&case.template)
        .expect("the DV_COUNT range template");
    let mut body = case.body;
    edit(
        body.pointer_mut("/content/0")
            .expect("the inlined OBSERVATION"),
    );
    (body, wt)
}

/// An archetype root the template did not inline is refused: an OPT2 inlines
/// every slot filler (AM OPT2 `master03-opt_raw.adoc` §Flattening), so a
/// version of the archetype other than the one the template names is a node
/// the template does not define.
#[test]
fn an_archetype_root_the_template_does_not_inline_is_refused() {
    let (body, wt) = count_case_with(|obs| {
        obs["archetype_node_id"] = Value::from("openEHR-EHR-OBSERVATION.cnf_minimal.v1.2.0");
    });
    let findings = validate_archetype_conformance(&body, &wt);
    assert!(
        findings
            .iter()
            .any(|m| m.kind == ValidationKind::Unexpected),
        "an OBSERVATION the template does not inline was admitted: {findings:?}"
    );
}

/// An interior node whose id-code the template does not define is refused
/// (AOM2 `master04.5` §"Conformance Semantics: C_ATTRIBUTE").
#[test]
fn an_interior_node_the_template_does_not_define_is_refused() {
    let (body, wt) = count_case_with(|obs| {
        obs["data"]["events"][0]["data"]["items"][0]["archetype_node_id"] = Value::from("id9");
    });
    let findings = validate_archetype_conformance(&body, &wt);
    assert!(
        findings
            .iter()
            .any(|m| m.kind == ValidationKind::Unexpected),
        "an ELEMENT the template does not define was admitted: {findings:?}"
    );
}

/// The interface form the Web Template names an inlined archetype by stays
/// admitted beside the complete identifier.
#[test]
fn the_interface_archetype_id_is_still_matched() {
    let (body, wt) = count_case_with(|obs| {
        obs["archetype_node_id"] = Value::from("openEHR-EHR-OBSERVATION.cnf_minimal.v1");
        obs["data"]["events"][0]["data"]["items"][0]["value"]["magnitude"] = Value::from(25);
    });
    let findings = validate_archetype_conformance(&body, &wt);
    assert!(
        findings
            .iter()
            .any(|m| m.kind == ValidationKind::RangeError),
        "the out-of-range magnitude under the interface id was not refused: {findings:?}"
    );
}

/// `body` with every archetype root it records by a full reference retargeted
/// to the interface id of its template node, the id the Web Template names it
/// by.
fn under_interface_ids(body: &Value, wt: &WebTemplate) -> Value {
    fn retarget(value: &mut Value, refs: &BTreeMap<String, String>) {
        match value {
            Value::Object(o) => {
                if let Some(Value::String(id)) = o.get_mut("archetype_node_id")
                    && let Some(interface) = refs.get(id.as_str())
                {
                    interface.clone_into(id);
                }
                o.values_mut().for_each(|v| retarget(v, refs));
            }
            Value::Array(items) => items.iter_mut().for_each(|v| retarget(v, refs)),
            _ => {}
        }
    }
    let refs = wt.archetype_refs.as_ref().expect("an OPT2 Web Template");
    let mut body = body.clone();
    retarget(&mut body, refs);
    body
}

/// Every ELEMENT in the `content` of `composition`, keyed by the chain of
/// `archetype_node_id`s down to it, with its `name` and `value`.
fn elements(composition: &Value) -> BTreeMap<String, (Value, Value)> {
    fn collect(value: &Value, at: &str, out: &mut BTreeMap<String, (Value, Value)>) {
        match value {
            Value::Object(o) => {
                let at = match o.get("archetype_node_id").and_then(Value::as_str) {
                    Some(id) => format!("{at}/{id}"),
                    None => at.to_owned(),
                };
                if o.get("_type").and_then(Value::as_str) == Some("ELEMENT") {
                    let field = |k: &str| o.get(k).cloned().unwrap_or(Value::Null);
                    out.insert(at, (field("name"), field("value")));
                    return;
                }
                o.values().for_each(|v| collect(v, &at, out));
            }
            Value::Array(items) => items
                .iter()
                .enumerate()
                .for_each(|(i, v)| collect(v, &format!("{at}:{i}"), out)),
            _ => {}
        }
    }
    let mut out = BTreeMap::new();
    if let Some(content) = composition.get("content") {
        collect(content, "", &mut out);
    }
    out
}

/// ITS-REST `simplified_formats/master04-basic_concepts.adoc` §Format variants:
/// the FLAT and STRUCTURED read of a composition recording its inlined
/// archetype roots by their complete identifier is the read of the same
/// content under the interface ids the Web Template names them by.
#[test]
fn the_simplified_read_matches_a_full_archetype_reference_to_its_node() {
    let templates = templates();
    let admitted: Vec<Case> = cases().into_iter().filter(|c| c.status == 201).collect();
    assert!(!admitted.is_empty(), "the catalogue's 201 cases");
    for case in &admitted {
        let wt = templates.get(&case.template).expect("the case's template");
        let interface = under_interface_ids(&case.body, wt);
        assert_ne!(
            interface, case.body,
            "{}: records no full reference",
            case.workflow
        );
        let flat = composition_to_flat(&case.body, wt).expect("FLAT read");
        assert_eq!(
            flat,
            composition_to_flat(&interface, wt).expect("FLAT read under interface ids"),
            "{}: the FLAT read differs",
            case.workflow
        );
        assert_eq!(
            composition_to_structured(&case.body, wt).expect("STRUCTURED read"),
            composition_to_structured(&interface, wt).expect("STRUCTURED under interface ids"),
            "{}: the STRUCTURED read differs",
            case.workflow
        );
    }
}

/// The FLAT and STRUCTURED read of the accepted `DV_COUNT` composition, whose
/// OBSERVATION is recorded as `…cnf_minimal.v1.0.0`, carries its ELEMENT: the
/// composition built back from either form holds it unchanged.
#[test]
fn the_simplified_read_keeps_the_content_below_a_full_archetype_reference() {
    let (body, wt) = count_case_with(|obs| {
        assert_eq!(
            obs["archetype_node_id"],
            Value::from("openEHR-EHR-OBSERVATION.cnf_minimal.v1.0.0")
        );
    });
    let stored = elements(&under_interface_ids(&body, &wt));
    assert_eq!(stored.len(), 1, "the one DV_COUNT ELEMENT");
    let flat = composition_to_flat(&body, &wt).expect("FLAT read");
    let structured = composition_to_structured(&body, &wt).expect("STRUCTURED read");
    let now = "2026-01-01T10:00:00Z";
    let from_flat = composition_from_flat(&flat, &wt, now).expect("FLAT build");
    let from_structured =
        composition_from_structured(&structured, &wt, now).expect("STRUCTURED build");
    assert_eq!(elements(&from_flat), stored, "an ELEMENT is lost in FLAT");
    assert_eq!(
        elements(&from_structured),
        stored,
        "an ELEMENT is lost in STRUCTURED"
    );
}
