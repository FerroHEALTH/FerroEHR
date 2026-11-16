// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The public slot admission decision, `WebTemplateArchetypeSlot::admits`
//! (AOM 1.4 `ARCHETYPE_SLOT`, `includes` and `excludes`), on hand-made slots
//! and on the slot a real template leaves open.

use openehr_its::opt14;
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::constraint::WebTemplateArchetypeSlot;
use openehr_sdt::flat::webtemplate::model::WebTemplateNode;

fn slot(rm_type: &str, includes: &[&str], excludes: &[&str]) -> WebTemplateArchetypeSlot {
    WebTemplateArchetypeSlot {
        rm_type: rm_type.to_owned(),
        min: 0,
        max: -1,
        includes: includes.iter().map(|&p| p.to_owned()).collect(),
        excludes: excludes.iter().map(|&p| p.to_owned()).collect(),
    }
}

#[test]
fn an_include_pattern_admits_the_whole_id_only() {
    let names = slot(
        "CLUSTER",
        &[r"openEHR-EHR-CLUSTER\.structured_name(-[a-zA-Z0-9_]+)*\.v1"],
        &[],
    );
    assert!(names.admits("CLUSTER", "openEHR-EHR-CLUSTER.structured_name.v1"));
    assert!(names.admits("CLUSTER", "openEHR-EHR-CLUSTER.structured_name-de.v1"));
    assert!(!names.admits("CLUSTER", "openEHR-EHR-CLUSTER.structured_name.v10"));
    assert!(!names.admits("CLUSTER", "openEHR-EHR-CLUSTER.address.v1"));
}

#[test]
fn slashes_around_a_pattern_are_dropped() {
    let address = slot("CLUSTER", &[r"/openEHR-EHR-CLUSTER\.address\.v1/"], &[]);
    assert!(address.admits("CLUSTER", "openEHR-EHR-CLUSTER.address.v1"));
}

#[test]
fn an_empty_include_list_admits_every_id_the_excludes_leave() {
    let open = slot("CLUSTER", &[], &[r"openEHR-EHR-CLUSTER\.device\.v1"]);
    assert!(open.admits("CLUSTER", "openEHR-EHR-CLUSTER.address.v1"));
    assert!(!open.admits("CLUSTER", "openEHR-EHR-CLUSTER.device.v1"));
}

#[test]
fn a_match_all_exclude_beside_includes_is_the_closed_slot_idiom() {
    for all in [".*", ".+", " .* "] {
        let closed = slot("CLUSTER", &[r"openEHR-EHR-CLUSTER\.address\.v1"], &[all]);
        assert!(closed.admits("CLUSTER", "openEHR-EHR-CLUSTER.address.v1"));
        assert!(!closed.admits("CLUSTER", "openEHR-EHR-CLUSTER.person.v1"));
    }
    let shut = slot("CLUSTER", &[], &[".*"]);
    assert!(
        !shut.admits("CLUSTER", "openEHR-EHR-CLUSTER.address.v1"),
        "a match-all exclude with no includes admits nothing"
    );
}

#[test]
fn the_filler_type_conforms_to_the_slot_type() {
    let entries = slot("ENTRY", &[".*"], &[]);
    assert!(entries.admits("OBSERVATION", "openEHR-EHR-OBSERVATION.blood_pressure.v2"));
    assert!(!entries.admits("CLUSTER", "openEHR-EHR-CLUSTER.device.v1"));
    assert!(!entries.admits("NOT_AN_RM_CLASS", "openEHR-EHR-OBSERVATION.x.v1"));
    let any_type = slot("", &[".*"], &[]);
    assert!(any_type.admits("CLUSTER", "openEHR-EHR-CLUSTER.device.v1"));
}

#[test]
fn a_pattern_no_engine_compiles_counts_as_a_match() {
    assert!(slot("CLUSTER", &["("], &[]).admits("CLUSTER", "openEHR-EHR-CLUSTER.address.v1"));
    assert!(!slot("CLUSTER", &[], &["("]).admits("CLUSTER", "openEHR-EHR-CLUSTER.address.v1"));
}

/// The IDCR problem list keeps the `at0002` slot of its section open to the
/// problem/diagnosis family only.
#[test]
fn the_idcr_problem_slot_admits_its_family_only() {
    let path = crate::common::service_corpus_dir().join("knowledge/IDCR Problem List.v1.opt");
    let xml = std::fs::read_to_string(&path).expect("read the IDCR corpus OPT");
    let opt = opt14::from_xml(&xml).expect("the IDCR OPT parses");
    let wt = build_web_template(&opt).expect("build the IDCR web template");
    let pattern = r"openEHR-EHR-EVALUATION\.problem_diagnosis(-[a-zA-Z0-9_]+)*\.v1";
    let problem = find_slot(&wt.tree, pattern).expect("the problem/diagnosis slot");
    assert!(problem.admits("EVALUATION", "openEHR-EHR-EVALUATION.problem_diagnosis.v1"));
    assert!(problem.admits(
        "EVALUATION",
        "openEHR-EHR-EVALUATION.problem_diagnosis-xx.v1"
    ));
    assert!(!problem.admits(
        "EVALUATION",
        "openEHR-EHR-EVALUATION.adverse_reaction_risk.v1"
    ));
    assert!(!problem.admits("CLUSTER", "openEHR-EHR-EVALUATION.problem_diagnosis.v1"));
}

fn find_slot<'a>(node: &'a WebTemplateNode, include: &str) -> Option<&'a WebTemplateArchetypeSlot> {
    node.closed_attributes
        .iter()
        .flat_map(|attribute| &attribute.slots)
        .find(|slot| slot.includes.iter().any(|p| p == include))
        .or_else(|| {
            node.children
                .iter()
                .find_map(|child| find_slot(child, include))
        })
}
