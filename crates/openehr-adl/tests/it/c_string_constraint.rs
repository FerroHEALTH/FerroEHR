// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `C_STRING.constraint`: a string constraint is either a list of fixed Strings
//! or a single regular expression, never a mixture
//! (`docs/specs/openehr/AM/docs/ADL2/master04.5-cadl_primitive_types.adoc`
//! §Constraints on String; the AOM2 `C_STRING` class table, which stores a
//! regular expression with its `/` or `^` delimiters).
//!
//! The ADL 2 grammar cannot spell a mixture, so the refusing cases build the
//! constraint on a parsed model, the shape an AOM2 XML, JSON or OPT 1.4 input
//! reaches the validator in.

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test assertions panic/unwrap/expect by design, including in fixture helpers outside #[test] fns"
)]

use openehr_adl::assemble::parse_artefact;
use openehr_adl::parse::Dialect;
use openehr_adl::validate::catalogue::ValidationCode;
use openehr_adl::validate::{ValidationIssue, validate_integrity, validate_source_integrity};
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_string::CString;

/// A phase-1-clean ADL 2 archetype whose `ELEMENT.value` holds a `DV_TEXT`
/// with the string constraint `{CONSTRAINT}`.
const TEMPLATE: &str = r#"archetype (adl_version=2.0.5; rm_release=1.0.2)
    openEHR-EHR-ENTRY.c_string_form.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <"draft">

definition
    ENTRY[id1] matches {
        element_attr matches {
            ELEMENT[id2] matches {
                value matches {
                    DV_TEXT[id3] matches {
                        value matches {CONSTRAINT}
                    }
                }
            }
        }
    }

terminology
    term_definitions = <
        ["en"] = <
            ["id1"] = < text = <"root"> description = <"root"> >
            ["id2"] = < text = <"el"> description = <"el"> >
            ["id3"] = < text = <"text"> description = <"text"> >
        >
    >
"#;

fn source(constraint: &str) -> String {
    TEMPLATE.replace("CONSTRAINT", constraint)
}

fn string_issues(issues: &[ValidationIssue]) -> Vec<&ValidationIssue> {
    issues
        .iter()
        .filter(|i| i.code == ValidationCode::CStringConstraint)
        .collect()
}

/// The archetype of [`TEMPLATE`] with its `C_STRING` constraint replaced by
/// `constraint` on the parsed model.
fn with_model_constraint(constraint: &[&str]) -> Archetype {
    let mut a = parse_artefact(&source("\"x\""), Dialect::Adl2)
        .unwrap_or_else(|e| panic!("source must parse: {e:?}"));
    let Archetype::AuthoredArchetype(b) = &mut a else {
        panic!("a plain archetype");
    };
    let AuthoredArchetype::AuthoredArchetype(d) = b.as_mut() else {
        panic!("a plain AUTHORED_ARCHETYPE");
    };
    let CComplexObject::CComplexObject(root) = &mut d.definition else {
        panic!("a plain root");
    };
    let c_string = string_node(first_child(root.attributes.as_deref_mut()));
    c_string.constraint = Some(constraint.iter().map(|c| (*c).to_owned()).collect());
    a
}

/// The first object of the first attribute, descending one level.
fn first_child(attrs: Option<&mut [CAttribute]>) -> &mut CObject {
    &mut attrs.unwrap_or_default()[0]
        .children
        .as_deref_mut()
        .unwrap_or_default()[0]
}

/// The `C_STRING` under `ELEMENT.value` / `DV_TEXT.value`.
fn string_node(element: &mut CObject) -> &mut CString {
    let CObject::CComplexObject(CComplexObject::CComplexObject(el)) = element else {
        panic!("ELEMENT");
    };
    let CObject::CComplexObject(CComplexObject::CComplexObject(text)) =
        first_child(el.attributes.as_deref_mut())
    else {
        panic!("DV_TEXT");
    };
    let CObject::CString(s) = first_child(text.attributes.as_deref_mut()) else {
        panic!("C_STRING");
    };
    s
}

#[test]
fn a_fixed_string_beside_a_regular_expression_is_refused() {
    let issues = validate_integrity(&with_model_constraint(&["a", "/b.*/"]), None);
    let found = string_issues(&issues);
    assert_eq!(found.len(), 1, "got {issues:?}");
    assert!(
        found[0].message.contains("mixes fixed Strings"),
        "{}",
        found[0].message
    );
    assert_eq!(
        found[0].path.as_deref(),
        Some("/element_attr[id2]/value[id3]/value[id9999]"),
        "the issue is anchored at the C_STRING"
    );
}

#[test]
fn two_regular_expressions_are_refused() {
    let issues = validate_integrity(&with_model_constraint(&["/a.*/", "^b.*^"]), None);
    let found = string_issues(&issues);
    assert_eq!(found.len(), 1, "got {issues:?}");
    assert!(
        found[0]
            .message
            .contains("more than one regular expression"),
        "{}",
        found[0].message
    );
}

#[test]
fn a_list_of_fixed_strings_is_accepted() {
    let issues = validate_source_integrity(&source(r#""a", "b"; "b""#), Dialect::Adl2, None)
        .expect("a fixed-string list parses");
    assert!(string_issues(&issues).is_empty(), "got {issues:?}");
    let issues = validate_integrity(&with_model_constraint(&["a", "b", "c"]), None);
    assert!(string_issues(&issues).is_empty(), "got {issues:?}");
}

#[test]
fn a_single_regular_expression_with_an_assumed_value_is_accepted() {
    let issues = validate_source_integrity(&source(r#"/a.*/; "abc""#), Dialect::Adl2, None)
        .expect("a single regular expression parses");
    assert!(string_issues(&issues).is_empty(), "got {issues:?}");
    let issues = validate_integrity(&with_model_constraint(&["^a.*^"]), None);
    assert!(string_issues(&issues).is_empty(), "got {issues:?}");
}

#[test]
fn the_adl2_grammar_cannot_spell_a_mixture() {
    assert!(
        validate_source_integrity(&source(r#""a", /b.*/"#), Dialect::Adl2, None).is_err(),
        "a fixed String and a regular expression in one block are a syntax error"
    );
}
