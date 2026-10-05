// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The **v3** (`org.openehr.lang.bmm3`) behavioural surface, through its public
//! API: the type lattice of `LANG/docs/bmm3/master06-core-types.adoc`, the
//! class/feature functions of `master07-core-classes.adoc` +
//! `master08-core-features.adoc`, and the `P_BMM` → v3 `BMM_MODEL`
//! materialisation.
//!
//! Most cases run over the vendored openEHR RM 1.0.2 inclusion chain
//! (`tests/vendor/bmm/openehr/`), so what is pinned is the real corpus rather than
//! a hand-shaped fixture. Three v3-only capabilities the RM 1.0.2 corpus never
//! states — a generic ancestor's parameter SUBSTITUTION, class routines, and a
//! value-set constrained type — are exercised over the small hand-written
//! [`ROUTINE_SCHEMA`], whose every construct is taken from
//! `LANG/docs/bmm_persistence/master04-syntax.adoc`.

#![expect(
    clippy::panic_in_result_fn,
    reason = "the Book ch11 test shape: `?` propagates the read/resolve/model plumbing while the assertions ARE the test — an assertion panic is how these tests fail"
)]
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use openehr_lang::v1_1::bmm_persistence::create_bmm3_model::create_bmm3_model;
use openehr_lang::v1_1::bmm_persistence::error::PBmmReadError;
use openehr_lang::v1_1::bmm_persistence::include_resolution::resolve_includes;
use openehr_lang::v1_1::bmm_persistence::p_bmm_schema::PBmmSchema;
use openehr_lang::v1_1::bmm_persistence::reader::read_schema;
use openehr_lang::v1_1::bmm3::core::entity::bmm_class::BmmClass;
use openehr_lang::v1_1::bmm3::core::entity::bmm_model_type::BmmModelType;
use openehr_lang::v1_1::bmm3::core::entity::bmm_simple_class::BmmSimpleClass;
use openehr_lang::v1_1::bmm3::core::entity::bmm_type::BmmType;
use openehr_lang::v1_1::bmm3::core::entity::bmm_unitary_type::BmmUnitaryType;
use openehr_lang::v1_1::bmm3::core::entity::range_constrained::bmm_enumeration::BmmEnumeration;
use openehr_lang::v1_1::bmm3::core::feature::bmm_property::BmmProperty;
use openehr_lang::v1_1::bmm3::core::model::bmm_model::BmmModel;

/// The vendored openEHR RM 1.0.2 inclusion chain, deepest first.
const CHAIN: &[&str] = &[
    "openehr_primitive_types_102.bmm",
    "openehr_basic_types_102.bmm",
    "openehr_structures_102.bmm",
    "openehr_ehr_102.bmm",
];

/// The source of one vendored chain file.
fn vendored(file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/vendor/bmm/openehr")
        .join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The v3 model of the RM 1.0.2 chain's top schema (`openehr_ehr_102.bmm`), with
/// the three included schemas resolved into it.
fn rm_model() -> Result<BmmModel, PBmmReadError> {
    let mut available: BTreeMap<String, PBmmSchema> = BTreeMap::new();
    let mut top: Option<PBmmSchema> = None;
    for file in CHAIN {
        let schema = read_schema(&vendored(file))?;
        available.insert(schema.schema_id(), schema.clone());
        top = Some(schema);
    }
    let resolved = resolve_includes(top.expect("the chain is non-empty"), &available)?;
    create_bmm3_model(&resolved)
}

/// The class named `name` in `model`.
fn class<'a>(model: &'a BmmModel, name: &str) -> &'a BmmClass {
    model
        .class_definitions
        .as_ref()
        .expect("the model defines classes")
        .get(name)
        .unwrap_or_else(|| panic!("{name} is missing from the model"))
}

/// `master07-core-classes.adoc` §Overview: the model materialises as v3 classes,
/// each of which is also a `BMM_MODULE` (§Inherit), and both maps cover the same
/// population (`…bmm3.bmm_model.adoc` §Attributes).
#[test]
fn the_vendored_rm_chain_materialises_a_v3_model() -> Result<(), PBmmReadError> {
    let model = rm_model()?;
    let classes = model
        .class_definitions
        .as_ref()
        .expect("the model defines classes");
    let modules = model.modules.as_ref().expect("the model lists modules");
    assert_eq!(classes.len(), modules.len());
    assert!(
        classes.len() > 100,
        "the RM 1.0.2 chain materialised only {} classes",
        classes.len(),
    );
    assert_eq!(model.rm_publisher, "openehr");
    assert_eq!(model.rm_release, "1.0.2");
    assert!(model.packages.is_some(), "no package tree materialised");
    Ok(())
}

