// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Web Template of a template over an ADL 1.4 specialised archetype holds
//! one `/context` (#4979).
//!
//! The CKM `report-result.v1` COMPOSITION specialises `report.v1` in ADL 1.4,
//! whose archetypes are authored in flat form
//! (`docs/specs/openehr/AM/docs/ADL2/master07.02-adl_conventions.adoc`
//! §File-naming Convention), so the operational template carries the child's
//! `context` once and a consumer naming `/context` meets one node.

#![expect(
    clippy::expect_used,
    reason = "integration-test fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::opt::create_opt;
use openehr_adl::parse::Dialect;
use openehr_sdt::flat::webtemplate::builder_v2_4::build_web_template_v2_4;
use openehr_sdt::flat::webtemplate::model::{WebTemplate, WebTemplateNode};

/// The CKM ADL 1.4 sources: `report`, its specialisation `report-result`, and
/// the `laboratory_test_result` OBSERVATION the template holds.
const SOURCES: [&str; 3] = [
    include_str!("../../../../corpus/archetypes/ckm/adl14/openEHR-EHR-COMPOSITION.report.v1.adl"),
    include_str!(
        "../../../../corpus/archetypes/ckm/adl14/openEHR-EHR-COMPOSITION.report-result.v1.adl"
    ),
    include_str!(
        "../../../../corpus/archetypes/ckm/adl14/openEHR-EHR-OBSERVATION.laboratory_test_result.v1.adl"
    ),
];

/// A template over the unconverted 1.4 `report-result`, holding a laboratory
/// test result.
const TEMPLATE: &str = "\
template (adl_version=2.4.0; rm_release=1.1.0)
    openEHR-EHR-COMPOSITION.t_report_result_14.v1.0.0

specialize
    openEHR-EHR-COMPOSITION.report-result.v1

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <\"unmanaged\">
    original_author = <
        [\"name\"] = <\"FerroEHR\">
    >

definition
    COMPOSITION[at0000.1.1] matches {
        content matches {
            use_archetype OBSERVATION[at0000.1.2, openEHR-EHR-OBSERVATION.laboratory_test_result.v1]
        }
    }

terminology
    term_definitions = <
        [\"en\"] = <
            [\"at0000.1.1\"] = <
                text = <\"Laboratory report\">
                description = <\"A laboratory result report\">
            >
            [\"at0000.1.2\"] = <
                text = <\"Laboratory test result\">
                description = <\"The result\">
            >
        >
    >
";

fn web_template() -> WebTemplate {
    let mut repo = ArchetypeRepository::default();
    for source in SOURCES {
        repo.insert(parse_artefact(source, Dialect::Adl14).expect("the CKM source parses as 1.4"));
    }
    let template = parse_artefact(TEMPLATE, Dialect::Adl2).expect("the template parses");
    let opt = create_opt(&template, &repo).expect("the operational template builds");
    build_web_template_v2_4(&opt).expect("the web template builds")
}

/// Every node under `node`, itself included, whose AQL path is `aql_path`.
fn nodes_at<'a>(node: &'a WebTemplateNode, aql_path: &str, found: &mut Vec<&'a WebTemplateNode>) {
    if node.aql_path == aql_path {
        found.push(node);
    }
    for child in &node.children {
        nodes_at(child, aql_path, found);
    }
}

#[test]
fn a_template_over_an_adl14_specialised_archetype_has_one_context() {
    let wt = web_template();
    let mut contexts = Vec::new();
    nodes_at(&wt.tree, "/context", &mut contexts);
    assert_eq!(
        contexts.len(),
        1,
        "one /context node, found {:?}",
        contexts.iter().map(|n| &n.id).collect::<Vec<_>>()
    );
    let context = contexts.first().expect("the context node");
    assert_eq!(context.rm_type, "EVENT_CONTEXT");

    // The builder folds a single-value ELEMENT into its value node.
    let mut reports = Vec::new();
    nodes_at(
        &wt.tree,
        "/context/other_context[at0001]/items[at0002]/value",
        &mut reports,
    );
    assert_eq!(reports.len(), 1, "one Report ID node");
}
