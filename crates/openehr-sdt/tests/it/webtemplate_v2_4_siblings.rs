// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Two `use_archetype` overlays of one archetype that the template tells apart
//! by a fixed name (#4927).
//!
//! In data both sections carry the archetype id as their `archetype_node_id`
//! (RM common `master03-archetyped_package.adoc` §"The `LOCATABLE` class"), so
//! only the runtime name tells them apart (BASE
//! `architecture_overview/master11-paths.adoc` §"Using a Name-based
//! Predicate"). The ADL 2.4 WebTemplate builder must give each sibling its own
//! name-qualified path, as the OPT 1.4 builder does, so a composition holding
//! both validates and a third instance of one of them is still refused.

#![expect(
    clippy::expect_used,
    reason = "integration-test fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::opt::create_opt;
use openehr_adl::parse::Dialect;
use openehr_sdt::flat::validation::validate_archetype_conformance;
use openehr_sdt::flat::webtemplate::builder_v2_4::build_web_template_v2_4;
use openehr_sdt::flat::webtemplate::model::{WebTemplate, WebTemplateNode};
use openehr_sdt::rm_instance::ValidationKind;
use serde_json::{Value, json};

/// A synthetic ad hoc SECTION with an open `items` attribute.
const SECTION: &str = "\
archetype (adl_version=2.4.0; rm_release=1.1.0)
    openEHR-EHR-SECTION.adhoc.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <\"unmanaged\">
    original_author = <
        [\"name\"] = <\"FerroEHR\">
    >

definition
    SECTION[at0000] matches {
        items matches {
            ENTRY[at0001] occurrences matches {0..*}
        }
    }

terminology
    term_definitions = <
        [\"en\"] = <
            [\"at0000\"] = <
                text = <\"Ad hoc heading\">
                description = <\"A heading\">
            >
            [\"at0001\"] = <
                text = <\"Entry\">
                description = <\"Entry\">
            >
        >
    >
";

/// A COMPOSITION with an open content slot.
const COMPOSITION: &str = "\
archetype (adl_version=2.4.0; rm_release=1.1.0)
    openEHR-EHR-COMPOSITION.summary.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <\"unmanaged\">
    original_author = <
        [\"name\"] = <\"FerroEHR\">
    >

definition
    COMPOSITION[at0000] matches {
        content matches {
            allow_archetype SECTION[at0002] occurrences matches {0..*} matches {
                include
                    archetype_id/value matches {/.*/}
            }
        }
    }

terminology
    term_definitions = <
        [\"en\"] = <
            [\"at0000\"] = <
                text = <\"Summary\">
                description = <\"Summary\">
            >
            [\"at0002\"] = <
                text = <\"Content\">
                description = <\"Content\">
            >
        >
    >
";

/// An overlay of the ad hoc SECTION named `name`.
fn overlay(id: &str, name: &str) -> String {
    format!(
        "----------------------------------------------------------------
template_overlay
    openEHR-EHR-SECTION.{id}.v1.0.0

specialize
    openEHR-EHR-SECTION.adhoc.v1

definition
    SECTION[at0000.1] matches {{
        name matches {{
            DV_TEXT matches {{
                value matches {{\"{name}\"}}
            }}
        }}
    }}

terminology
    term_definitions = <
        [\"en\"] = <
            [\"at0000.1\"] = <
                text = <\"{name}\">
                description = <\"{name}\">
            >
        >
    >
"
    )
}

/// A template filling the slot with two named overlays of the ad hoc SECTION,
/// each at most once.
fn template() -> String {
    let head = "\
template (adl_version=2.4.0; rm_release=1.1.0)
    openEHR-EHR-COMPOSITION.t_summary.v1.0.0

specialize
    openEHR-EHR-COMPOSITION.summary.v1

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <\"unmanaged\">
    original_author = <
        [\"name\"] = <\"FerroEHR\">
    >

definition
    COMPOSITION[at0000.1] matches {
        content matches {
            use_archetype SECTION[at0002.1, openEHR-EHR-SECTION.t_medication.v1] occurrences matches {0..1}
            use_archetype SECTION[at0002.2, openEHR-EHR-SECTION.t_immunisations.v1] occurrences matches {0..1}
        }
    }

terminology
    term_definitions = <
        [\"en\"] = <
            [\"at0000.1\"] = <
                text = <\"Templated summary\">
                description = <\"Templated summary\">
            >
        >
    >
";
    format!(
        "{head}{}{}",
        overlay("t_medication", "Medication summary"),
        overlay("t_immunisations", "Immunisations")
    )
}

fn web_template() -> WebTemplate {
    let mut repo = ArchetypeRepository::default();
    repo.insert(parse_artefact(SECTION, Dialect::Adl2).expect("the section parses"));
    repo.insert(parse_artefact(COMPOSITION, Dialect::Adl2).expect("the composition parses"));
    let template = parse_artefact(&template(), Dialect::Adl2).expect("the template parses");
    let opt = create_opt(&template, &repo).expect("the operational template builds");
    build_web_template_v2_4(&opt.template, &opt.prohibited_attributes)
        .expect("the web template builds")
}

fn sections(node: &WebTemplateNode, out: &mut Vec<String>) {
    if node.rm_type == "SECTION" {
        out.push(node.aql_path.clone());
    }
    for child in &node.children {
        sections(child, out);
    }
}

/// A canonical SECTION of the ad hoc archetype named `name`.
fn section(name: &str) -> Value {
    json!({
        "_type": "SECTION",
        "name": { "_type": "DV_TEXT", "value": name },
        "archetype_node_id": "openEHR-EHR-SECTION.adhoc.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-SECTION.adhoc.v1" },
            "rm_version": "1.1.0"
        }
    })
}

/// A canonical COMPOSITION of the template holding `content`.
fn composition(content: &[Value]) -> Value {
    json!({
        "_type": "COMPOSITION",
        "name": { "_type": "DV_TEXT", "value": "Templated summary" },
        "archetype_node_id": "openEHR-EHR-COMPOSITION.summary.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-COMPOSITION.summary.v1" },
            "template_id": { "_type": "TEMPLATE_ID", "value": "t_summary" },
            "rm_version": "1.1.0"
        },
        "language": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" }, "code_string": "en" },
        "territory": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_3166-1" }, "code_string": "NL" },
        "category": {
            "_type": "DV_CODED_TEXT", "value": "event",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "433" }
        },
        "composer": { "_type": "PARTY_SELF" },
        "content": content
    })
}

fn occurrence_errors(composition: &Value, wt: &WebTemplate) -> Vec<String> {
    validate_archetype_conformance(composition, wt)
        .into_iter()
        .filter(|m| m.kind == ValidationKind::Occurrences)
        .map(|m| format!("{}: {}", m.path, m.message))
        .collect()
}

/// Each named overlay is its own sibling, addressed by its name.
#[test]
fn same_archetype_siblings_take_name_qualified_paths() {
    let wt = web_template();
    let mut paths = Vec::new();
    sections(&wt.tree, &mut paths);
    paths.sort();
    assert_eq!(
        paths,
        [
            "/content[openEHR-EHR-SECTION.adhoc.v1,'Immunisations']",
            "/content[openEHR-EHR-SECTION.adhoc.v1,'Medication summary']",
        ]
    );
}

/// A composition holding one section of each name passes the occurrence
/// check; a second section of one name is refused.
#[test]
fn a_composition_holding_both_named_siblings_validates() {
    let wt = web_template();
    let both = composition(&[section("Medication summary"), section("Immunisations")]);
    assert_eq!(occurrence_errors(&both, &wt), Vec::<String>::new());

    let twice = composition(&[
        section("Medication summary"),
        section("Medication summary"),
        section("Immunisations"),
    ]);
    assert!(
        !occurrence_errors(&twice, &wt).is_empty(),
        "a second medication summary exceeds its occurrences"
    );
}
