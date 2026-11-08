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
//! occurrences `0..0` (AM `OPT2/master03-opt_raw.adoc`), so `openehr-adl`
//! names the removed attributes beside the OPT and the `v2_4` front end reads
//! them (#5154).
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::opt::create_opt;
use openehr_adl::parse::Dialect;
use openehr_its::opt14;
use openehr_sdt::flat::convert::{composition_from_flat, composition_to_flat};
use openehr_sdt::flat::example::{DetailLevel, example_composition};
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::builder_v2_4::build_web_template_v2_4;
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

fn context_min(wt: &WebTemplate) -> Option<i32> {
    wt.tree
        .children
        .iter()
        .find(|c| c.aql_path == "/context")
        .expect("the /context node")
        .min
}

/// The `/context` node's `min` follows the template's existence on `context`,
/// and the RM's `0..1` where the template declares none (#5154; AM
/// `ADL2/master04.3-cadl_complex_types.adoc` §Existence: an archetype's
/// existence constrains "the existence defined by the underlying reference
/// model").
#[test]
fn the_context_node_min_follows_the_existence_and_defaults_to_the_rm() {
    assert_eq!(context_min(&opt14_web_template(None)), Some(0), "open");
    assert_eq!(
        context_min(&opt14_web_template(Some(&context_attribute((0, 1), None)))),
        Some(0),
        "existence 0..1"
    );
    assert_eq!(
        context_min(&opt14_web_template(Some(&context_attribute((1, 1), None)))),
        Some(1),
        "existence 1..1"
    );
    assert_eq!(
        context_min(&opt14_web_template(Some(&context_attribute(
            (0, 1),
            Some((1, 1))
        )))),
        Some(0),
        "existence 0..1 over an EVENT_CONTEXT of occurrences 1..1"
    );
    assert_eq!(
        context_min(&opt14_web_template(Some(&context_attribute(
            (1, 1),
            Some((1, 1))
        )))),
        Some(1),
        "existence 1..1 over an EVENT_CONTEXT of occurrences 1..1"
    );
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

/// The Web Template of an ADL2 COMPOSITION archetype with `constraints` on its
/// root, compiled to its OPT2 and built with the attributes the OPT removed
/// when `with_prohibited`, or with none as for an OPT2 received compiled.
fn opt2_web_template(constraints: &str, with_prohibited: bool) -> WebTemplate {
    let source = format!(
        "archetype (adl_version=2.0.6; rm_release=1.0.2; generated)\n    \
         openEHR-EHR-COMPOSITION.context_probe.v1.0.0\n\n\
         language\n    original_language = <[ISO_639-1::en]>\n\n\
         description\n    lifecycle_state = <\"unmanaged\">\n\n\
         definition\n    COMPOSITION[id1] matches {{\n        {constraints}\n        \
         content cardinality matches {{0..*; unordered}} matches {{\n            \
         OBSERVATION[id2] occurrences matches {{0..*}}\n        }}\n    }}\n\n\
         terminology\n    term_definitions = <\n        [\"en\"] = <\n            \
         [\"id1\"] = <text = <\"Probe\">; description = <\"A probe\">>\n            \
         [\"id2\"] = <text = <\"Observation\">; description = <\"An observation\">>\n        \
         >\n    >\n"
    );
    let archetype = parse_artefact(&source, Dialect::Adl2).expect("the probe parses");
    let opt = create_opt(&archetype, &ArchetypeRepository::default()).expect("the OPT builds");
    let prohibited: &[String] = if with_prohibited {
        &opt.prohibited_attributes
    } else {
        &[]
    };
    build_web_template_v2_4(&opt.template, prohibited).expect("build web template")
}

#[test]
fn opt2_deleted_context_prohibits_the_context_node() {
    let wt = opt2_web_template("context existence matches {0}", true);
    assert_no_context(&wt);
    let ids: Vec<&str> = wt.tree.children.iter().map(|c| c.id.as_str()).collect();
    for expected in ["category", "language", "territory", "composer"] {
        assert!(ids.contains(&expected), "missing {expected}: {ids:?}");
    }
}

#[test]
fn opt2_open_or_optional_context_keeps_the_context_node() {
    assert_context_with_leaves(&opt2_web_template("", true));
    assert_context_with_leaves(&opt2_web_template("context existence matches {0..1}", true));
    // An OPT2 received compiled names no removed attribute, so its deleted
    // context reads at the RM's 0..1.
    assert_context_with_leaves(&opt2_web_template("context existence matches {0}", false));
}

#[test]
fn a_context_less_opt2_refuses_a_composition_carrying_context() {
    let open = opt2_web_template("", true);
    let deleted = opt2_web_template("context existence matches {0}", true);
    let example = example_composition(&open, DetailLevel::Complete).expect("the example builds");
    assert!(
        example.get("context").is_some(),
        "the open template's example carries a context"
    );
    assert!(
        validate_composition(&example, &open).is_empty(),
        "the example validates against its own template"
    );
    assert!(
        !validate_composition(&example, &deleted).is_empty(),
        "a template that deletes context refuses a composition carrying one"
    );
    let own = example_composition(&deleted, DetailLevel::Complete).expect("the example builds");
    assert!(
        own.get("context").is_none(),
        "no context in its own example"
    );
    let messages = validate_composition(&own, &deleted);
    assert!(
        messages.is_empty(),
        "its own example validates: {messages:?}"
    );
}

#[test]
fn a_context_less_opt14_refuses_a_composition_carrying_context() {
    let open = opt14_web_template(None);
    let prohibited = opt14_web_template(Some(&context_attribute((0, 0), None)));
    let example = example_composition(&open, DetailLevel::Complete).expect("the example builds");
    assert!(
        example.get("context").is_some(),
        "the example carries a context"
    );
    let messages = validate_composition(&example, &prohibited);
    assert!(
        messages
            .iter()
            .any(|m| m.path == "/context" && m.message.contains("prohibited")),
        "a template that prohibits context refuses a composition carrying one: {messages:?}"
    );
}
