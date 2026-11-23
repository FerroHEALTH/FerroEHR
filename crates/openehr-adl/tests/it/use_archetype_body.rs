// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A `use_archetype` reference with a constraint body (SPECPR-217, SPECAM-87;
//! `cadl2.g4` `c_archetype_root`; `ADL2/master10-templates.adoc`).

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_adl::aom::access::{complex_attributes, object_node_id};
use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::opt::create_opt;
use openehr_adl::parse::{Dialect, parse_definition_body};
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
use openehr_am::v2_4::aom2::constraint_model::c_archetype_root::CArchetypeRoot;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;

fn parse(src: &str) -> Archetype {
    parse_artefact(src, Dialect::Adl2).unwrap_or_else(|e| panic!("parse failed: {e:?}"))
}

/// The one `C_ARCHETYPE_ROOT` under the root's `content`.
fn content_root(def: &CComplexObject) -> &CArchetypeRoot {
    let content = complex_attributes(def)
        .iter()
        .find(|a| a.rm_attribute_name == "content")
        .expect("a content attribute");
    match content.children.as_deref().unwrap_or_default() {
        [CObject::CComplexObject(CComplexObject::CArchetypeRoot(root))] => root,
        other => panic!("content holds one archetype root, got {other:?}"),
    }
}

fn attribute_names(root: &CArchetypeRoot) -> Vec<&str> {
    root.attributes
        .iter()
        .flatten()
        .map(|a| a.rm_attribute_name.as_str())
        .collect()
}

#[test]
fn a_use_archetype_body_fills_the_root_attributes() {
    let def = parse_definition_body(
        "COMPOSITION[id1] matches {
            content matches {
                use_archetype SECTION[id2, openEHR-EHR-SECTION.event.v1] occurrences matches {1} matches {
                    items matches {
                        ELEMENT[id3] occurrences matches {0}
                    }
                    _default = (SECTION) <
                        archetype_node_id = <\"id2\">
                    >
                }
            }
        }",
        Dialect::Adl2,
    )
    .expect("the body form parses");
    let root = content_root(&def);
    assert_eq!(root.archetype_ref, "openEHR-EHR-SECTION.event.v1");
    assert_eq!(root.node_id, "id2");
    assert!(root.occurrences.is_some());
    assert_eq!(attribute_names(root), ["items"]);
    assert!(root.default_value.is_some(), "the body's _default is kept");
}

#[test]
fn a_bodiless_use_archetype_has_no_attributes() {
    let def = parse_definition_body(
        "COMPOSITION[id1] matches {
            content matches {
                use_archetype SECTION[id2, openEHR-EHR-SECTION.event.v1] occurrences matches {1}
            }
        }",
        Dialect::Adl2,
    )
    .expect("the bodiless form parses");
    let root = content_root(&def);
    assert_eq!(attribute_names(root), Vec::<&str>::new());
    assert!(root.default_value.is_none());
}

#[test]
fn a_use_archetype_body_needs_an_attribute_definition() {
    for body in [
        "matches { }",
        "matches { _default = <archetype_node_id = <\"id2\">> }",
    ] {
        let src = format!(
            "COMPOSITION[id1] matches {{
                content matches {{
                    use_archetype SECTION[id2, openEHR-EHR-SECTION.event.v1] {body}
                }}
            }}"
        );
        let errors = parse_definition_body(&src, Dialect::Adl2)
            .expect_err("c_attribute_def+ refuses a body without an attribute");
        assert!(
            errors
                .iter()
                .any(|e| e.to_string().contains("attribute definition")),
            "the refusal names the missing attribute definition, got {errors:?}"
        );
    }
}

/// The id-coded OPT2 listing of `ADL2/master10-templates.adoc`, verbatim from
/// the vendored specification.
fn master10_opt2_example() -> String {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/specs/openehr/AM/docs/ADL2/master10-templates.adoc"
    );
    let text = std::fs::read_to_string(path).expect("read the vendored templates chapter");
    let marker = "use_archetype SECTION[id0.1, openEHR-EHR-SECTION.t_patient_event_info_ds_sf.v1.0.0] occurrences matches {1} matches {";
    let (before, after) = text
        .split_once(marker)
        .expect("the OPT2 example carries a use_archetype body");
    let (_, head) = before
        .rsplit_once("operational_template (")
        .expect("the listing opens with its header");
    let (tail, _) = after.split_once("\n--------").expect("the listing closes");
    format!("operational_template ({head}{marker}{tail}")
}

