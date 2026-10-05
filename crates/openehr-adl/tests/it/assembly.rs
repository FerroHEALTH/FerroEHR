// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Assembly unit tests: hand-checked ODIN-section → AOM mappings
//! and the `regression`-tag reader, driving the public
//! [`openehr_adl::assemble::parse_artefact`] API.

// Test-scoped allows: assertions panic by design, and the artefact-variant
// matches name one variant + a diagnostic wildcard.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]
#![allow(
    clippy::match_wildcard_for_single_variants,
    reason = "the assertions match one artefact variant and treat every other as the failure case; naming the single remaining variant would silently stop covering a newly added one"
)]

use openehr_adl::assemble::parse_artefact;
use openehr_adl::meta::regression_tag;
use openehr_adl::parse::Dialect;
use openehr_adl::print::print;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
use openehr_am::v2_4::aom2::rm_overlay::visibility_type::VisibilityType;
use openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;
use openehr_am::v2_4::resource::resource_description::ResourceDescription;

/// The `AUTHORED_ARCHETYPE` data of an assembled source archetype.
fn authored(
    src: &str,
) -> openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetypeData {
    match parse_artefact(src, Dialect::Adl2).unwrap_or_else(|e| panic!("assemble failed: {e:?}")) {
        Archetype::AuthoredArchetype(a) => match *a {
            AuthoredArchetype::AuthoredArchetype(d) => d,
            other => panic!("expected a plain authored archetype, got {other:?}"),
        },
        other => panic!("expected an authored archetype, got {other:?}"),
    }
}

fn terminology(src: &str) -> ArchetypeTerminology {
    *authored(src).terminology
}

fn description(src: &str) -> ResourceDescription {
    *authored(src).description.expect("a description")
}

/// A minimal well-formed archetype with the given `definition`/`terminology`/…
/// section bodies spliced in.
fn archetype_with(sections: &str) -> String {
    format!(
        "archetype (adl_version=2.0.5; rm_release=1.0.2)\n\
         \topenehr-TEST_PKG-WHOLE.unit_test.v1.0.0\n\
         \nlanguage\n\toriginal_language = <[ISO_639-1::en]>\n\
         {sections}"
    )
}

// ── terminology mapping (incl. deprecated 1.4 forms; master07.13) ──────────

#[test]
fn terminology_maps_definitions_bindings_and_value_sets() {
    let src = archetype_with(
        "\ndescription\n\tlifecycle_state = <\"published\">\n\
         \ndefinition\n\tWHOLE[id1]\n\
         \nterminology\n\
         \tterm_definitions = <\n\
         \t\t[\"en\"] = <\n\
         \t\t\t[\"id1\"] = <\n\t\t\t\ttext = <\"Whole\">\n\t\t\t\tdescription = <\"The whole\">\n\t\t\t>\n\
         \t\t\t[\"ac1\"] = <\n\t\t\t\ttext = <\"a set\">\n\t\t\t\tdescription = <\"a value set\">\n\t\t\t>\n\
         \t\t>\n\t>\n\
         \tterm_bindings = <\n\
         \t\t[\"SNOMED_CT\"] = <\n\t\t\t[\"id1\"] = <http://snomed.info/id/123>\n\t\t>\n\t>\n\
         \tvalue_sets = <\n\
         \t\t[\"ac1\"] = <\n\t\t\tid = <\"ac1\">\n\t\t\tmembers = <\"at1\", \"at2\">\n\t\t>\n\t>\n",
    );
    let t = terminology(&src);
    assert_eq!(t.concept_code, "id1");
    assert_eq!(t.original_language, "en");
    let en = t.term_definitions.get("en").expect("en terms");
    assert_eq!(en.get("id1").expect("id1").text, "Whole");
    assert_eq!(en.get("ac1").expect("ac1").description, "a value set");
    let bindings = t.term_bindings.as_ref().expect("bindings");
    assert_eq!(
        bindings
            .get("SNOMED_CT")
            .and_then(|m| m.get("id1"))
            .map(String::as_str),
        Some("http://snomed.info/id/123")
    );
    let vs = t.value_sets.as_ref().expect("value_sets");
    assert_eq!(
        vs.get("ac1").expect("ac1").members.as_ref(),
        ["at1".to_owned(), "at2".to_owned()]
    );
}

#[test]
fn terminology_accepts_deprecated_items_wrapper_and_constraint_definitions() {
    // `master07.13` §Deprecated Terminology Section Features: the `items = <…>`
    // wrapper is unwrapped, and `constraint_definitions` merge into
    // `term_definitions`.
    let src = archetype_with(
        "\ndescription\n\tlifecycle_state = <\"published\">\n\
         \ndefinition\n\tWHOLE[id1]\n\
         \nterminology\n\
         \tterm_definitions = <\n\
         \t\t[\"en\"] = <\n\
         \t\t\titems = <\n\
         \t\t\t\t[\"id1\"] = <\n\t\t\t\t\ttext = <\"Whole\">\n\t\t\t\t\tdescription = <\"The whole\">\n\t\t\t\t>\n\
         \t\t\t>\n\t\t>\n\t>\n\
         \tconstraint_definitions = <\n\
         \t\t[\"en\"] = <\n\
         \t\t\t[\"ac1\"] = <\n\t\t\t\ttext = <\"constraint\">\n\t\t\t\tdescription = <\"a constraint\">\n\t\t\t>\n\
         \t\t>\n\t>\n",
    );
    let t = terminology(&src);
    let en = t.term_definitions.get("en").expect("en terms");
    // The `items`-wrapped id1 is present …
    assert_eq!(en.get("id1").expect("id1").text, "Whole");
    // … and the deprecated `constraint_definitions` ac1 is merged in.
    assert_eq!(en.get("ac1").expect("ac1").text, "constraint");
}

