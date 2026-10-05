// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![expect(clippy::panic, reason = "test assertions/diagnostics/fixtures")]
//! TDD (Ocean **Template Data Document**) → canonical `COMPOSITION` conversion,
//! against the vendored CNF corpus TDD instances + their operational templates
//! (`docs/specs/openehr/CNF/tests/platform/robot/_resources/test_data_sets/`).
//!
//! The matching CNF `CANONICAL_JSON` fixtures are *independently generated*
//! instances of the same template (different territory/composer/leaf data, and
//! for the persistent case no `context`), so they are **not** a byte-for-byte
//! conversion oracle. These tests therefore assert (a) the produced COMPOSITION
//! deserialises as an `openehr-rm` [`Composition`], (b) it passes the
//! `WebTemplate` + RM-invariant + terminology validation
//! ([`validate_composition`]) — the real correctness bar the SM `import_tdd`
//! commit path enforces — and (c) the structural skeleton the OPT supplies
//! (`archetype_node_id`s, re-materialised `HISTORY`/`EVENT`/`ITEM_TREE`/`ELEMENT`
//! wrappers, `_type` tags) and the *instance* leaf/context values carried from
//! the TDD are exactly as expected.
#![expect(
    clippy::expect_used,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_its::opt14;
use openehr_rm::prelude::Composition;
use openehr_sdt::flat::tdd::from_tdd;
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::WebTemplate;
use openehr_sdt::rm_instance::validate_composition;
use serde_json::Value;

const CORPUS: &str = "../../docs/specs/openehr/CNF/tests/platform/robot/_resources/test_data_sets";

fn wt_from(opt_path: &str) -> WebTemplate {
    let opt_xml =
        std::fs::read_to_string(opt_path).unwrap_or_else(|e| panic!("read {opt_path}: {e}"));
    build_web_template(&opt14::from_xml(&opt_xml).expect("parse OPT")).expect("build WebTemplate")
}

fn convert(opt_path: &str, tdd_rel: &str) -> Value {
    let wt = wt_from(opt_path);
    let tdd = std::fs::read_to_string(format!("{CORPUS}/compositions/TDD/{tdd_rel}"))
        .unwrap_or_else(|e| panic!("read TDD {tdd_rel}: {e}"));
    from_tdd(&tdd, &wt).unwrap_or_else(|e| panic!("from_tdd {tdd_rel}: {e}"))
}

/// The converted COMPOSITION deserialises as RM and passes validation.
fn assert_valid(comp: &Value, opt_path: &str) {
    openehr_its::json::from_canonical_value::<Composition>(comp)
        .expect("deserialises as RM Composition");
    let errors = validate_composition(comp, &wt_from(opt_path));
    assert!(
        errors.is_empty(),
        "converted COMPOSITION must validate; errors: {:?}",
        errors
            .iter()
            .map(|m| format!("[{:?}] {}: {}", m.kind, m.path, m.message))
            .collect::<Vec<_>>()
    );
}

