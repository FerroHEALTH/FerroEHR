// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![expect(
    clippy::panic_in_result_fn,
    reason = "the Book ch11 test shape: `?` propagates the read/resolve/model plumbing while the assertions ARE the test — an assertion panic is how these tests fail"
)]
use std::collections::BTreeMap;

use super::read_schema;
use crate::v1_1::bmm_persistence::error::PBmmReadError;
use crate::v1_1::bmm_persistence::p_bmm_base_type::PBmmBaseType;
use crate::v1_1::bmm_persistence::p_bmm_class::PBmmClass;
use crate::v1_1::bmm_persistence::p_bmm_container_property::PBmmContainerProperty;
use crate::v1_1::bmm_persistence::p_bmm_container_type::PBmmContainerType;
use crate::v1_1::bmm_persistence::p_bmm_indexed_container_type::PBmmIndexedContainerType;
use crate::v1_1::bmm_persistence::p_bmm_property::PBmmProperty;
use crate::v1_1::bmm_persistence::p_bmm_schema::PBmmSchema;
use crate::v1_1::bmm_persistence::p_bmm_type::PBmmType;
use openehr_base::v1_3::prelude::Interval;
use openehr_base::v1_3::prelude::ProperInterval;

/// The four identifying header items every fixture below needs.
const HEADER: &str = r#"
    bmm_version = <"2.4">
    rm_publisher = <"openehr">
    schema_name = <"adltest">
    rm_release = <"1.0.2">
"#;

/// Reads `HEADER` plus `body`.
fn read(body: &str) -> Result<PBmmSchema, PBmmReadError> {
    read_schema(&format!("{HEADER}{body}"))
}

/// The class definition named `name` in `schema`.
fn class<'a>(schema: &'a PBmmSchema, name: &str) -> &'a PBmmClass {
    schema
        .class_definitions
        .iter()
        .flat_map(BTreeMap::values)
        .find(|class| class.name() == name)
        .expect("the fixture defines the class")
}

/// The property named `property` of the class named `class_name`.
fn property<'a>(schema: &'a PBmmSchema, class_name: &str, property: &str) -> &'a PBmmProperty {
    class(schema, class_name)
        .properties()
        .expect("the class declares properties")
        .get(property)
        .expect("the class declares the property")
}

#[test]
fn header_items_read_verbatim() -> Result<(), PBmmReadError> {
    // master04-syntax.adoc §Header Items, verbatim (including `model_name`,
    // which P_BMM_SCHEMA does not declare and the reader tolerates).
    let schema = read_schema(
        r#"
        bmm_version = <"2.3">
        rm_publisher = <"openehr">
        schema_name = <"adltest">
        rm_release = <"1.0.2">
        model_name = <"TEST_PKG">
        schema_revision = <"1.0.36">
        schema_lifecycle_state = <"stable">
        schema_description = <"openEHR schema to support test archetypes">
    "#,
    )?;
    assert_eq!(schema.bmm_version, "2.3");
    assert_eq!(schema.schema_id(), "openehr_adltest_1.0.2");
    assert_eq!(schema.schema_revision, "1.0.36");
    assert_eq!(schema.schema_lifecycle_state, "stable");
    // Declared 1..1 but omitted by the chapter's own example.
    assert_eq!(schema.schema_author, "");
    Ok(())
}

