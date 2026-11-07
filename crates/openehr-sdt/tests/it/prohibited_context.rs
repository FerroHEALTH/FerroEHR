// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A template that prohibits `COMPOSITION.context` yields a Web Template with
//! no `/context` node and none of its `start_time` or `setting` leaves (#5147).
//! The RM makes `context` optional (RM `UML/classes/org.openehr.rm.composition.composition.adoc`
//! §Attributes, `0..1`), so an OPT may close it with existence `0..0` (AOM 1.4
//! `master04-constraint_model_package.adoc` §existence) or with an
//! `EVENT_CONTEXT` of occurrences `0..0`. A template that leaves `context`
//! open keeps the synthesized node of ITS-REST `simplified_formats`
//! master04 §"Web Template Metadata". An OPT2 carries no such constraint: a
//! raw OPT removes every attribute of existence `0..0` and every object of
//! occurrences `0..0` (AM `OPT2/master03-opt_raw.adoc`).
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_its::opt14;
use openehr_sdt::flat::convert::{composition_from_flat, composition_to_flat};
use openehr_sdt::flat::example::{DetailLevel, example_composition};
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::{WebTemplate, WebTemplateNode};
use openehr_sdt::rm_instance::validate_composition;

const PERSISTENT_OPT: &str = "../../docs/specs/openehr/CNF/tests/platform/robot/_resources/test_data_sets/valid_templates/minimal_persistent/persistent_minimal.opt";
const NOW: &str = "2026-01-01T00:00:00Z";

/// The anchor in the OPT 1.4 fixture the `context` attribute is inserted before.
const CATEGORY_ATTRIBUTE: &str = "<attributes xsi:type=\"C_SINGLE_ATTRIBUTE\">\n      <rm_attribute_name>category</rm_attribute_name>";

fn interval(tag: &str, lower: u32, upper: u32) -> String {
    format!(
        "<{tag}><lower_included>true</lower_included><upper_included>true</upper_included>\
         <lower_unbounded>false</lower_unbounded><upper_unbounded>false</upper_unbounded>\
         <lower>{lower}</lower><upper>{upper}</upper></{tag}>"
    )
}

/// The persistent minimal OPT 1.4 with `context_attribute` inserted on the
/// COMPOSITION (none: the fixture as vendored, which leaves `context` open).
fn opt14_web_template(context_attribute: Option<&str>) -> WebTemplate {
    let xml = std::fs::read_to_string(PERSISTENT_OPT)
        .unwrap_or_else(|e| panic!("read {PERSISTENT_OPT}: {e}"));
    assert!(
        !xml.contains("<rm_attribute_name>context</rm_attribute_name>"),
        "the fixture leaves context unconstrained"
    );
    let xml = match context_attribute {
        None => xml,
        Some(attribute) => {
            assert_eq!(
                xml.matches(CATEGORY_ATTRIBUTE).count(),
                1,
                "one category attribute"
            );
            xml.replacen(
                CATEGORY_ATTRIBUTE,
                &format!("{attribute}\n{CATEGORY_ATTRIBUTE}"),
                1,
            )
        }
    };
    let opt = opt14::from_xml(&xml).expect("parse OPT 1.4");
    build_web_template(&opt).expect("build web template")
}

/// A `context` attribute with the given existence and, when given, one
/// `EVENT_CONTEXT` child with the given occurrences.
fn context_attribute(existence: (u32, u32), child_occurrences: Option<(u32, u32)>) -> String {
    let child = child_occurrences.map_or_else(String::new, |(lower, upper)| {
        format!(
            "<children xsi:type=\"C_COMPLEX_OBJECT\"><rm_type_name>EVENT_CONTEXT</rm_type_name>\
             {}<node_id/></children>",
            interval("occurrences", lower, upper)
        )
    });
    format!(
        "<attributes xsi:type=\"C_SINGLE_ATTRIBUTE\"><rm_attribute_name>context</rm_attribute_name>\
         {}{child}</attributes>",
        interval("existence", existence.0, existence.1)
    )
}

fn any_node(node: &WebTemplateNode, f: &dyn Fn(&WebTemplateNode) -> bool) -> bool {
    f(node) || node.children.iter().any(|c| any_node(c, f))
}

fn assert_no_context(wt: &WebTemplate) {
    assert!(
        !any_node(&wt.tree, &|n| n.aql_path.starts_with("/context")
            || n.rm_type == "EVENT_CONTEXT"),
        "a template that prohibits context has no /context node"
    );
}

fn assert_context_with_leaves(wt: &WebTemplate) {
    let ctx = wt
        .tree
        .children
        .iter()
        .find(|c| c.aql_path == "/context")
        .expect("the /context node");
    assert_eq!(
        ctx.rm_type, "EVENT_CONTEXT",
        "the context node is an EVENT_CONTEXT"
    );
    let leaves: Vec<&str> = ctx.children.iter().map(|c| c.aql_path.as_str()).collect();
    for expected in ["/context/start_time", "/context/setting"] {
        assert!(leaves.contains(&expected), "missing {expected}: {leaves:?}");
    }
}

#[test]
fn opt14_existence_zero_prohibits_the_context_node() {
    let wt = opt14_web_template(Some(&context_attribute((0, 0), None)));
    assert_no_context(&wt);
    // The other COMPOSITION in-context children stay.
    let ids: Vec<&str> = wt.tree.children.iter().map(|c| c.id.as_str()).collect();
    for expected in ["category", "language", "territory", "composer"] {
        assert!(ids.contains(&expected), "missing {expected}: {ids:?}");
    }
}

#[test]
fn opt14_event_context_of_zero_occurrences_prohibits_the_context_node() {
    let wt = opt14_web_template(Some(&context_attribute((0, 1), Some((0, 0)))));
    assert_no_context(&wt);
}

#[test]
fn opt14_open_or_optional_context_keeps_the_context_node() {
    assert_context_with_leaves(&opt14_web_template(None));
    assert_context_with_leaves(&opt14_web_template(Some(&context_attribute((0, 1), None))));
    assert_context_with_leaves(&opt14_web_template(Some(&context_attribute(
        (1, 1),
        Some((1, 1)),
    ))));
}

#[test]
fn a_context_less_template_builds_and_round_trips_a_composition_without_context() {
    let wt = opt14_web_template(Some(&context_attribute((0, 0), None)));
    for level in [
        DetailLevel::Required,
        DetailLevel::Medium,
        DetailLevel::Complete,
    ] {
        let example = example_composition(&wt, level).expect("the example builds");
        assert!(
            example.get("context").is_none(),
            "the {level:?} example carries no context"
        );
        let messages = validate_composition(&example, &wt);
        assert!(
            messages.is_empty(),
            "the {level:?} example validates: {messages:?}"
        );

        let flat = composition_to_flat(&example, &wt).expect("flatten");
        assert!(
            !flat
                .keys()
                .any(|k| k.starts_with("ctx/time") || k.contains("/context")),
            "the FLAT form names no context: {flat:?}"
        );
        let rebuilt = composition_from_flat(&flat, &wt, NOW).expect("rebuild");
        assert!(
            rebuilt.get("context").is_none(),
            "the rebuild adds no context"
        );
    }
}