#[test]
fn persistent_minimal_converts_and_validates() {
    let opt = format!("{CORPUS}/valid_templates/minimal_persistent/persistent_minimal.opt");
    let comp = convert(&opt, "persistent_minimal.en.v1__full.xml");
    assert_valid(&comp, &opt);

    // Root identity supplied by the OPT.
    assert_eq!(comp["_type"], "COMPOSITION");
    assert_eq!(
        comp["archetype_node_id"],
        "openEHR-EHR-COMPOSITION.persistent_minimal.v1"
    );
    assert_eq!(
        comp["archetype_details"]["template_id"]["value"],
        "persistent_minimal.en.v1"
    );
    // Instance context carried faithfully from the TDD.
    assert_eq!(comp["name"]["value"], "Persistent minimal");
    assert_eq!(comp["territory"]["code_string"], "US");
    assert_eq!(comp["category"]["defining_code"]["code_string"], "431");
    assert_eq!(comp["composer"]["_type"], "PARTY_IDENTIFIED");
    assert_eq!(comp["composer"]["name"], "composer test value");
    assert_eq!(comp["links"].as_array().unwrap().len(), 2);
    assert_eq!(comp["links"][0]["target"]["value"], "ehr://target1");
    assert_eq!(
        comp["context"]["participations"].as_array().unwrap().len(),
        2
    );

    // Content: one OBSERVATION; the `WebTemplate` compacted the
    // HISTORY/EVENT/ITEM_TREE/ELEMENT chain, re-materialised here from the leaf's
    // aqlPath node-ids (at0001..at0004), with the TDD's leaf datum.
    let obs = &comp["content"][0];
    assert_eq!(obs["_type"], "OBSERVATION");
    assert_eq!(
        obs["archetype_node_id"],
        "openEHR-EHR-OBSERVATION.minimal.v1"
    );
    let history = &obs["data"];
    assert_eq!(history["_type"], "HISTORY");
    assert_eq!(history["archetype_node_id"], "at0001");
    // The TDD spells the compacted HISTORY out as a <data> wrapper carrying
    // its own <origin>: the instance value travels, never the RM-mandatory
    // 1970 default (#2982).
    assert_eq!(history["origin"]["value"], "2021-05-20T16:47:47.044+03:00");
    let event = &history["events"][0];
    assert_eq!(event["_type"], "POINT_EVENT");
    assert_eq!(event["archetype_node_id"], "at0002");
    // Same for the spelled-out event wrapper (<Cualquier_evento_as_Point_Event>,
    // corresponding to the POINT_EVENT via its Ocean `_as_` suffix): its <time>
    // and <name> instance data land on the re-materialised node.
    assert_eq!(event["time"]["value"], "2021-05-20T16:47:47.044+03:00");
    assert_eq!(event["name"]["value"], "Cualquier evento");
    let item_tree = &event["data"];
    assert_eq!(item_tree["_type"], "ITEM_TREE");
    assert_eq!(item_tree["archetype_node_id"], "at0003");
    let element = &item_tree["items"][0];
    assert_eq!(element["_type"], "ELEMENT");
    assert_eq!(element["archetype_node_id"], "at0004");
    assert_eq!(element["value"]["_type"], "DV_TEXT");
    assert_eq!(element["value"]["value"], "value 1");
}

#[test]
fn nested_converts_and_validates() {
    // The nested OPT is not vendored under the CNF corpus; use the flat-crate
    // SDK fixture copy (same `nested.en.v1` operational template).
    let opt = "../openehr-its/tests/fixtures/sdk/nested.en.v1.opt";
    let comp = convert(opt, "nested.en.v1__full.xml");
    assert_valid(&comp, opt);

    assert_eq!(
        comp["archetype_node_id"],
        "openEHR-EHR-COMPOSITION.nesting.v1"
    );
    // SECTION → INSTRUCTION → ACTIVITY, all identified from the OPT.
    let section = &comp["content"][0];
    assert_eq!(section["_type"], "SECTION");
    assert_eq!(
        section["archetype_node_id"],
        "openEHR-EHR-SECTION.nested.v1"
    );
    let instruction = &section["items"][0];
    assert_eq!(instruction["_type"], "INSTRUCTION");
    assert_eq!(instruction["narrative"]["value"], "narrative");
    let activity = &instruction["activities"][0];
    assert_eq!(activity["_type"], "ACTIVITY");
    assert_eq!(activity["archetype_node_id"], "at0001");
    assert_eq!(
        activity["timing"]["value"],
        "R5/2008-03-01T13:00:00Z/P1Y2M10DT2H30M"
    );

    // ACTIVITY.description ITEM_TREE was omitted in the TDD (unlike the explicit
    // OBSERVATION HISTORY) and re-materialised from the `WebTemplate` aqlPath, as an
    // archetyped root carrying archetype_details.
    let desc = &activity["description"];
    assert_eq!(desc["_type"], "ITEM_TREE");
    assert_eq!(desc["archetype_node_id"], "openEHR-EHR-ITEM_TREE.nested.v1");
    assert_eq!(
        desc["archetype_details"]["archetype_id"]["value"],
        "openEHR-EHR-ITEM_TREE.nested.v1"
    );

    // Leaf datatypes parsed from the rm:-namespaced fragments (typed via the
    // canonical-XML codec: nested _type tags + numeric coercion).
    let ordinal = &desc["items"][0];
    assert_eq!(ordinal["value"]["_type"], "DV_ORDINAL");
    assert_eq!(ordinal["value"]["value"], 0);
    assert_eq!(
        ordinal["value"]["symbol"]["defining_code"]["code_string"],
        "code"
    );

    let cluster = &desc["items"][1];
    assert_eq!(cluster["_type"], "CLUSTER");
    assert_eq!(
        cluster["archetype_node_id"],
        "openEHR-EHR-CLUSTER.nested.v1"
    );
    assert_eq!(cluster["items"][0]["value"]["value"], "value 1");
    assert_eq!(cluster["items"][1]["value"]["_type"], "DV_DATE_TIME");

    // Nested2 CLUSTER: DV_BOOLEAN + DV_COUNT (magnitude coerced to a number).
    let nested2 = &cluster["items"][2];
    assert_eq!(nested2["_type"], "CLUSTER");
    assert_eq!(nested2["items"][0]["value"]["_type"], "DV_BOOLEAN");
    assert_eq!(nested2["items"][0]["value"]["value"], false);
    assert_eq!(nested2["items"][1]["value"]["_type"], "DV_COUNT");
    assert_eq!(nested2["items"][1]["value"]["magnitude"], 99265);
}

