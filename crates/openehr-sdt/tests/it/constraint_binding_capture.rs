// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![expect(clippy::panic, reason = "test assertions/diagnostics")]
//! Archetype **constraint bindings** — the OPT → `WebTemplate` capture and the
//! instance-side check collection.
//!
//! BASE `docs/architecture_overview/master12-terminology.adoc` §"Binding
//! Terminology Value-sets to Archetypes": where a terminology is the
//! appropriate source of values, "an internal code is defined, in this case an
//! 'ac' code ('ac' = archetype constraint), and this is bound to queries to one
//! or more external terminologies, whose result would be a (possibly
//! structured) value set from that terminology". The query "is defined within a
//! 'terminology query server'", so this crate cannot resolve it — it captures
//! the binding
//! ([`WebTemplateConstraintBinding`](openehr_sdt::flat::webtemplate::model::WebTemplateConstraintBinding))
//! and surfaces the questions
//! ([`collect_constraint_binding_checks`](openehr_sdt::flat::validation::collect_constraint_binding_checks))
//! for a caller that owns one.
//!
//! The constraint-model side is AOM 1.4
//! `AM/docs/AOM1.4/master04-constraint_model_package.adoc` §Reference Objects
//! (a `CONSTRAINT_REF` is "a proxy for a set of constraints … expressed in the
//! binding of the constraint reference (e.g. 'ac0004') to a query"), and the
//! ac-code → query map is the ontology's `constraint_bindings`
//! (`AM/docs/ADL1.4/master08-adl.adoc` §`Constraint_bindings`).

use std::path::PathBuf;

use openehr_its::opt14;
use openehr_sdt::flat::validation::collect_constraint_binding_checks;
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::{WebTemplate, WebTemplateNode};
use serde_json::json;

const BOUND_VS: &str = "http://terminology.example/ValueSet/blood-group";
const SNOMED: &str = "http://snomed.info/sct";

/// The vendored corpus template whose `DV_CODED_TEXT.defining_code` is a
/// `CONSTRAINT_REF` to `ac0001`. It deliberately carries NO binding (its
/// purpose is the *unbound* case), so the bound case is produced by adding the
/// ontology `constraint_bindings` element the bound case is defined by — the
/// only difference between the two.
fn corpus_opt() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/templates/dt_coded_text_constraint_ref.opt");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The corpus template with an `ontology` binding `ac0001` to a
/// terminology-query URI (`ARCHETYPE_ONTOLOGY.constraint_bindings`, an
/// `archetype_id`-scoped `ConstraintBindingSet`).
fn bound_opt() -> String {
    let ontology = format!(
        r#"<ontology archetype_id="openEHR-EHR-OBSERVATION.minimal.v1">
             <constraint_bindings terminology="SNOMED-CT">
               <items code="ac0001"><value>{BOUND_VS}</value></items>
             </constraint_bindings>
           </ontology>"#
    );
    let xml = corpus_opt();
    assert!(
        xml.contains("</definition>"),
        "the corpus template shape changed; the ontology injection point is gone"
    );
    xml.replace("</definition>", &format!("</definition>\n  {ontology}"))
}

fn build(xml: &str) -> WebTemplate {
    let opt = opt14::from_xml(xml).unwrap_or_else(|e| panic!("opt14 parse: {e}"));
    build_web_template(&opt).unwrap_or_else(|e| panic!("build_web_template: {e}"))
}

/// Depth-first search for the first node carrying a constraint binding.
fn find_bound(n: &WebTemplateNode) -> Option<&WebTemplateNode> {
    if !n.constraint_bindings.is_empty() {
        return Some(n);
    }
    n.children.iter().find_map(find_bound)
}

/// A bound `CONSTRAINT_REF` is captured onto the coded leaf, naming the RM
/// attribute it constrains, the ac-code, the binding terminology, and the
/// query URI.
#[test]
fn a_bound_constraint_ref_is_captured_on_the_coded_leaf() {
    let wt = build(&bound_opt());
    let node = find_bound(&wt.tree).expect("the bound coded leaf carries its constraint binding");
    assert_eq!(node.constraint_bindings.len(), 1);
    let binding = &node.constraint_bindings[0];
    assert_eq!(binding.ac_code, "ac0001");
    assert_eq!(binding.terminology, "SNOMED-CT");
    assert_eq!(binding.query_uri, BOUND_VS);
    assert_eq!(
        binding.attr, "defining_code",
        "the binding names the coded RM attribute it constrains"
    );
}

/// An `ac`-code with no binding captures nothing: AOM 1.4 leaves the
/// no-binding case undefined and no local fallback exists, so it constrains
/// nothing enforceable at commit time.
#[test]
fn an_unbound_constraint_ref_captures_nothing() {
    let wt = build(&corpus_opt());
    assert!(
        find_bound(&wt.tree).is_none(),
        "an unbound CONSTRAINT_REF must not raise a terminology question"
    );
}

/// The instance walk raises exactly one question per bound coded value
/// present, carrying both the binding's terminology (the routing key) and the
/// instance's own code + terminology (the membership question).
#[test]
fn a_bound_coded_value_raises_one_check() {
    let wt = build(&bound_opt());
    let leaf = find_bound(&wt.tree).expect("bound leaf");
    let composition = composition_with_code(&leaf.aql_path, "278149003");

    let checks = collect_constraint_binding_checks(&composition, &wt);
    assert_eq!(checks.len(), 1, "got {checks:?}");
    let check = &checks[0];
    assert_eq!(check.ac_code, "ac0001");
    assert_eq!(check.binding_terminology, "SNOMED-CT");
    assert_eq!(check.query_uri, BOUND_VS);
    assert_eq!(check.instance_terminology, SNOMED);
    assert_eq!(check.instance_code, "278149003");
    assert_eq!(check.path, leaf.aql_path);
}