/// `…bmm3.bmm_class.adoc` §Description: `_ancestors_` "contains a list of _types_
/// rather than classes" — so an ordinary ancestor is a `BMM_SIMPLE_TYPE` over the
/// parent class, and `has_ancestor_class` walks it (case-insensitively, per
/// `master06-core-types.adoc` §Type Conformance).
#[test]
fn ancestors_are_types_and_the_inheritance_walk_reads_them() -> Result<(), PBmmReadError> {
    let model = rm_model()?;
    let element = class(&model, "ELEMENT");
    let ancestors = element.ancestors().expect("ELEMENT states ancestors");
    assert!(
        ancestors
            .values()
            .any(|a| matches!(a, BmmModelType::BmmSimpleType(_))),
        "no simple-type ancestor materialised for ELEMENT",
    );
    assert!(element.has_ancestor_class("LOCATABLE"));
    assert!(element.has_ancestor_class("locatable"));
    assert!(!element.has_ancestor_class("DV_TEXT"));
    assert!(element.all_ancestors().iter().any(|n| n == "ITEM"));
    Ok(())
}

/// `master13-model_semantics.adoc` §Generic Inheritance: an ancestor that is a
/// generic class materialises as a generic TYPE carrying its parameter list —
/// `DV_INTERVAL` inherits `Interval<T>` in the vendored RM chain, where the
/// persisted form names the ancestor without a substitution, so the ancestor type
/// is the parent's fully open one.
#[test]
fn a_generic_ancestor_materialises_as_a_generic_type() -> Result<(), PBmmReadError> {
    let model = rm_model()?;
    let interval = class(&model, "DV_INTERVAL");
    let ancestors = interval.ancestors().expect("DV_INTERVAL states ancestors");
    let generic = ancestors
        .values()
        .find_map(|a| match a {
            BmmModelType::BmmGenericType(generic) => Some(generic),
            BmmModelType::BmmSimpleType(_) => None,
        })
        .expect("DV_INTERVAL inherits a generic type");
    assert_eq!(generic.base_class.name, "Interval");
    assert_eq!(
        generic
            .generic_parameters
            .iter()
            .map(BmmUnitaryType::type_name)
            .collect::<Vec<_>>(),
        vec!["T".to_owned()],
        "the ancestor's parameter substitution was not carried",
    );
    Ok(())
}

/// `master07-core-classes.adoc` §Range-Constrained Classes + §7.8: enumeration
/// values are literal-value objects, not raw JSON — `item_values` is
/// `List<BMM_PRIMITIVE_VALUE>` (`…bmm3.bmm_enumeration.adoc` §Attributes),
/// redefined to `List<BMM_INTEGER_VALUE>` by the integer form.
#[test]
fn an_enumeration_lands_typed_item_values_and_a_name_map() -> Result<(), PBmmReadError> {
    let model = rm_model()?;
    let BmmClass::BmmSimpleClass(BmmSimpleClass::BmmEnumeration(enumeration)) =
        class(&model, "PROPORTION_KIND")
    else {
        panic!("PROPORTION_KIND did not materialise as an enumeration");
    };
    let BmmEnumeration::BmmEnumerationInteger(integer) = enumeration else {
        panic!("PROPORTION_KIND is an integer enumeration");
    };
    assert_eq!(integer.item_names.as_ref().map_or(0, Vec::len), 5);
    // The vendored PROPORTION_KIND states names only, so the spec's assumed
    // values (0, 1, 2, ...) are what the name map reports
    // (`…bmm3.bmm_enumeration.adoc` §Attributes).
    assert!(integer.item_values.as_ref().is_none_or(Vec::is_empty));
    let names = enumeration.name_map();
    assert_eq!(names.get("pk_ratio").map(String::as_str), Some("0"));
    assert_eq!(names.get("pk_percent").map(String::as_str), Some("2"));
    assert_eq!(names.len(), 5);
    Ok(())
}