/// The refusal twin of the wrapper-instance-data recovery (#2982): a
/// spelled-out wrapper whose metadata cannot legally sit on the node it
/// corresponds to is a typed conversion error, never silently dropped —
/// here a `<time>` on the `<data>` wrapper, which re-materialises as a
/// HISTORY, and HISTORY carries no `time` attribute.
#[test]
fn ill_fitting_wrapper_metadata_is_refused_not_dropped() {
    let opt = format!("{CORPUS}/valid_templates/minimal_persistent/persistent_minimal.opt");
    let wt = wt_from(&opt);
    let tdd = std::fs::read_to_string(format!(
        "{CORPUS}/compositions/TDD/persistent_minimal.en.v1__full.xml"
    ))
    .expect("read corpus TDD");
    let mutated = tdd.replace(
        "<origin>",
        "<time><rm:value>2021-01-01T00:00:00Z</rm:value></time><origin>",
    );
    assert_ne!(mutated, tdd, "the fixture must carry the mutation");
    let err = from_tdd(&mutated, &wt).expect_err("time on a HISTORY wrapper must refuse");
    assert!(
        err.to_string().contains("not an attribute"),
        "the refusal names the illegal key: {err}"
    );
}

/// A payload whose root does not match the template root is a typed conversion
/// error, not a panic.
#[test]
fn wrong_root_is_conversion_error() {
    let opt = format!("{CORPUS}/valid_templates/minimal_persistent/persistent_minimal.opt");
    let wt = wt_from(&opt);
    let err = from_tdd(
        r#"<Bogus xmlns="http://schemas.oceanehr.com/templates"/>"#,
        &wt,
    )
    .expect_err("root mismatch must error");
    assert!(
        err.to_string().contains("does not match template root"),
        "got: {err}"
    );
}

/// A TDD conforms to the template-derived TDS ("a kind of XSD" — AM OPT2
/// `master02-overview.adoc` §Purpose of the OPT): an element the template
/// defines no node for is REJECTED, never silently dropped into a thinner
/// (still-valid) COMPOSITION — the silent absorption is exactly how a
/// nonconforming import committed on the 2026-07-29 conformance run.
#[test]
fn nonconforming_content_is_rejected_not_dropped() {
    let opt = format!("{CORPUS}/valid_templates/minimal_persistent/persistent_minimal.opt");
    let wt = wt_from(&opt);
    let tdd = std::fs::read_to_string(format!(
        "{CORPUS}/compositions/TDD/persistent_minimal.en.v1__full.xml"
    ))
    .expect("read TDD");

    // The conforming document still converts (the tolerance for compacted-
    // wrapper RM metadata holds).
    from_tdd(&tdd, &wt).expect("the vendored TDD conforms");

    // Rename one content node to a name the template does not define — the
    // one defect the derived cnf corpus fixture carries.
    let renamed = tdd
        .replace("<Minimal>", "<Unknown_content>")
        .replace("</Minimal>", "</Unknown_content>");
    assert_ne!(tdd, renamed, "the fixture must contain the renamed node");
    let err = from_tdd(&renamed, &wt).expect_err("nonconforming content must be rejected");
    assert!(
        err.to_string()
            .contains("matches no node of the operational template"),
        "got: {err}"
    );
}