/// An unbound template raises no question at all, so a deployment with no
/// bindings pays nothing.
#[test]
fn an_unbound_template_raises_no_check() {
    // The two templates share one definition and differ only in the ontology
    // binding, so the bound build supplies the leaf path and the unbound build
    // is the subject: the SAME composition raises a check against one and none
    // against the other.
    let bound = build(&bound_opt());
    let leaf = find_bound(&bound.tree).expect("bound leaf");
    let composition = composition_with_code(&leaf.aql_path, "278149003");
    assert_eq!(
        collect_constraint_binding_checks(&composition, &bound).len(),
        1,
        "control: the bound template does raise a check for this composition"
    );

    let unbound = build(&corpus_opt());
    assert!(collect_constraint_binding_checks(&composition, &unbound).is_empty());
}

/// A composition that leaves the bound node uncoded raises no question — the
/// binding constrains a value that is not there.
#[test]
fn an_absent_coded_value_raises_no_check() {
    let wt = build(&bound_opt());
    let composition = json!({
        "_type": "COMPOSITION",
        "archetype_node_id": "openEHR-EHR-COMPOSITION.minimal.v1",
        "name": { "_type": "DV_TEXT", "value": "minimal" }
    });
    assert!(collect_constraint_binding_checks(&composition, &wt).is_empty());
}

/// The two COMMITTED bound corpus templates carry their binding in the file,
/// not in a test-time injection: `dt_coded_text_binding_sct.opt` binds ac0001
/// into the terminology namespace a conformance deployment routes to a
/// reachable terminology server, and `dt_coded_text_binding_tsdown.opt` binds
/// it into one routed to a server declared unreachable. They are the payload
/// ground of the commit-time binding cases, so a silent change to either file
/// — a lost `ontology` element, a renamed terminology, a mistyped query URI —
/// must fail here rather than as a puzzling conformance row.
#[test]
fn the_committed_bound_corpus_templates_carry_their_bindings() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/templates");
    let expected = [
        (
            "dt_coded_text_binding_sct.opt",
            "cnf-sct-shaped",
            "http://cnf.example.test/fhir/ValueSet/sct-shaped-disorders",
        ),
        (
            "dt_coded_text_binding_tsdown.opt",
            "cnf-ts-down",
            "http://cnf.example.test/fhir/ValueSet/ts-down-set",
        ),
    ];
    for (file, terminology, query_uri) in expected {
        let path = corpus.join(file);
        let xml = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let wt = build(&xml);
        let node = find_bound(&wt.tree)
            .unwrap_or_else(|| panic!("{file}: no node carries a constraint binding"));
        assert_eq!(node.constraint_bindings.len(), 1, "{file}");
        let binding = &node.constraint_bindings[0];
        assert_eq!(binding.ac_code, "ac0001", "{file}");
        assert_eq!(binding.terminology, terminology, "{file}");
        assert_eq!(binding.query_uri, query_uri, "{file}");
        assert_eq!(binding.attr, "defining_code", "{file}");

        // And the instance walk raises exactly the membership question the
        // commit-time cases rest on.
        let composition = composition_with_code(&node.aql_path, "1000002");
        let checks = collect_constraint_binding_checks(&composition, &wt);
        assert_eq!(checks.len(), 1, "{file}: got {checks:?}");
        assert_eq!(checks[0].query_uri, query_uri, "{file}");
        assert_eq!(checks[0].instance_code, "1000002", "{file}");
    }
}

/// A minimal COMPOSITION whose single coded leaf sits at `aql_path` carrying
/// `code`. The path is the template's own `aqlPath`, so the walk matches it.
fn composition_with_code(aql_path: &str, code: &str) -> serde_json::Value {
    // The corpus template's shape:
    // COMPOSITION[at0000] / content[at0000 OBSERVATION] / data[at0001 HISTORY]
    //   / events[at0002 EVENT] / data[at0003 ITEM_TREE] / items[at0004 ELEMENT]
    //   / value (DV_CODED_TEXT).
    assert!(
        aql_path.ends_with("/value"),
        "the coded leaf is the ELEMENT value; got {aql_path}"
    );
    json!({
        "_type": "COMPOSITION",
        "archetype_node_id": "openEHR-EHR-COMPOSITION.minimal.v1",
        "name": { "_type": "DV_TEXT", "value": "minimal" },
        "content": [{
            "_type": "OBSERVATION",
            "archetype_node_id": "openEHR-EHR-OBSERVATION.minimal.v1",
            "name": { "_type": "DV_TEXT", "value": "Minimal" },
            "data": {
                "_type": "HISTORY",
                "archetype_node_id": "at0001",
                "name": { "_type": "DV_TEXT", "value": "Event Series" },
                "events": [{
                    "_type": "POINT_EVENT",
                    "archetype_node_id": "at0002",
                    "name": { "_type": "DV_TEXT", "value": "Any event" },
                    "data": {
                        "_type": "ITEM_TREE",
                        "archetype_node_id": "at0003",
                        "name": { "_type": "DV_TEXT", "value": "Tree" },
                        "items": [{
                            "_type": "ELEMENT",
                            "archetype_node_id": "at0004",
                            "name": { "_type": "DV_TEXT", "value": "value" },
                            "value": {
                                "_type": "DV_CODED_TEXT",
                                "value": "A positive",
                                "defining_code": {
                                    "_type": "CODE_PHRASE",
                                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": SNOMED },
                                    "code_string": code
                                }
                            }
                        }]
                    }
                }]
            }
        }]
    })
}