// ── description mapping (master07.08) ──────────────────────────────────────

#[test]
fn description_maps_author_details_and_regression_tag() {
    let src = archetype_with(
        "\ndescription\n\
         \toriginal_author = <\n\t\t[\"name\"] = <\"Test Author\">\n\t\t[\"email\"] = <\"a@example.org\">\n\t>\n\
         \tlifecycle_state = <\"published\">\n\
         \tcopyright = <\"Copyright 2026\">\n\
         \tdetails = <\n\
         \t\t[\"en\"] = <\n\
         \t\t\tlanguage = <[ISO_639-1::en]>\n\
         \t\t\tpurpose = <\"A test purpose.\">\n\
         \t\t\tkeywords = <\"test\", \"adl\">\n\t\t>\n\t>\n\
         \tother_details = <\n\t\t[\"regression\"] = <\"PASS\">\n\t>\n\
         \ndefinition\n\tWHOLE[id1]\n\
         \nterminology\n\tterm_definitions = <\n\t\t[\"en\"] = <\n\t\t\t[\"id1\"] = <\n\t\t\t\ttext = <\"x\">\n\t\t\t\tdescription = <\"x\">\n\t\t\t>\n\t\t>\n\t>\n",
    );
    let d = description(&src);
    assert_eq!(
        d.original_author.get("name").map(String::as_str),
        Some("Test Author")
    );
    assert_eq!(d.lifecycle_state, "published");
    assert_eq!(d.copyright.as_deref(), Some("Copyright 2026"));
    let en = d
        .details
        .as_ref()
        .and_then(|m| m.get("en"))
        .expect("en detail");
    assert_eq!(en.purpose, "A test purpose.");
    assert_eq!(en.keywords, Some(vec!["test".to_owned(), "adl".to_owned()]));
    assert_eq!(
        d.other_details
            .as_ref()
            .and_then(|m| m.get("regression"))
            .map(String::as_str),
        Some("PASS")
    );
}

// ── rm_overlay (master07.12; no corpus fixture — inline) ───────────────────

#[test]
fn rm_overlay_maps_visibility_and_alias() {
    let src = archetype_with(
        "\ndescription\n\tlifecycle_state = <\"published\">\n\
         \ndefinition\n\tWHOLE[id1]\n\
         \nterminology\n\tterm_definitions = <\n\t\t[\"en\"] = <\n\t\t\t[\"id1\"] = <\n\t\t\t\ttext = <\"x\">\n\t\t\t\tdescription = <\"x\">\n\t\t\t>\n\t\t>\n\t>\n\
         \nrm_overlay\n\
         \trm_visibility = <\n\
         \t\t[\"/context/other_context\"] = <\n\
         \t\t\tvisibility = <\"hide\">\n\
         \t\t\talias = <[local::at9000]>\n\t\t>\n\t>\n",
    );
    let a = authored(&src);
    {
        let rm = a.rm_overlay.as_ref().expect("rm_overlay");
        let vis = rm.rm_visibility.as_ref().expect("rm_visibility");
        let entry = vis.get("/context/other_context").expect("path entry");
        assert_eq!(
            entry.visibility.as_ref().map(VisibilityType::as_str),
            Some("hide")
        );
        assert_eq!(
            entry.alias.as_ref().map(|c| c.code_string.as_str()),
            Some("at9000")
        );
    }
    // and it round-trips.
    let first = Archetype::AuthoredArchetype(Box::new(AuthoredArchetype::AuthoredArchetype(a)));
    let second = parse_artefact(&print(&first).expect("print"), Dialect::Adl2).expect("re-parse");
    assert_eq!(first, second);
}

// ── template with overlays (no corpus fixture — inline; master10) ──────────

