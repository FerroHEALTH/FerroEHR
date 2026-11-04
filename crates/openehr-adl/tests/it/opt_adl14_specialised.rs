// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An OPT over an ADL 1.4 specialised archetype carries its structure once
//! (#4979).
//!
//! The CKM `report-result.v1` COMPOSITION specialises `report.v1` in ADL 1.4,
//! and an ADL 1.4 archetype is authored in flat form: "Up until ADL 1.4,
//! archetypes were expressed in 'flat' form ADL"
//! (`docs/specs/openehr/AM/docs/ADL2/master07.02-adl_conventions.adoc`
//! §File-naming Convention; AOM2 `master03-archetype_package.adoc`
//! §Structural Variants: "ADL 1.4 legacy archetypes (always in flat form)").
//! The child restates the parent's whole `context`, so overlaying it on the
//! parent as an ADL 2 differential appended a second `EVENT_CONTEXT`.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::collections::BTreeSet;

use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::flatten::flat_form;
use openehr_adl::opt::{create_opt, unresolved_constituents};
use openehr_adl::parse::Dialect;
use openehr_adl::paths::locate;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
use openehr_am::v2_4::aom2::archetype::operational_template::OperationalTemplate;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;

/// The CKM ADL 1.4 `report` COMPOSITION, which specialises nothing.
const REPORT_14: &str =
    include_str!("../../../../corpus/archetypes/ckm/adl14/openEHR-EHR-COMPOSITION.report.v1.adl");

/// The CKM ADL 1.4 `report-result` COMPOSITION, a specialisation of `report`
/// that restates its `context` and narrows `Status` to `{0..1}`.
const REPORT_RESULT_14: &str = include_str!(
    "../../../../corpus/archetypes/ckm/adl14/openEHR-EHR-COMPOSITION.report-result.v1.adl"
);

/// The CKM ADL 1.4 `laboratory_test_result` OBSERVATION the template holds.
const LAB_RESULT_14: &str = include_str!(
    "../../../../corpus/archetypes/ckm/adl14/openEHR-EHR-OBSERVATION.laboratory_test_result.v1.adl"
);

/// The specialisation parent `report-result.v1` names.
const REPORT_ID: &str = "openEHR-EHR-COMPOSITION.report.v1";

/// A template over the unconverted 1.4 `report-result`, holding a laboratory
/// test result, as a consumer builds it over data in a CDR.
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

/// A repository of the three CKM sources, parsed in the 1.4 dialect and never
/// converted.
fn repo() -> ArchetypeRepository {
    let mut repo = ArchetypeRepository::default();
    for source in [REPORT_14, REPORT_RESULT_14, LAB_RESULT_14] {
        repo.insert(parse_artefact(source, Dialect::Adl14).expect("the CKM source parses as 1.4"));
    }
    repo
}

fn template() -> Archetype {
    parse_artefact(TEMPLATE, Dialect::Adl2).expect("the template parses")
}

fn opt() -> OperationalTemplate {
    create_opt(&template(), &repo()).unwrap_or_else(|e| panic!("create_opt failed: {e}"))
}

/// The objects directly under `attribute` of the complex object at `path`,
/// the root when `path` is `None`.
fn children_of<'a>(
    opt: &'a OperationalTemplate,
    path: Option<&str>,
    attribute: &str,
) -> Vec<&'a CObject> {
    let object = match path {
        None => &opt.definition,
        Some(path) => match locate(&opt.definition, path) {
            Some(CObject::CComplexObject(object)) => object,
            other => panic!("{path} is a complex object, found {other:?}"),
        },
    };
    let path = path.unwrap_or("/");
    let attrs = match object {
        CComplexObject::CComplexObject(d) => d.attributes.as_ref(),
        CComplexObject::CArchetypeRoot(r) => r.attributes.as_ref(),
    };
    let attrs = attrs.unwrap_or_else(|| panic!("{path} carries attributes"));
    attrs
        .iter()
        .filter(|a| a.rm_attribute_name == attribute)
        .flat_map(|a| a.children.iter().flatten())
        .collect()
}

/// Every place under `object` where one attribute name repeats on an object
/// or one attribute holds two equal objects, by the path of the holder.
fn collect_repeats(object: &CComplexObject, path: &str, repeated: &mut Vec<String>) {
    let attrs = match object {
        CComplexObject::CComplexObject(d) => d.attributes.as_deref(),
        CComplexObject::CArchetypeRoot(r) => r.attributes.as_deref(),
    };
    let mut names = BTreeSet::new();
    for attr in attrs.unwrap_or_default() {
        let at = format!("{path}/{}", attr.rm_attribute_name);
        if !names.insert(attr.rm_attribute_name.as_str()) {
            repeated.push(at.clone());
        }
        let children: Vec<&CObject> = attr.children.iter().flatten().collect();
        for (i, child) in children.iter().enumerate() {
            if children.iter().skip(i + 1).any(|other| other == child) {
                repeated.push(at.clone());
            }
            if let CObject::CComplexObject(complex) = child {
                collect_repeats(complex, &at, repeated);
            }
        }
    }
}

