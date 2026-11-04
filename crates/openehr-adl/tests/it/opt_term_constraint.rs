// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An OPT2 that carries an external term constraint survives print then
//! parse, and an unconverted ADL 1.4 term constraint is refused by the printer.
//!
//! Oracle: `docs/specs/openehr/AM/docs/ADL2/master04.5-cadl_primitive_types.adoc`
//! §Formal Terminology Constraint (`[atN]`, `[acN]`, `[acN; atN]`) and
//! §Operational Binding Constraints (`[atN@ttttt]`, `[acN@ttttt]` in an OPT),
//! with `AM/docs/UML/classes/org.openehr.am.aom2.c_terminology_code.adoc`
//! (`constraint` is a local at- or ac-code). The CKM `encounter.v1` COMPOSITION
//! constrains its `category` to the ADL 1.4 qualified `[openehr::433]`, which
//! `adl14::convert` rewrites to an at-code bound to `openehr::433`.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_adl::adl14::convert::{ConvertConfig, parse_and_convert};
use openehr_adl::adl14::log::ConversionLog;
use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::opt::create_opt;
use openehr_adl::parse::Dialect;
use openehr_adl::print::{PrintError, print};
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
use openehr_am::v2_4::aom2::archetype::operational_template::OperationalTemplate;
use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_terminology_code::CTerminologyCode;

/// The CKM ADL 1.4 `encounter` COMPOSITION, its `category` held to
/// `[openehr::433]` (event).
const ENCOUNTER_14: &str = include_str!(
    "../../../../corpus/archetypes/ckm/adl14/openEHR-EHR-COMPOSITION.encounter.v1.adl"
);

/// An ADL2 template over `encounter` that adds nothing, so the OPT carries
/// the parent's category constraint as it stands.
const TEMPLATE: &str = "\
template (adl_version=2.4.0; rm_release=1.0.3)
    openEHR-EHR-COMPOSITION.t_encounter.v1.0.0

specialize
    openEHR-EHR-COMPOSITION.encounter.v1

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <\"unmanaged\">
    original_author = <
        [\"name\"] = <\"FerroEHR\">
    >

definition
    COMPOSITION[id1.1]    -- Templated encounter

terminology
    term_definitions = <
        [\"en\"] = <
            [\"id1.1\"] = <
                text = <\"Templated encounter\">
                description = <\"An encounter template\">
            >
        >
    >
";

fn converted_encounter() -> Archetype {
    parse_and_convert(
        ENCOUNTER_14,
        &ConvertConfig::default(),
        &mut ConversionLog::default(),
    )
    .expect("the CKM encounter converts")
}

fn opt_over(constituent: Archetype) -> OperationalTemplate {
    let mut repo = ArchetypeRepository::default();
    repo.insert(constituent);
    let template = parse_artefact(TEMPLATE, Dialect::Adl2).expect("the template parses");
    create_opt(&template, &repo).unwrap_or_else(|e| panic!("create_opt failed: {e}"))
}

fn as_artefact(opt: OperationalTemplate) -> Archetype {
    Archetype::AuthoredArchetype(Box::new(AuthoredArchetype::OperationalTemplate(Box::new(
        opt,
    ))))
}

fn attribute<'a>(attrs: Option<&'a mut Vec<CAttribute>>, name: &str) -> &'a mut CAttribute {
    attrs
        .and_then(|attrs| attrs.iter_mut().find(|a| a.rm_attribute_name == name))
        .unwrap_or_else(|| panic!("no `{name}` attribute"))
}

/// The `C_TERMINOLOGY_CODE` under `COMPOSITION.category/defining_code`.
fn category_code(opt: &mut OperationalTemplate) -> &mut CTerminologyCode {
    let CComplexObject::CComplexObject(root) = &mut opt.definition else {
        panic!("the OPT root is a plain C_COMPLEX_OBJECT");
    };
    let category = attribute(root.attributes.as_mut(), "category");
    let Some(CObject::CComplexObject(CComplexObject::CComplexObject(coded))) =
        category.children.as_mut().and_then(|c| c.first_mut())
    else {
        panic!("category holds a DV_CODED_TEXT object");
    };
    let defining = attribute(coded.attributes.as_mut(), "defining_code");
    match defining.children.as_mut().and_then(|c| c.first_mut()) {
        Some(CObject::CTerminologyCode(code)) => code,
        other => panic!("defining_code holds a C_TERMINOLOGY_CODE, found {other:?}"),
    }
}

/// Prints `opt`, parses the text back, and asserts the parse is the OPT it
/// was printed from.
fn assert_round_trips(opt: OperationalTemplate) {
    let artefact = as_artefact(opt);
    let text = print(&artefact).expect("the OPT prints");
    let reparsed = parse_artefact(&text, Dialect::Adl2)
        .unwrap_or_else(|e| panic!("the printed OPT parses back: {e:?}\n{text}"));
    assert_eq!(reparsed, artefact, "print then parse gives back the OPT");
}

#[test]
fn an_opt_over_a_converted_14_constituent_round_trips_its_external_term_constraint() {
    let mut opt = opt_over(converted_encounter());
    let code = category_code(&mut opt).constraint.clone();
    assert!(
        code.starts_with("at"),
        "the converter rewrites [openehr::433] to a local at-code, found {code:?}"
    );
    let bound = opt
        .terminology
        .term_bindings
        .as_ref()
        .and_then(|bindings| bindings.get("openehr"))
        .and_then(|openehr| openehr.get(&code))
        .expect("the at-code is bound to the openehr terminology");
    assert_eq!(bound.as_str(), "http://openehr.org/id/433");
    assert_round_trips(opt);
}

#[test]
fn an_operational_binding_term_constraint_round_trips() {
    let mut opt = opt_over(converted_encounter());
    let code = category_code(&mut opt);
    code.constraint = format!("{}@openehr", code.constraint);
    assert_round_trips(opt);
}

#[test]
fn an_unconverted_14_term_constraint_is_refused_by_the_printer() {
    let unconverted =
        parse_artefact(ENCOUNTER_14, Dialect::Adl14).expect("the CKM encounter parses as 1.4");
    let refusal = print(&unconverted).expect_err("[openehr::433] has no ADL2 syntax");
    assert!(
        matches!(
            &refusal,
            PrintError::TerminologyConstraint { constraint, assumed_value: None, .. }
                if constraint == "openehr::433"
        ),
        "the refusal names the constraint: {refusal:?}"
    );
}