/// `master08-core-features.adoc` §Overview: a v3 class carries its features in
/// `_features_` with the specific maps as subsets — and `flat_features` is
/// "Consolidated list of all feature definitions from this class and all
/// inheritance ancestors" (`…bmm3.bmm_class.adoc` §Functions).
#[test]
fn class_features_are_populated_and_flatten_over_ancestors() -> Result<(), PBmmReadError> {
    let model = rm_model()?;
    let element = class(&model, "ELEMENT");
    let properties = element.properties().expect("ELEMENT declares properties");
    assert!(properties.contains_key("value"));
    assert_eq!(element.features().len(), properties.len());
    let flat = element.flat_features();
    assert!(
        flat.len() > element.features().len(),
        "flat_features did not pick up inherited features ({} vs {})",
        flat.len(),
        element.features().len(),
    );
    // `name` is declared by LOCATABLE, an ancestor — the flat set has it, the
    // differential set does not.
    assert!(flat.iter().any(|f| f.name() == "name"));
    assert!(!element.features().iter().any(|f| f.name() == "name"));
    Ok(())
}

/// `master06-core-types.adoc` §Overview rows 2-3 + `…bmm3.bmm_model_type.adoc`
/// §Functions: a type answers `is_abstract` / `is_primitive` / `type_base_name`
/// from its base class.
#[test]
fn a_model_type_answers_abstractness_primitiveness_and_base_name()
-> Result<(), Box<dyn std::error::Error>> {
    let model = rm_model()?;
    // LOCATABLE is abstract in the RM; String is a primitive_types class.
    let locatable = class(&model, "LOCATABLE").r#type();
    assert!(locatable.is_abstract());
    assert!(!locatable.is_primitive());
    assert_eq!(locatable.type_base_name(), "LOCATABLE");

    let element = class(&model, "ELEMENT").r#type();
    assert!(!element.is_abstract());

    let value = class(&model, "ELEMENT")
        .properties()
        .ok_or("ELEMENT declares properties")?
        .get("value")
        .ok_or("ELEMENT.value")?;
    let BmmProperty::BmmUnitaryProperty(value) = value else {
        return Err("ELEMENT.value is a unitary property".into());
    };
    // DATA_VALUE is abstract, so the type of `value` is an abstract type.
    assert!(value.r#type.is_abstract());
    assert_eq!(value.r#type.type_name(), "DATA_VALUE");
    Ok(())
}

/// `…bmm3.bmm_container_type.adoc` §Functions: `unitary_type()` returns
/// `_item_type_`, `effective_type()` returns the item's effective type, and
/// `flattened_type_list` is the item's — while `…bmm3.bmm_unitary_type.adoc`
/// §Functions makes `unitary_type()` the identity on a unitary type.
#[test]
fn a_container_type_reduces_to_its_item_type() -> Result<(), Box<dyn std::error::Error>> {
    let model = rm_model()?;
    let items = class(&model, "ITEM_LIST")
        .properties()
        .ok_or("ITEM_LIST declares properties")?
        .get("items")
        .ok_or("ITEM_LIST.items")?;
    let BmmProperty::BmmContainerProperty(items) = items else {
        return Err("ITEM_LIST.items is a container property".into());
    };
    let container = items.r#type();
    assert_eq!(container.type_name(), "List<ELEMENT>");
    assert_eq!(container.unitary_type().type_name(), "ELEMENT");
    assert_eq!(container.flattened_type_list(), vec!["ELEMENT".to_owned()]);
    assert_eq!(
        container
            .effective_type()
            .map(|effective| effective.type_name()),
        Some("ELEMENT".to_owned()),
    );
    // The whole-type dispatch agrees with the container-level one.
    let whole = BmmType::BmmContainerType(Box::new(container));
    assert_eq!(whole.unitary_type().type_name(), "ELEMENT");
    Ok(())
}

/// `…bmm3.bmm_generic_class.adoc` §Functions: `type()` generates a "fully open"
/// generic type, `generic_parameter_conformance_type` answers the parameter's
/// constraint (`Any` when unconstrained), and `…bmm3.bmm_generic_type.adoc`
/// §Functions distinguishes open from partially closed.
#[test]
fn a_generic_class_generates_its_fully_open_type() -> Result<(), Box<dyn std::error::Error>> {
    let model = rm_model()?;
    let BmmClass::BmmGenericClass(interval) = class(&model, "Interval") else {
        return Err("Interval is a generic class".into());
    };
    let open = interval.r#type();
    assert_eq!(open.type_name(), "Interval<T>");
    // Fully open: no parameter has been substituted.
    assert!(!open.is_partially_closed());
    assert!(!open.is_open(), "a fully open type is not closed");
    assert_eq!(
        interval.generic_parameter_conformance_type("T"),
        Some("Ordered".to_owned()),
    );
    // Case-insensitive parameter lookup, and no answer for a parameter the class
    // does not declare.
    assert_eq!(
        interval.generic_parameter_conformance_type("t"),
        Some("Ordered".to_owned()),
    );
    assert_eq!(interval.generic_parameter_conformance_type("Z"), None);

    // The DV_INTERVAL ancestor type substitutes T, so it IS partially closed
    // relative to its own parameter list only when the substitution is concrete;
    // the ancestor here carries the still-formal `T`.
    let signature = open.type_signature();
    assert_eq!(signature, "Interval<T:Ordered>");
    Ok(())
}

/// `master06-core-types.adoc` §Overview L41-45: a formal parameter is a unitary
/// type whose effective type is its constraint, "or if not set, `'Any'`"
/// (`…bmm3.bmm_parameter_type.adoc` §Functions) — the `None` case here, since
/// `Any` is a class of the model rather than a type object the parameter carries.
#[test]
fn a_formal_parameter_reduces_to_its_constraint() -> Result<(), Box<dyn std::error::Error>> {
    let model = rm_model()?;
    let BmmClass::BmmGenericClass(interval) = class(&model, "Interval") else {
        return Err("Interval is a generic class".into());
    };
    let parameter = interval
        .generic_parameters
        .get("T")
        .ok_or("Interval declares T")?;
    assert!(!parameter.is_abstract());
    assert!(!parameter.is_primitive());
    assert_eq!(
        parameter.effective_type().map(|e| e.type_name()),
        Some("Ordered".to_owned()),
    );
    assert_eq!(parameter.type_signature(), "T:Ordered");

    // An unconstrained parameter has no effective type object; its conformance
    // name is the `Any` top.
    let unconstrained =
        openehr_lang::v1_1::bmm3::core::entity::bmm_parameter_type::BmmParameterType {
            name: "U".to_owned(),
            type_constraint: None,
            inheritance_precursor: None,
        };
    assert_eq!(unconstrained.effective_type(), None);
    assert_eq!(unconstrained.conformance_type_name(), "Any");
    Ok(())
}

/// A hand-written schema exercising the three things the v3 materialisation
/// carries and the vendored RM 1.0.2 corpus does not state: generic inheritance
/// WITH a substitution, class routines, and a value-set constrained property
/// type.
///
/// `master04-syntax.adoc` §Inheritance (`ancestor_defs`), §Class Definitions
/// (`functions`), §Value-set Constraints (`value_constraint`).
const ROUTINE_SCHEMA: &str = r#"
    bmm_version = <"2.4">
    rm_publisher = <"openehr">
    schema_name = <"bmm3_routines">
    rm_release = <"1.0.2">
    packages = <
        ["test"] = <
            name = <"test">
            classes = <"String", "Boolean", "Integer", "SUPPLIER", "SUPPLIER_A", "SUPPLIER_B", "CODE_PHRASE", "GENERIC_PARENT", "GENERIC_CHILD_OPEN_T", "SERVICE">
        >
    >
    class_definitions = <
        ["String"] = < name = <"String"> >
        ["Boolean"] = < name = <"Boolean"> >
        ["Integer"] = < name = <"Integer"> >
        ["CODE_PHRASE"] = < name = <"CODE_PHRASE"> >
        ["SUPPLIER"] = < name = <"SUPPLIER"> >
        ["SUPPLIER_A"] = < name = <"SUPPLIER_A"> ancestors = <"SUPPLIER"> >
        ["SUPPLIER_B"] = < name = <"SUPPLIER_B"> ancestors = <"SUPPLIER"> >
        ["GENERIC_PARENT"] = <
            name = <"GENERIC_PARENT">
            generic_parameter_defs = <
                ["T"] = < name = <"T"> conforms_to_type = <"SUPPLIER"> >
                ["U"] = < name = <"U"> conforms_to_type = <"SUPPLIER"> >
            >
            properties = <
                ["parent_prop"] = (P_BMM_SINGLE_PROPERTY_OPEN) < name = <"parent_prop"> type = <"T"> >
            >
        >
        ["GENERIC_CHILD_OPEN_T"] = <
            name = <"GENERIC_CHILD_OPEN_T">
            ancestor_defs = <
                ["GENERIC_PARENT<T,SUPPLIER_B>"] = (P_BMM_GENERIC_TYPE) <
                    root_type = <"GENERIC_PARENT">
                    generic_parameters = <"T", "SUPPLIER_B">
                >
            >
            generic_parameter_defs = <
                ["T"] = < name = <"T"> conforms_to_type = <"SUPPLIER"> >
            >
            properties = <
                ["child_prop"] = (P_BMM_SINGLE_PROPERTY) < name = <"child_prop"> type = <"String"> >
            >
        >
        ["SERVICE"] = <
            name = <"SERVICE">
            properties = <
                ["language"] = (P_BMM_SINGLE_PROPERTY) <
                    name = <"language">
                    type_ref = <
                        type = <"CODE_PHRASE">
                        value_constraint = <"openEHR::languages">
                    >
                >
            >
            constants = <
                ["Max_retries"] = < name = <"Max_retries"> type = <"Integer"> value = <"3"> >
            >
            functions = <
                ["is_available"] = <
                    name = <"is_available">
                    result = (P_BMM_SIMPLE_TYPE) < type = <"Boolean"> >
                >
                ["has_code"] = <
                    name = <"has_code">
                    parameters = <
                        ["a_code"] = (P_BMM_SINGLE_FUNCTION_PARAMETER) < name = <"a_code"> type = <"String"> >
                    >
                    result = (P_BMM_SIMPLE_TYPE) < type = <"Boolean"> >
                >
                ["reset"] = <
                    name = <"reset">
                >
            >
        >
    >
"#;

/// The v3 model of [`ROUTINE_SCHEMA`].
fn routine_model() -> Result<BmmModel, PBmmReadError> {
    create_bmm3_model(&read_schema(ROUTINE_SCHEMA)?)
}

/// `master13-model_semantics.adoc` §Generic Inheritance: "the formal parameters
/// of the inheriting class may further constrain any of the ancestor type's
/// formal parameters", and a substituted parameter is carried — the binding the
/// v2.x `ancestors: Hash<String, BMM_CLASS>` cannot hold.
#[test]
fn a_generic_ancestor_carries_its_parameter_substitution() -> Result<(), Box<dyn std::error::Error>>
{
    let model = routine_model()?;
    let child = class(&model, "GENERIC_CHILD_OPEN_T");
    let ancestors = child
        .ancestors()
        .ok_or("GENERIC_CHILD_OPEN_T states ancestors")?;
    let parent = ancestors
        .get("GENERIC_PARENT")
        .ok_or("the generic ancestor")?;
    let BmmModelType::BmmGenericType(parent) = parent else {
        return Err("the ancestor is a generic type".into());
    };
    assert_eq!(
        parent
            .generic_parameters
            .iter()
            .map(BmmUnitaryType::type_name)
            .collect::<Vec<_>>(),
        vec!["T".to_owned(), "SUPPLIER_B".to_owned()],
        "the ancestor's `<T,SUPPLIER_B>` substitution was not carried",
    );
    // The still-open half is a formal parameter, the substituted half a model
    // type — so the ancestor type is partially closed
    // (`…bmm3.bmm_generic_type.adoc` §Functions).
    assert!(parent.is_partially_closed());
    assert!(!parent.is_open());
    assert_eq!(parent.type_name(), "GENERIC_PARENT<T,SUPPLIER_B>");
    Ok(())
}

/// `master08-core-features.adoc` §Functions and Procedures + §Class Definitions:
/// a persisted function with a result materialises as a `BMM_FUNCTION` (with its
/// `Result` variable, `arity()` and `signature()`), one without as a
/// `BMM_PROCEDURE` (`…bmm_persistence.p_bmm_function.adoc` §Attributes: a result
/// is "absent for procedures").
#[test]
fn class_routines_reach_the_model_with_signatures_and_arity()
-> Result<(), Box<dyn std::error::Error>> {
    let model = routine_model()?;
    let service = class(&model, "SERVICE");
    let functions = service.functions().ok_or("SERVICE declares functions")?;
    let procedures = service.procedures().ok_or("SERVICE declares procedures")?;
    assert_eq!(functions.len(), 2);
    assert_eq!(procedures.len(), 1);

    let is_available = functions
        .get("is_available")
        .ok_or("SERVICE.is_available")?;
    assert_eq!(is_available.arity(), 0);
    assert!(is_available.is_boolean());
    assert_eq!(is_available.signature().argument_types, None);
    assert_eq!(is_available.signature().result_type.type_name(), "Boolean");
    // `Inv_result_type`: `type = Result.type` (`…bmm3.bmm_function.adoc`
    // §Invariants).
    assert_eq!(is_available.result.r#type.type_name(), "Boolean");
    assert_eq!(is_available.result.name, "Result");

    let has_code = functions.get("has_code").ok_or("SERVICE.has_code")?;
    assert_eq!(has_code.arity(), 1);
    let signature = has_code.signature();
    let arguments = signature.argument_types.ok_or("has_code takes arguments")?;
    assert_eq!(
        arguments
            .item_types
            .iter()
            .map(|(name, r#type)| (name.clone(), r#type.type_name()))
            .collect::<Vec<_>>(),
        vec![("a_code".to_owned(), "String".to_owned())],
    );

    let reset = procedures.get("reset").ok_or("SERVICE.reset")?;
    assert_eq!(reset.arity(), 0);
    assert!(!reset.is_boolean());
    // A procedure's signature result is the built-in Status meta-type
    // (`…bmm3.bmm_procedure_type.adoc` §Description).
    assert!(reset.signature().result_type.is_some());

    // Every routine is also a feature of the class (`master07-core-classes.adoc`
    // §Overview — the specific maps are subsets of `_features_`).
    assert!(
        service
            .features()
            .iter()
            .any(|f| f.name() == "is_available")
    );
    assert!(service.features().iter().any(|f| f.name() == "reset"));
    Ok(())
}

/// `master07-core-classes.adoc` §Value-set Types: a persisted `value_constraint`
/// lands on the TYPE as a `BMM_VALUE_SET_SPEC`, split around `::`.
#[test]
fn a_value_set_constraint_lands_on_the_property_type() -> Result<(), Box<dyn std::error::Error>> {
    let model = routine_model()?;
    let language = class(&model, "SERVICE")
        .properties()
        .ok_or("SERVICE declares properties")?
        .get("language")
        .ok_or("SERVICE.language")?;
    let BmmProperty::BmmUnitaryProperty(language) = language else {
        return Err("SERVICE.language is a unitary property".into());
    };
    let BmmUnitaryType::BmmSimpleType(simple) = &language.r#type else {
        return Err("SERVICE.language is of a simple type".into());
    };
    let spec = simple
        .value_constraint
        .as_ref()
        .ok_or("the value-set constraint reached the type")?;
    assert_eq!(spec.resource_id, "openEHR");
    assert_eq!(spec.value_set_id, "languages");
    Ok(())
}

/// `master08-core-features.adoc` §Properties: a persisted constant materialises
/// as a `BMM_CONSTANT` static property whose `generator` carries the serialised
/// literal (`…bmm3.bmm_constant.adoc` §Attributes).
#[test]
fn a_class_constant_materialises_with_its_literal_generator()
-> Result<(), Box<dyn std::error::Error>> {
    let model = routine_model()?;
    let statics = class(&model, "SERVICE")
        .static_properties()
        .ok_or("SERVICE declares static properties")?;
    let openehr_lang::v1_1::bmm3::core::feature::bmm_static::BmmStatic::BmmConstant(constant) =
        statics.get("Max_retries").ok_or("SERVICE.Max_retries")?
    else {
        return Err("Max_retries is a constant".into());
    };
    assert_eq!(constant.r#type.type_name(), "Integer");
    assert_eq!(constant.is_nullable, Some(false));
    assert_eq!(constant.generator.value_literal(), "3");
    // `_syntax_` unset means the `json` default applies
    // (`…bmm3.bmm_literal_value.adoc` §Attributes).
    assert_eq!(constant.generator.syntax(), "json");
    Ok(())
}

/// The two generic-inheritance examples of
/// `master13-model_semantics.adoc` §Generic Inheritance, side by side:
/// constraint-narrowing (`DV_INTERVAL<T:DV_ORDERED>` over `Interval<T:Ordered>`)
/// and closed substitution (`TIMER_WAIT` over `WAIT<TIMER_EVENT>`).
const GENERIC_INHERITANCE_SCHEMA: &str = r#"
    bmm_version = <"2.4">
    rm_publisher = <"openehr">
    schema_name = <"bmm3_generic_inheritance">
    rm_release = <"1.0.2">
    packages = <
        ["test"] = <
            name = <"test">
            classes = <"String", "ORDERED", "DV_ORDERED", "EVENT", "TIMER_EVENT", "INTERVAL", "DV_INTERVAL", "WAIT", "TIMER_WAIT", "SHADOWING_WAIT">
        >
    >
    class_definitions = <
        ["String"] = < name = <"String"> >
        ["ORDERED"] = < name = <"ORDERED"> >
        ["DV_ORDERED"] = < name = <"DV_ORDERED"> ancestors = <"ORDERED"> >
        ["EVENT"] = < name = <"EVENT"> >
        ["TIMER_EVENT"] = < name = <"TIMER_EVENT"> ancestors = <"EVENT"> >
        ["INTERVAL"] = <
            name = <"INTERVAL">
            generic_parameter_defs = <
                ["T"] = < name = <"T"> conforms_to_type = <"ORDERED"> >
            >
            properties = <
                ["lower"] = (P_BMM_SINGLE_PROPERTY_OPEN) < name = <"lower"> type = <"T"> >
            >
        >
        ["DV_INTERVAL"] = <
            name = <"DV_INTERVAL">
            ancestor_defs = <
                ["INTERVAL<T>"] = (P_BMM_GENERIC_TYPE) <
                    root_type = <"INTERVAL">
                    generic_parameters = <"T">
                >
            >
            generic_parameter_defs = <
                ["T"] = < name = <"T"> conforms_to_type = <"DV_ORDERED"> >
            >
        >
        ["WAIT"] = <
            name = <"WAIT">
            generic_parameter_defs = <
                ["T"] = < name = <"T"> conforms_to_type = <"EVENT"> >
            >
            properties = <
                ["event"] = (P_BMM_SINGLE_PROPERTY_OPEN) < name = <"event"> type = <"T"> >
            >
        >
        ["TIMER_WAIT"] = <
            name = <"TIMER_WAIT">
            ancestor_defs = <
                ["WAIT<TIMER_EVENT>"] = (P_BMM_GENERIC_TYPE) <
                    root_type = <"WAIT">
                    generic_parameters = <"TIMER_EVENT">
                >
            >
        >
        ["SHADOWING_WAIT"] = <
            name = <"SHADOWING_WAIT">
            ancestor_defs = <
                ["WAIT<TIMER_EVENT>"] = (P_BMM_GENERIC_TYPE) <
                    root_type = <"WAIT">
                    generic_parameters = <"TIMER_EVENT">
                >
            >
            properties = <
                ["event"] = (P_BMM_SINGLE_PROPERTY) < name = <"event"> type = <"String"> >
            >
        >
    >
"#;

/// The property named `name` on `class`.
fn property<'a>(class: &'a BmmClass, name: &str) -> &'a BmmProperty {
    class
        .properties()
        .unwrap_or_else(|| panic!("{} declares properties", class.name()))
        .get(name)
        .unwrap_or_else(|| panic!("{}.{name} is missing", class.name()))
}

/// A unitary property's type name + synthesis flag.
fn unitary(property: &BmmProperty) -> (String, Option<bool>) {
    match property {
        BmmProperty::BmmUnitaryProperty(unitary) => (
            BmmType::from(unitary.r#type.clone()).type_name(),
            unitary.is_synthesised_generic,
        ),
        BmmProperty::BmmContainerProperty(_) => panic!("expected a unitary property"),
    }
}

/// `master13-model_semantics.adoc` §Generic Inheritance, the closed case:
/// `TIMER_WAIT` inheriting `WAIT<TIMER_EVENT>` gets `event` "synthesised new
/// within `TIMER_WAIT`" typed `TIMER_EVENT` rather than `T:EVENT`, "with the
/// meta-attribute `_is_synthesised_generic_` set `True`".
#[test]
fn a_closed_generic_ancestor_synthesises_its_substituted_property()
-> Result<(), Box<dyn std::error::Error>> {
    let model = create_bmm3_model(&read_schema(GENERIC_INHERITANCE_SCHEMA)?)?;
    let (type_name, synthesised) = unitary(property(class(&model, "TIMER_WAIT"), "event"));
    assert_eq!(type_name, "TIMER_EVENT");
    assert_eq!(synthesised, Some(true));
    // The ancestor keeps its own open property, unflagged.
    let (ancestor_type, ancestor_flag) = unitary(property(class(&model, "WAIT"), "event"));
    assert_eq!(ancestor_type, "T");
    assert_eq!(ancestor_flag, None);
    Ok(())
}

/// The constraint-narrowing case of the same section: `DV_INTERVAL
/// <T:DV_ORDERED>` inheriting `Interval<T:Ordered>` synthesises `lower` whose
/// "resulting type … is now `T:DV_ORDERED` rather than `T:Ordered` from the
/// parent".
#[test]
fn a_narrowed_generic_parameter_synthesises_its_property() -> Result<(), Box<dyn std::error::Error>>
{
    let model = create_bmm3_model(&read_schema(GENERIC_INHERITANCE_SCHEMA)?)?;
    let interval = class(&model, "DV_INTERVAL");
    let (type_name, synthesised) = unitary(property(interval, "lower"));
    assert_eq!(type_name, "T");
    assert_eq!(synthesised, Some(true));
    // The narrowing is what the synthesis carries: the descendant's own `T`
    // conforms to DV_ORDERED, the ancestor's to ORDERED.
    let BmmClass::BmmGenericClass(generic) = interval else {
        return Err("DV_INTERVAL is generic".into());
    };
    assert_eq!(
        generic.generic_parameter_conformance_type("T").as_deref(),
        Some("DV_ORDERED")
    );
    Ok(())
}

/// A property the descendant DECLARES is never overwritten by synthesis: the
/// section synthesises the ancestor's properties "within" the descendant,
/// which cannot mean displacing one it defines for itself.
#[test]
fn a_declared_property_is_not_displaced_by_synthesis() -> Result<(), Box<dyn std::error::Error>> {
    let model = create_bmm3_model(&read_schema(GENERIC_INHERITANCE_SCHEMA)?)?;
    let (type_name, synthesised) = unitary(property(class(&model, "SHADOWING_WAIT"), "event"));
    assert_eq!(type_name, "String");
    assert_eq!(synthesised, None);
    Ok(())
}

/// `master06-core-types.adoc` §Type Conformance, the base-class branch:
/// "either type names are identical, or else `a_desc_type` has `an_anc_type`
/// in its ancestors", with the `Any` top implicit
/// (`master05-core-model.adoc` §The Any Class and Type).
#[test]
fn model_type_conformance_follows_the_base_class_test() -> Result<(), Box<dyn std::error::Error>> {
    let model = create_bmm3_model(&read_schema(GENERIC_INHERITANCE_SCHEMA)?)?;
    assert!(model.type_conforms_to("DV_ORDERED", "DV_ORDERED"));
    // Case-insensitive: §Naming Convention.
    assert!(model.type_conforms_to("dv_ordered", "ORDERED"));
    assert!(model.type_conforms_to("TIMER_EVENT", "EVENT"));
    assert!(!model.type_conforms_to("ORDERED", "DV_ORDERED"));
    assert!(!model.type_conforms_to("EVENT", "TIMER_EVENT"));
    // Every class reaches the implicit `Any` top.
    assert!(model.type_conforms_to("TIMER_EVENT", "Any"));
    Ok(())
}

/// The generic branches of the same section: both generic with matching
/// parameter counts and recursively conformant arguments; and "the case where
/// anc type is not provided in generic form, but desc is, e.g.
/// `Interval<Integer>` conforms to `Interval`". A non-generic descendant
/// against a generic ancestor does NOT conform (the section's final `else`).
#[test]
fn model_type_conformance_compares_generic_parameters() -> Result<(), Box<dyn std::error::Error>> {
    let model = create_bmm3_model(&read_schema(GENERIC_INHERITANCE_SCHEMA)?)?;
    assert!(model.type_conforms_to("INTERVAL<DV_ORDERED>", "INTERVAL<ORDERED>"));
    assert!(!model.type_conforms_to("INTERVAL<ORDERED>", "INTERVAL<DV_ORDERED>"));
    assert!(model.type_conforms_to("INTERVAL<DV_ORDERED>", "INTERVAL"));
    assert!(!model.type_conforms_to("INTERVAL", "INTERVAL<ORDERED>"));
    // The descendant's own root must still pass the base-class test.
    assert!(!model.type_conforms_to("WAIT<EVENT>", "INTERVAL<ORDERED>"));
    Ok(())
}

/// `BMM_MODEL.all_ancestor_classes` walks the lineage transitively and
/// excludes the class itself; `property_definition` resolves an INHERITED
/// property through the flattened class.
#[test]
fn model_navigation_walks_ancestors_and_flat_properties() -> Result<(), Box<dyn std::error::Error>>
{
    let model = create_bmm3_model(&read_schema(GENERIC_INHERITANCE_SCHEMA)?)?;
    let ancestors = model.all_ancestor_classes("TIMER_EVENT");
    assert!(
        ancestors.iter().any(|name| name == "EVENT"),
        "{ancestors:?}"
    );
    assert!(
        !ancestors.iter().any(|name| name == "TIMER_EVENT"),
        "the class itself is excluded: {ancestors:?}"
    );
    // A parentless class closes with the implicit `Any` top.
    assert_eq!(model.all_ancestor_classes("EVENT"), vec!["Any".to_owned()]);
    // `event` is declared on WAIT and reached through TIMER_WAIT's lineage.
    assert!(model.property_definition("TIMER_WAIT", "event").is_some());
    assert!(model.property_definition("TIMER_WAIT", "no_such").is_none());
    assert!(model.class_definition("timer_wait").is_some());
    Ok(())
}