#[test]
fn template_with_overlay_assembles_and_round_trips() {
    let src = "template (adl_version=2.0.5; rm_release=1.0.2)\n\
        \topenehr-TEST_PKG-WHOLE.tpl.v1.0.0\n\
        \nlanguage\n\toriginal_language = <[ISO_639-1::en]>\n\
        \ndescription\n\tlifecycle_state = <\"published\">\n\
        \ndefinition\n\tWHOLE[id1]\n\
        \nterminology\n\tterm_definitions = <\n\t\t[\"en\"] = <\n\t\t\t[\"id1\"] = <\n\t\t\t\ttext = <\"x\">\n\t\t\t\tdescription = <\"x\">\n\t\t\t>\n\t\t>\n\t>\n\
        \ntemplate_overlay\n\
        \topenehr-TEST_PKG-WHOLE.ov.v1.0.0\n\
        \ndefinition\n\tWHOLE[id1]\n\
        \nterminology\n\tterm_definitions = <\n\t\t[\"en\"] = <\n\t\t\t[\"id1\"] = <\n\t\t\t\ttext = <\"o\">\n\t\t\t\tdescription = <\"o\">\n\t\t\t>\n\t\t>\n\t>\n";
    let first = parse_artefact(src, Dialect::Adl2).unwrap_or_else(|e| panic!("assemble: {e:?}"));
    let Archetype::AuthoredArchetype(inner) = &first else {
        panic!("expected authored archetype");
    };
    let AuthoredArchetype::Template(t) = inner.as_ref() else {
        panic!("expected a TEMPLATE, got {inner:?}");
    };
    assert_eq!(t.overlays.as_ref().map_or(0, Vec::len), 1);
    assert_eq!(
        t.overlays.as_deref().unwrap_or_default()[0]
            .archetype_id
            .concept_id,
        "ov"
    );
    // round-trips through the printer.
    let second = parse_artefact(&print(&first).expect("print"), Dialect::Adl2).expect("re-parse");
    assert_eq!(first, second);
}

// ── component_terminologies on an OPT (master10 worked-example shape) ───────

#[test]
fn operational_template_component_terminologies_round_trip() {
    let src = "operational_template (adl_version=2.0.5; rm_release=1.0.2)\n\
        \topenehr-TEST_PKG-WHOLE.opt.v1.0.0\n\
        \nlanguage\n\toriginal_language = <[ISO_639-1::en]>\n\
        \ndescription\n\tlifecycle_state = <\"published\">\n\
        \ndefinition\n\tWHOLE[id1]\n\
        \nterminology\n\tterm_definitions = <\n\t\t[\"en\"] = <\n\t\t\t[\"id1\"] = <\n\t\t\t\ttext = <\"x\">\n\t\t\t\tdescription = <\"x\">\n\t\t\t>\n\t\t>\n\t>\n\
        \ncomponent_terminologies\n\
        \t<\n\
        \t\t[\"openEHR-EHR-CLUSTER.device.v1.0.0\"] = <\n\
        \t\t\tterm_definitions = <\n\
        \t\t\t\t[\"en\"] = <\n\
        \t\t\t\t\t[\"id1\"] = <\n\t\t\t\t\t\ttext = <\"Device\">\n\t\t\t\t\t\tdescription = <\"A device\">\n\t\t\t\t\t>\n\
        \t\t\t\t>\n\t\t\t>\n\t\t>\n\t>\n";
    let first = parse_artefact(src, Dialect::Adl2).unwrap_or_else(|e| panic!("assemble: {e:?}"));
    let Archetype::AuthoredArchetype(inner) = &first else {
        panic!("expected authored archetype");
    };
    let AuthoredArchetype::OperationalTemplate(opt) = inner.as_ref() else {
        panic!("expected an OPERATIONAL_TEMPLATE, got {inner:?}");
    };
    let ct = opt
        .component_terminologies
        .as_ref()
        .expect("component_terminologies");
    let device = ct
        .get("openEHR-EHR-CLUSTER.device.v1.0.0")
        .expect("device terminology");
    assert_eq!(
        device
            .term_definitions
            .get("en")
            .and_then(|m| m.get("id1"))
            .map(|t| t.text.as_str()),
        Some("Device")
    );
    // round-trips.
    let second = parse_artefact(&print(&first).expect("print"), Dialect::Adl2).expect("re-parse");
    assert_eq!(first, second);
}

// ── the regression-tag reader ──────────────────────────────────────────────

/// Read a corpus file and return its assembled artefact.
fn corpus(rel: &str) -> Archetype {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/corpus/adl2-reference")
        .join(rel);
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {rel}: {e}"));
    parse_artefact(&src, Dialect::Adl2).unwrap_or_else(|e| panic!("assemble {rel}: {e:?}"))
}

#[test]
fn regression_tag_reads_the_corpus_oracle() {
    // A PASS-tagged file …
    assert_eq!(
        regression_tag(&corpus(
            "features/description/meta_data/openEHR-TEST_PKG-WHOLE.child_with_oid.v1.0.0.adls"
        )),
        Some("PASS".to_owned())
    );
    // … a rule-code-tagged validity file …
    assert_eq!(
        regression_tag(&corpus(
            "validity/basics/openEHR-TEST_PKG-ENTRY.VARCN_illegal_concept_code.v1.0.0.adls"
        )),
        Some("VARCN".to_owned())
    );
    // … and a FAIL-tagged (but assembling) file.
    assert_eq!(
        regression_tag(&corpus(
            "validity/domain_types/openEHR-TEST_PKG-ENTRY.VCOV_value_duplicated_in_ordinal.v1.0.0.adls"
        )),
        Some("PASS".to_owned())
    );
}
