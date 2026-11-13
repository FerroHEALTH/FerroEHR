// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A differential path on an attribute below the root of a specialised
//! archetype (#5452).
//!
//! AOM2 `C_ATTRIBUTE.differential_path` is the "Path to the parent object of
//! this attribute", and `C_ATTRIBUTE.path` the "Path of this attribute with
//! respect to owning `C_OBJECT`, including differential path where
//! applicable" (`AM/docs/UML/classes/org.openehr.am.aom2.c_attribute.adoc`),
//! so a nested path reads from the object that owns it. VDIFP requires that
//! path to exist in the flat parent (`AOM2/master04.5` §`C_ATTRIBUTE`).

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_adl::aom::access::{complex_attributes, object_node_id};
use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::flatten::{FlattenError, flat_form};
use openehr_adl::parse::Dialect;
use openehr_adl::validate::bindings::NoTerminologyResolver;
use openehr_adl::validate::catalogue::{Severity, ValidationCode};
use openehr_adl::validate::rm::ProductionRmModel;
use openehr_adl::validate::validate_source;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;

/// A coded statement under `/data[id2]/items[id3]/value[id4]/defining_code`.
const PARENT: &str = r#"archetype (adl_version=2.4.0; rm_release=1.1.0)
	openEHR-EHR-EVALUATION.nested_diff.v1.0.0

language
	original_language = <[ISO_639-1::en]>

description
	original_author = <
		["name"] = <"FerroEHR">
	>
	details = <
		["en"] = <
			language = <[ISO_639-1::en]>
			purpose = <"A parent whose coded element a specialisation narrows">
		>
	>
	lifecycle_state = <"unmanaged">

definition
	EVALUATION[id1] matches {
		data matches {
			ITEM_TREE[id2] matches {
				items matches {
					ELEMENT[id3] occurrences matches {0..*} matches {
						value matches {
							DV_CODED_TEXT[id4] matches {
								defining_code matches {[ac1]}
							}
						}
					}
				}
			}
		}
	}

terminology
	term_definitions = <
		["en"] = <
			["id1"] = <
				text = <"Statement">
				description = <"A statement">
			>
			["id3"] = <
				text = <"Kind">
				description = <"The kind of statement">
			>
			["ac1"] = <
				text = <"Kinds">
				description = <"Every kind">
			>
			["at1"] = <
				text = <"First">
				description = <"The first kind">
			>
			["at2"] = <
				text = <"Second">
				description = <"The second kind">
			>
		>
	>
	value_sets = <
		["ac1"] = <
			id = <"ac1">
			members = <"at1", "at2">
		>
	>
"#;

/// The specialisation, narrowing the value set through the nested
/// differential path `differential` inside the redefined `ELEMENT[id3]`.
fn child(differential: &str) -> String {
    format!(
        r#"archetype (adl_version=2.4.0; rm_release=1.1.0)
	openEHR-EHR-EVALUATION.nested_diff-first.v1.0.0

specialize
	openEHR-EHR-EVALUATION.nested_diff.v1

language
	original_language = <[ISO_639-1::en]>

description
	original_author = <
		["name"] = <"FerroEHR">
	>
	details = <
		["en"] = <
			language = <[ISO_639-1::en]>
			purpose = <"Narrows the parent's coded element to its first kind">
		>
	>
	lifecycle_state = <"unmanaged">

definition
	EVALUATION[id1.1] matches {{
		/data[id2]/items matches {{
			ELEMENT[id3] matches {{
				{differential} matches {{[ac1.1]}}
			}}
		}}
	}}

terminology
	term_definitions = <
		["en"] = <
			["id1.1"] = <
				text = <"First statement">
				description = <"A statement of the first kind">
			>
			["ac1.1"] = <
				text = <"First kind">
				description = <"The first kind only">
			>
		>
	>
	value_sets = <
		["ac1.1"] = <
			id = <"ac1.1">
			members = <"at1">
		>
	>
"#
    )
}

fn parse(src: &str) -> Archetype {
    parse_artefact(src, Dialect::Adl2).expect("the fixture parses")
}

fn repo() -> ArchetypeRepository {
    let mut repo = ArchetypeRepository::default();
    repo.insert(parse(PARENT));
    repo
}

/// The error-severity codes the child raises against the parent.
fn error_codes(differential: &str) -> Vec<ValidationCode> {
    validate_source(
        &child(differential),
        Some(&repo()),
        &ProductionRmModel,
        &NoTerminologyResolver,
    )
    .expect("the child parses")
    .into_iter()
    .filter(|i| i.severity == Severity::Error)
    .map(|i| i.code)
    .collect()
}

fn root(a: &Archetype) -> &CComplexObject {
    match a {
        Archetype::AuthoredArchetype(inner) => match inner.as_ref() {
            AuthoredArchetype::AuthoredArchetype(d) => &d.definition,
            AuthoredArchetype::Template(t) => &t.definition,
            AuthoredArchetype::OperationalTemplate(o) => &o.definition,
        },
        Archetype::TemplateOverlay(t) => &t.definition,
    }
}

/// The complex object one attribute and node id below `obj`.
fn below<'a>(obj: &'a CComplexObject, attr: &str, id: &str) -> &'a CComplexObject {
    let child = complex_attributes(obj)
        .iter()
        .find(|a| a.rm_attribute_name == attr)
        .and_then(|a| {
            a.children
                .iter()
                .flatten()
                .find(|c| object_node_id(c) == id)
        })
        .expect("the path resolves in the flat form");
    match child {
        CObject::CComplexObject(cco) => cco,
        other => panic!("{attr}[{id}] is not a complex object: {other:?}"),
    }
}

fn attribute_names(obj: &CComplexObject) -> Vec<&str> {
    complex_attributes(obj)
        .iter()
        .map(|a| a.rm_attribute_name.as_str())
        .collect()
}

#[test]
fn a_nested_path_the_parent_carries_passes_vdifp() {
    let codes = error_codes("/value[id4]/defining_code");
    assert!(codes.is_empty(), "{codes:?}");
}

#[test]
fn a_nested_path_the_parent_lacks_fails_vdifp() {
    let codes = error_codes("/value[id5]/defining_code");
    assert!(codes.contains(&ValidationCode::Vdifp), "{codes:?}");
}

#[test]
fn the_flat_form_overlays_a_nested_path_on_the_node_it_names() {
    let flat = flat_form(&parse(&child("/value[id4]/defining_code")), &repo())
        .expect("the child flattens");
    let element = below(below(root(&flat), "data", "id2"), "items", "id3");
    assert_eq!(attribute_names(element), ["value"]);
    let coded = below(element, "value", "id4");
    let constraint = format!("{:?}", complex_attributes(coded));
    assert!(constraint.contains("ac1.1"), "{constraint}");
}

#[test]
fn flattening_refuses_a_nested_path_the_parent_lacks() {
    let refused = flat_form(&parse(&child("/value[id5]/defining_code")), &repo());
    assert!(
        matches!(
            &refused,
            Err(FlattenError::UnresolvedDifferentialPath(path)) if path == "/value[id5]"
        ),
        "{refused:?}"
    );
}