#[test]
fn an_opt_over_an_adl14_specialised_archetype_builds_one_event_context() {
    let opt = opt();

    let contexts = children_of(&opt, None, "context");
    assert_eq!(contexts.len(), 1, "one EVENT_CONTEXT, found {contexts:#?}");

    let mut repeated = Vec::new();
    collect_repeats(&opt.definition, "", &mut repeated);
    assert!(repeated.is_empty(), "no object appears twice: {repeated:?}");

    let items = "/context/other_context[at0001]";
    let ids: Vec<String> = children_of(&opt, Some(items), "items")
        .into_iter()
        .map(|c| match c {
            CObject::CComplexObject(CComplexObject::CComplexObject(d)) => d.node_id.clone(),
            CObject::ArchetypeSlot(s) => s.node_id.clone(),
            other => panic!("an item is an ELEMENT or a slot, found {other:?}"),
        })
        .collect();
    assert_eq!(ids, ["at0002", "at0005", "at0006", "at0007"]);

    // The child's own narrowing of `Status` to `{0..1}` is the flat form's.
    match locate(&opt.definition, &format!("{items}/items[at0005]")) {
        Some(CObject::CComplexObject(CComplexObject::CComplexObject(status))) => {
            let occurrences = status
                .occurrences
                .as_ref()
                .expect("Status carries occurrences");
            assert_eq!(occurrences.upper, Some(1), "{occurrences:?}");
        }
        other => panic!("Status is an ELEMENT, found {other:?}"),
    }

    assert!(
        locate(&opt.definition, "/content[at0000.1.2]").is_some(),
        "the template's laboratory result fills the content"
    );
}

#[test]
fn the_14_term_constraint_is_still_rewritten_under_an_adl14_specialised_root() {
    let opt = opt();
    let code = match locate(&opt.definition, "/category/defining_code") {
        Some(CObject::CTerminologyCode(c)) => c.constraint.clone(),
        other => panic!("category/defining_code is a C_TERMINOLOGY_CODE, found {other:?}"),
    };
    assert!(
        code.starts_with("at"),
        "[openehr::433] is rewritten to a local at-code, found {code:?}"
    );
    let bound = opt
        .terminology
        .term_bindings
        .as_ref()
        .and_then(|bindings| bindings.get("openehr"))
        .and_then(|openehr| openehr.get(&code))
        .expect("the at-code is bound to the openehr terminology");
    assert_eq!(bound.as_str(), "http://openehr.org/id/433");
}

#[test]
fn the_flat_form_of_an_adl14_specialised_archetype_is_its_authored_form_with_its_lineage() {
    let repo = repo();
    let child = parse_artefact(REPORT_RESULT_14, Dialect::Adl14).expect("report-result parses");
    let flat = flat_form(&child, &repo).expect("the flat form builds");
    let Archetype::AuthoredArchetype(flat) = flat else {
        panic!("the flat form is an authored archetype");
    };
    let AuthoredArchetype::AuthoredArchetype(flat) = *flat else {
        panic!("the flat form is an archetype");
    };
    let Archetype::AuthoredArchetype(authored) = &child else {
        panic!("report-result is an authored archetype");
    };
    let AuthoredArchetype::AuthoredArchetype(authored) = authored.as_ref() else {
        panic!("report-result is an archetype");
    };
    assert_eq!(
        flat.definition, authored.definition,
        "the authored form is the flat form"
    );
    assert_eq!(flat.terminology, authored.terminology);
    assert!(!flat.is_differential);
    assert_eq!(
        flat.parent_archetype_id.as_deref(),
        Some(REPORT_ID),
        "the lineage is kept"
    );
}

#[test]
fn an_opt_over_an_adl14_specialised_archetype_needs_no_parent_of_it() {
    let mut repo = ArchetypeRepository::default();
    for source in [REPORT_RESULT_14, LAB_RESULT_14] {
        repo.insert(parse_artefact(source, Dialect::Adl14).expect("the CKM source parses as 1.4"));
    }
    let template = template();
    assert!(
        unresolved_constituents(&template, &repo).is_empty(),
        "report-result is its own flat form, so report.v1 is not needed"
    );
    let without_parent = create_opt(&template, &repo).expect("the OPT builds without report.v1");
    assert_eq!(without_parent, opt(), "the parent adds nothing to the OPT");
}