#[test]
fn a_missing_identifying_header_is_refused() {
    let error = read_schema(r#"bmm_version = <"2.4">"#).expect_err("rm_publisher is missing");
    assert_eq!(
        error,
        PBmmReadError::MissingAttribute {
            path: String::new(),
            attribute: "rm_publisher",
        }
    );
}

#[test]
fn an_undeclared_attribute_is_refused_by_name() {
    let error = read(r#"not_a_bmm_attribute = <"x">"#).expect_err("the attribute is unknown");
    assert_eq!(
        error,
        PBmmReadError::UnknownAttribute {
            path: String::new(),
            attribute: "not_a_bmm_attribute".to_owned(),
        }
    );
}

#[test]
fn includes_are_keyed_by_their_own_id() -> Result<(), PBmmReadError> {
    // master04-syntax.adoc §Inclusions, verbatim.
    let schema = read(
        r#"
        includes = <
            ["1"] = <
                id = <"openehr_basic_types_1.0.2">
            >
        >
    "#,
    )?;
    let includes = schema.includes.expect("the schema declares an include");
    assert_eq!(includes.len(), 1);
    assert_eq!(
        includes
            .get("openehr_basic_types_1.0.2")
            .map(|spec| spec.id.as_str()),
        Some("openehr_basic_types_1.0.2")
    );
    Ok(())
}

#[test]
fn packages_read_recursively_and_only_top_level_ids_may_be_paths() -> Result<(), PBmmReadError> {
    // master04-syntax.adoc §Package Definition, verbatim.
    let schema = read(
        r#"
        packages = <
            ["org.openehr.test_pkg"] = <
                name = <"org.openehr.test_pkg">
                classes = <"WHOLE", "SOME_TYPE", "BOOK">
            >
        >
    "#,
    )?;
    let package = schema
        .packages
        .get("org.openehr.test_pkg")
        .expect("the top-level package reads");
    assert_eq!(package.classes.as_ref().map_or(0, Vec::len), 3);

    let error = read(
        r#"
        packages = <
            ["ParentPackage"] = <
                name = <"ParentPackage">
                packages = <
                    ["invalid.ChildPackage"] = <
                        name = <"invalid.ChildPackage">
                    >
                >
            >
        >
    "#,
    )
    .expect_err("a nested package id may not be a path");
    assert!(
        matches!(error, PBmmReadError::QualifiedNestedPackage { .. }),
        "expected a qualified-nested-package refusal, got {error:?}"
    );
    Ok(())
}

#[test]
fn a_key_that_disagrees_with_the_name_attribute_is_refused() {
    let error = read(
        r#"
        packages = <
            ["ParentPackage"] = <
                name = <"OtherName">
            >
        >
    "#,
    )
    .expect_err("the key and name disagree");
    assert!(
        matches!(error, PBmmReadError::KeyNameMismatch { .. }),
        "expected a key/name refusal, got {error:?}"
    );
}

#[test]
fn single_and_container_properties_read_the_master04_examples() -> Result<(), PBmmReadError> {
    // master04-syntax.adoc §Class properties + §Container Properties.
    let schema = read(
        r#"
        class_definitions = <
            ["ELEMENT"] = <
                name = <"ELEMENT">
                ancestors = <"ITEM">
                properties = <
                    ["null_flavour"] = (P_BMM_SINGLE_PROPERTY) <
                        name = <"null_flavour">
                        type = <"DV_CODED_TEXT">
                        is_mandatory = <True>
                    >
                    ["items"] = (P_BMM_CONTAINER_PROPERTY) <
                        name = <"items">
                        type_def = <
                            container_type = <"List">
                            type = <"ITEM">
                        >
                        cardinality = <|>=1|>
                        is_mandatory = <True>
                    >
                    ["custom_actions"] = (P_BMM_INDEXED_CONTAINER_PROPERTY) <
                        name = <"custom_actions">
                        type_def = <
                            container_type = <"Hash">
                            index_type = <"String">
                            type = <"EVENT_ACTION">
                        >
                        cardinality = <|>=0|>
                    >
                >
            >
        >
    "#,
    )?;
    assert_eq!(class(&schema, "ELEMENT").ancestors(), ["ITEM"]);
    assert_eq!(class(&schema, "ELEMENT").uid(), Some(1));
    assert_eq!(
        class(&schema, "ELEMENT").source_schema_id(),
        Some("openehr_adltest_1.0.2")
    );

    let PBmmProperty::PBmmSingleProperty(single) = property(&schema, "ELEMENT", "null_flavour")
    else {
        panic!("null_flavour is a P_BMM_SINGLE_PROPERTY");
    };
    assert_eq!(single.r#type.as_deref(), Some("DV_CODED_TEXT"));
    assert_eq!(single.is_mandatory, Some(true));

    let PBmmProperty::PBmmContainerProperty(PBmmContainerProperty::PBmmContainerProperty(
        container,
    )) = property(&schema, "ELEMENT", "items")
    else {
        panic!("items is a P_BMM_CONTAINER_PROPERTY");
    };
    let type_def = container
        .type_def
        .as_ref()
        .expect("the container states a type_def");
    assert_eq!(type_def.as_type_string(), "List<ITEM>");
    // `|>=1|` is lower-bounded and upper-unbounded.
    let Some(Interval::ProperInterval(ProperInterval::ProperInterval(cardinality))) =
        container.cardinality.as_ref()
    else {
        panic!("the cardinality reads as a proper interval");
    };
    assert_eq!(cardinality.lower, Some(1));
    assert!(cardinality.lower_included);
    assert_eq!(cardinality.upper, None);
    assert!(cardinality.upper_unbounded);

    let PBmmProperty::PBmmContainerProperty(PBmmContainerProperty::PBmmIndexedContainerProperty(
        indexed,
    )) = property(&schema, "ELEMENT", "custom_actions")
    else {
        panic!("custom_actions is a P_BMM_INDEXED_CONTAINER_PROPERTY");
    };
    assert_eq!(
        indexed
            .type_def
            .as_ref()
            .map(PBmmIndexedContainerType::as_type_string)
            .as_deref(),
        Some("Hash<String,EVENT_ACTION>")
    );
    Ok(())
}

#[test]
fn a_property_without_a_type_marker_is_refused() {
    let error = read(
        r#"
        class_definitions = <
            ["ELEMENT"] = <
                name = <"ELEMENT">
                properties = <
                    ["value"] = <
                        name = <"value">
                        type = <"DATA_VALUE">
                    >
                >
            >
        >
    "#,
    )
    .expect_err("§Class properties requires the marker");
    assert_eq!(
        error,
        PBmmReadError::MissingTypeMarker {
            path: "class_definitions/ELEMENT/properties/value".to_owned(),
            expected: "P_BMM_PROPERTY",
        }
    );
}

#[test]
fn crazy_type_reads_every_mixed_generic_parameter_form() -> Result<(), PBmmReadError> {
    // master04-syntax.adoc §Generic Classes CRAZY_TYPE, verbatim.
    let schema = read(
        r#"
        class_definitions = <
            ["CRAZY_TYPE"] = <
                name = <"CRAZY_TYPE">
                ancestors = <"Any">
                properties = <
                    ["range"] = (P_BMM_GENERIC_PROPERTY) <
                        name = <"range">
                        type_def = <
                            root_type = <"REFERENCE_RANGE">
                            generic_parameter_defs = <
                                ["T"] = (P_BMM_GENERIC_TYPE) <
                                    root_type = <"DV_INTERVAL">
                                    generic_parameters = <"DV_QUANTITY">
                                >
                                ["U"] = (P_BMM_SIMPLE_TYPE) <
                                    type = <"Integer">
                                >
                                ["V"] = (P_BMM_CONTAINER_TYPE) <
                                    type = <"DV_QUANTITY">
                                    container_type = <"List">
                                >
                                ["W"] = (P_BMM_CONTAINER_TYPE) <
                                    type_def = (P_BMM_GENERIC_TYPE) <
                                        root_type = <"DV_INTERVAL">
                                        generic_parameters = <"DV_QUANTITY">
                                    >
                                    container_type = <"List">
                                >
                            >
                        >
                    >
                >
            >
        >
    "#,
    )?;
    let PBmmProperty::PBmmGenericProperty(generic) = property(&schema, "CRAZY_TYPE", "range")
    else {
        panic!("range is a P_BMM_GENERIC_PROPERTY");
    };
    let type_def = generic.type_def.as_ref().expect("range states a type_def");
    assert_eq!(
        type_def.as_type_string(),
        "REFERENCE_RANGE<DV_INTERVAL<DV_QUANTITY>,Integer,List<DV_QUANTITY>,List<DV_INTERVAL<DV_QUANTITY>>>"
    );
    Ok(())
}

#[test]
fn value_set_constraints_read_in_both_positions() -> Result<(), PBmmReadError> {
    // master04-syntax.adoc §Value-set Constraints, both forms.
    let schema = read(
        r#"
        class_definitions = <
            ["TRANSLATION_DETAILS"] = <
                name = <"TRANSLATION_DETAILS">
                properties = <
                    ["encoding"] = (P_BMM_SINGLE_PROPERTY) <
                        name = <"encoding">
                        type_ref = <
                            type = <"CODE_PHRASE">
                            value_constraint = <"openEHR::languages">
                        >
                    >
                    ["language"] = (P_BMM_CONTAINER_PROPERTY) <
                        name = <"language">
                        type_def = <
                            container_type = <"List">
                            type_def = (P_BMM_SIMPLE_TYPE) <
                                type = <"Coding">
                                value_constraint = <"hl7::Languages">
                            >
                        >
                    >
                >
            >
        >
    "#,
    )?;
    let PBmmProperty::PBmmSingleProperty(single) =
        property(&schema, "TRANSLATION_DETAILS", "encoding")
    else {
        panic!("encoding is a P_BMM_SINGLE_PROPERTY");
    };
    let type_ref = single
        .type_ref
        .as_ref()
        .expect("encoding states a type_ref");
    assert_eq!(type_ref.r#type, "CODE_PHRASE");
    assert_eq!(
        type_ref.value_constraint.as_deref(),
        Some("openEHR::languages")
    );

    let PBmmProperty::PBmmContainerProperty(PBmmContainerProperty::PBmmContainerProperty(
        container,
    )) = property(&schema, "TRANSLATION_DETAILS", "language")
    else {
        panic!("language is a P_BMM_CONTAINER_PROPERTY");
    };
    assert_eq!(
        container
            .type_def
            .as_ref()
            .map(PBmmContainerType::as_type_string)
            .as_deref(),
        Some("List<Coding>")
    );
    Ok(())
}

#[test]
fn functions_constants_and_invariants_read() -> Result<(), PBmmReadError> {
    // master04-syntax.adoc §Functions, §Constants, §Invariants, verbatim.
    let schema = read(
        r#"
        class_definitions = <
            ["BMM_MODEL"] = <
                name = <"BMM_MODEL">
                ancestors = <"BMM_PACKAGE_CONTAINER">
                functions = <
                    ["class_definition"] = <
                        name = <"class_definition">
                        parameters = <
                            ["a_name"] = (P_BMM_SINGLE_FUNCTION_PARAMETER) <
                                name = <"a_name">
                                type = <"String">
                            >
                        >
                        result = (P_BMM_SIMPLE_TYPE) <
                            type = <"BMM_CLASS">
                        >
                    >
                >
            >
            ["OPENEHR_DEFINITIONS"] = <
                name = <"OPENEHR_DEFINITIONS">
                constants = <
                    ["Local_terminology_id"] = <
                        name = <"Local_terminology_id">
                        type = <"String">
                        value = <"local">
                    >
                >
            >
            ["LOCATABLE"] = <
                name = <"LOCATABLE">
                invariants = <
                    ["Links_valid"] = <"links /= Void implies not links.is_empty">
                    ["Archetype_node_id_valid"] = <"not archetype_node_id.is_empty">
                >
            >
        >
    "#,
    )?;
    let functions = class(&schema, "BMM_MODEL")
        .functions()
        .expect("BMM_MODEL declares functions");
    let function = functions
        .get("class_definition")
        .expect("class_definition reads");
    assert_eq!(
        function
            .result
            .as_ref()
            .map(PBmmType::as_type_string)
            .as_deref(),
        Some("BMM_CLASS")
    );
    assert_eq!(function.parameters.as_ref().map(BTreeMap::len), Some(1));

    let constants = class(&schema, "OPENEHR_DEFINITIONS")
        .constants()
        .expect("OPENEHR_DEFINITIONS declares constants");
    assert_eq!(
        constants
            .get("Local_terminology_id")
            .and_then(|constant| constant.value.as_deref()),
        Some("local")
    );

    let invariants = class(&schema, "LOCATABLE")
        .invariants()
        .expect("LOCATABLE declares invariants");
    assert_eq!(invariants.len(), 2);
    assert_eq!(
        invariants.get("Links_valid").map(String::as_str),
        Some("links /= Void implies not links.is_empty")
    );
    Ok(())
}

#[test]
fn enumerations_read_names_and_values() -> Result<(), PBmmReadError> {
    // master04-syntax.adoc §Enumerated Types, verbatim.
    let schema = read(
        r#"
        class_definitions = <
            ["PROPORTION_KIND_2"] = (P_BMM_ENUMERATION_INTEGER) <
                name = <"PROPORTION_KIND_2">
                ancestors = <"Integer">
                item_names = <"pk_ratio", "pk_unitary", "pk_percent", "pk_fraction">
                item_values = <0, 1001, 1002, 1003>
            >
            ["MAGNITUDE_STATUS"] = (P_BMM_ENUMERATION_STRING) <
                name = <"MAGNITUDE_STATUS">
                ancestors = <"String", ...>
                item_names = <"le", "ge", "eq", "approx_eq">
                item_values = <"<=", ">=", "=", "~">
            >
        >
    "#,
    )?;
    let PBmmClass::PBmmEnumeration(integer) = class(&schema, "PROPORTION_KIND_2") else {
        panic!("PROPORTION_KIND_2 is a P_BMM_ENUMERATION_INTEGER");
    };
    assert_eq!(integer.item_names().len(), 4);
    assert_eq!(integer.item_values().len(), 4);
    assert_eq!(integer.underlying_type_name(), "INTEGER");

    let PBmmClass::PBmmEnumeration(string) = class(&schema, "MAGNITUDE_STATUS") else {
        panic!("MAGNITUDE_STATUS is a P_BMM_ENUMERATION_STRING");
    };
    assert_eq!(string.underlying_type_name(), "STRING");
    // The open-list marker of `ancestors = <"String", ...>` is not an item.
    assert_eq!(class(&schema, "MAGNITUDE_STATUS").ancestors(), ["String"]);
    Ok(())
}

#[test]
fn generic_inheritance_reads_ancestor_defs() -> Result<(), PBmmReadError> {
    // master04-syntax.adoc §Inheritance, GENERIC_CHILD_OPEN_T.
    let schema = read(
        r#"
        class_definitions = <
            ["GENERIC_CHILD_OPEN_T"] = <
                name = <"GENERIC_CHILD_OPEN_T">
                ancestor_defs = <
                    ["GENERIC_PARENT<T,SUPPLIER_B>"] = (P_BMM_GENERIC_TYPE) <
                        root_type = <"GENERIC_PARENT">
                        generic_parameters = <"T", "SUPPLIER_B">
                    >
                >
                generic_parameter_defs = <
                    ["T"] = <
                        name = <"T">
                        conforms_to_type = <"SUPPLIER">
                    >
                >
                properties = <
                    ["gen_child_open_t_prop"] = (P_BMM_SINGLE_PROPERTY) <
                        name = <"gen_child_open_t_prop">
                        type = <"String">
                    >
                >
            >
        >
    "#,
    )?;
    let child = class(&schema, "GENERIC_CHILD_OPEN_T");
    assert!(child.is_generic());
    let defs = child
        .ancestor_defs()
        .expect("the class declares ancestor_defs");
    assert_eq!(defs.len(), 1);
    let signature = defs
        .values()
        .next()
        .map(PBmmBaseType::as_type_string)
        .expect("the ancestor_defs entry reads");
    assert_eq!(signature, "GENERIC_PARENT<T,SUPPLIER_B>");
    Ok(())
}

#[test]
fn a_persisted_interface_reads_its_name_documentation_and_functions() -> Result<(), PBmmReadError> {
    // master02-overview.adoc §Conceptual Approach: the model "can also
    // represent pure interfaces via P_BMM_INTERFACE, i.e. class-like
    // definitions that declare only functions and carry no state".
    let schema = read(
        r#"
        class_definitions = <
            ["Math"] = (P_BMM_INTERFACE) <
                name = <"Math">
                documentation = <"Mathematical functions.">
                functions = <
                    ["abs"] = <
                        name = <"abs">
                        parameters = <
                            ["v"] = (P_BMM_SINGLE_FUNCTION_PARAMETER) <
                                name = <"v">
                                type = <"Real">
                            >
                        >
                        result = (P_BMM_SIMPLE_TYPE) <
                            type = <"Real">
                        >
                    >
                >
            >
        >
    "#,
    )?;
    let interface = class(&schema, "Math");
    assert!(matches!(interface, PBmmClass::PBmmInterface(_)));
    assert_eq!(interface.documentation(), Some("Mathematical functions."));
    let functions = interface.functions().expect("the declared functions");
    assert_eq!(functions.keys().collect::<Vec<&String>>(), ["abs"]);
    // An interface carries no state and no processing-stamped attribute.
    assert!(interface.properties().is_none());
    assert!(interface.is_abstract());
    assert_eq!(interface.source_schema_id(), None);
    assert_eq!(interface.uid(), None);
    Ok(())
}

#[test]
fn a_persisted_interface_refuses_a_state_carrying_attribute() {
    // `…p_bmm_interface.adoc` §Attributes declares `name` and `functions`
    // only (plus the inherited `documentation`), so a property block on an
    // interface is not part of the P_BMM model.
    let error = read(
        r#"
        class_definitions = <
            ["Env"] = (P_BMM_INTERFACE) <
                name = <"Env">
                properties = <
                    ["home"] = (P_BMM_SINGLE_PROPERTY) <
                        name = <"home">
                        type = <"String">
                    >
                >
            >
        >
    "#,
    )
    .expect_err("an interface declares no properties");
    assert_eq!(
        error,
        PBmmReadError::UnknownAttribute {
            path: "class_definitions/Env".to_owned(),
            attribute: "properties".to_owned(),
        }
    );
}

#[test]
fn primitive_types_read_as_ordinary_class_definitions() -> Result<(), PBmmReadError> {
    // master04-syntax.adoc §Classes for Primitive Types, verbatim.
    let schema = read(
        r#"
        primitive_types = <
            ["Any"] = <
                name = <"Any">
                is_abstract = <True>
            >
            ["Ordered"] = <
                name = <"Ordered">
                is_abstract = <True>
                ancestors = <"Any">
            >
        >
    "#,
    )?;
    assert_eq!(schema.primitive_types.as_ref().map_or(0, BTreeMap::len), 2);
    assert!(
        schema
            .primitive_types
            .iter()
            .flat_map(BTreeMap::values)
            .all(PBmmClass::is_abstract)
    );
    // uid numbering runs over primitive_types first, then class_definitions.
    assert_eq!(
        schema
            .primitive_types
            .iter()
            .flat_map(BTreeMap::values)
            .map(PBmmClass::uid)
            .collect::<Vec<Option<i32>>>(),
        [Some(1), Some(2)]
    );
    Ok(())
}