#[test]
fn the_specification_opt2_example_parses_with_its_override() {
    let source = master10_opt2_example();
    let Archetype::AuthoredArchetype(authored) = parse(&source) else {
        panic!("the example is an authored artefact");
    };
    let AuthoredArchetype::OperationalTemplate(opt) = *authored else {
        panic!("the example is an operational template");
    };
    let section = content_root(&opt.definition);
    assert_eq!(
        section.archetype_ref,
        "openEHR-EHR-SECTION.t_patient_event_info_ds_sf.v1.0.0"
    );
    let items = section
        .attributes
        .iter()
        .flatten()
        .find(|a| a.rm_attribute_name == "items")
        .expect("the section body constrains items");
    let [CObject::CComplexObject(CComplexObject::CArchetypeRoot(admin))] =
        items.children.as_deref().unwrap_or_default()
    else {
        panic!("items holds the nested use_archetype");
    };
    assert_eq!(attribute_names(admin), ["other_participations"]);
}

const CLUSTER: &str = "\
archetype (adl_version=2.4.0; rm_release=1.0.3)
    openEHR-EHR-SECTION.event.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <\"unmanaged\">
    original_author = <
        [\"name\"] = <\"openEHR\">
    >

definition
    SECTION[id1] matches {
        items matches {
            ELEMENT[id2] occurrences matches {0..1} matches {
                value matches {
                    DV_TEXT[id3]
                }
            }
            ELEMENT[id4] occurrences matches {0..1} matches {
                value matches {
                    DV_TEXT[id5]
                }
            }
        }
    }

terminology
    term_definitions = <
        [\"en\"] = <
            [\"id1\"] = <text = <\"Event\"> description = <\"Event\">>
            [\"id2\"] = <text = <\"Keep\"> description = <\"Kept\">>
            [\"id4\"] = <text = <\"Drop\"> description = <\"Removed by the template\">>
        >
    >
";

const TEMPLATE: &str = "\
template (adl_version=2.4.0; rm_release=1.0.3)
    openEHR-EHR-COMPOSITION.t_event.v1.0.0

specialize
    openEHR-EHR-COMPOSITION.report.v1

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <\"unmanaged\">
    original_author = <
        [\"name\"] = <\"openEHR\">
    >

definition
    COMPOSITION[id1.1] matches {
        content matches {
            use_archetype SECTION[id2.1, openEHR-EHR-SECTION.event.v1] occurrences matches {1} matches {
                items matches {
                    ELEMENT[id4] occurrences matches {0}
                }
            }
        }
    }

terminology
    term_definitions = <
        [\"en\"] = <
            [\"id1.1\"] = <text = <\"Templated report\"> description = <\"Templated report\">>
        >
    >
";

const COMPOSITION: &str = "\
archetype (adl_version=2.4.0; rm_release=1.0.3)
    openEHR-EHR-COMPOSITION.report.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <\"unmanaged\">
    original_author = <
        [\"name\"] = <\"openEHR\">
    >

definition
    COMPOSITION[id1] matches {
        content matches {
            allow_archetype SECTION[id2] matches {
                include
                    archetype_id/value matches {/openEHR-EHR-SECTION\\..*/}
            }
        }
    }

terminology
    term_definitions = <
        [\"en\"] = <
            [\"id1\"] = <text = <\"Report\"> description = <\"Report\">>
            [\"id2\"] = <text = <\"Section\"> description = <\"Section slot\">>
        >
    >
";

#[test]
fn a_template_with_an_overridden_root_round_trips_through_the_printer() {
    let template = parse(TEMPLATE);
    let printed = openehr_adl::print::print(&template).expect("print the template");
    assert!(
        printed.contains(
            "use_archetype SECTION[id2.1, openEHR-EHR-SECTION.event.v1] occurrences matches {1} matches {"
        ),
        "the printer writes the body back on the use_archetype line:\n{printed}"
    );
    let reparsed = parse(&printed);
    assert_eq!(template, reparsed, "the template round-trips");
}

#[test]
fn the_operational_template_applies_the_root_override() {
    let mut repo = ArchetypeRepository::default();
    repo.insert(parse(CLUSTER));
    repo.insert(parse(COMPOSITION));
    let opt = create_opt(&parse(TEMPLATE), &repo)
        .unwrap_or_else(|e| panic!("create_opt failed: {e}"))
        .template;
    let section = content_root(&opt.definition);
    assert_eq!(section.archetype_ref, "openEHR-EHR-SECTION.event.v1.0.0");
    let items = section
        .attributes
        .iter()
        .flatten()
        .find(|a| a.rm_attribute_name == "items")
        .expect("the inlined section keeps items");
    let ids: Vec<&str> = items
        .children
        .iter()
        .flatten()
        .map(object_node_id)
        .collect();
    assert_eq!(
        ids,
        ["id2"],
        "the body's occurrences {{0}} removes id4 from the inlined archetype"
    );
}
