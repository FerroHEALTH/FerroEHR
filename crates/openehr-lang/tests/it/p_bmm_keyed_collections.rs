// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The keyed `P_BMM` collections of SPECLANG-16: generic parameters keep their
//! declaration order through both serialisations and the canonical JSON codec
//! and bind by name, and `ancestor_defs` carries an ancestor of every
//! `P_BMM_BASE_TYPE` form.
//!
//! `P_BMM_GENERIC_TYPE.generic_parameter_defs` is "keyed by formal generic
//! parameter name" and "The order must match the order of the owning class's
//! formal generic parameter declarations"
//! (`LANG/docs/UML/classes/org.openehr.lang.bmm_persistence.p_bmm_generic_type.adoc`
//! §Attributes, SPECLANG-16). The fixture declares `PAIR`'s formals as `V`
//! then `K`, against their key order, so a sorted map would fail every check.

#![expect(
    clippy::panic,
    clippy::panic_in_result_fn,
    clippy::expect_used,
    reason = "test diagnostics: a fixture that does not read, or an assertion, is how these tests fail"
)]

use openehr_lang::v1_1::bmm::core::bmm_property::BmmProperty;
use openehr_lang::v1_1::bmm_persistence::create_bmm3_model::create_bmm3_model;
use openehr_lang::v1_1::bmm_persistence::create_model::create_bmm_model;
use openehr_lang::v1_1::bmm_persistence::error::PBmmReadError;
use openehr_lang::v1_1::bmm_persistence::json_reader::read_schema_json;
use openehr_lang::v1_1::bmm_persistence::p_bmm_base_type::PBmmBaseType;
use openehr_lang::v1_1::bmm_persistence::p_bmm_class::PBmmClass;
use openehr_lang::v1_1::bmm_persistence::p_bmm_property::PBmmProperty;
use openehr_lang::v1_1::bmm_persistence::p_bmm_schema::PBmmSchema;
use openehr_lang::v1_1::bmm_persistence::reader::read_schema;

/// `PAIR<V, K>` and a `HOLDER` whose `pair` binds it in declaration order.
const ODIN_SCHEMA: &str = r#"
bmm_version = <"2.4">
rm_publisher = <"test">
schema_name = <"generic_order">
rm_release = <"1.0.0">
packages = <
    ["org.test"] = <
        name = <"org.test">
        classes = <"Any", "Integer", "String", "PAIR", "HOLDER">
    >
>
primitive_types = <
    ["Any"] = <
        name = <"Any">
    >
    ["Integer"] = <
        name = <"Integer">
        ancestors = <"Any">
    >
    ["String"] = <
        name = <"String">
        ancestors = <"Any">
    >
>
class_definitions = <
    ["PAIR"] = <
        name = <"PAIR">
        ancestors = <"Any">
        generic_parameter_defs = <
            ["V"] = <
                name = <"V">
            >
            ["K"] = <
                name = <"K">
            >
        >
    >
    ["HOLDER"] = <
        name = <"HOLDER">
        ancestors = <"Any">
        properties = <
            ["pair"] = (P_BMM_GENERIC_PROPERTY) <
                name = <"pair">
                type_def = <
                    root_type = <"PAIR">
                    generic_parameter_defs = <
                        ["V"] = (P_BMM_SIMPLE_TYPE) <
                            type = <"Integer">
                        >
                        ["K"] = (P_BMM_SIMPLE_TYPE) <
                            type = <"String">
                        >
                    >
                >
            >
        >
    >
>
"#;

/// [`ODIN_SCHEMA`] in JSON.
const JSON_SCHEMA: &str = r#"{
    "bmm_version": "2.4",
    "rm_publisher": "test",
    "schema_name": "generic_order",
    "rm_release": "1.0.0",
    "packages": {
        "org.test": {"name": "org.test", "classes": ["Any", "Integer", "String", "PAIR", "HOLDER"]}
    },
    "primitive_types": {
        "Any": {"name": "Any"},
        "Integer": {"name": "Integer", "ancestors": ["Any"]},
        "String": {"name": "String", "ancestors": ["Any"]}
    },
    "class_definitions": {
        "PAIR": {
            "name": "PAIR",
            "ancestors": ["Any"],
            "generic_parameter_defs": {
                "V": {"name": "V"},
                "K": {"name": "K"}
            }
        },
        "HOLDER": {
            "name": "HOLDER",
            "ancestors": ["Any"],
            "properties": {
                "pair": {
                    "_type": "P_BMM_GENERIC_PROPERTY",
                    "name": "pair",
                    "type_def": {
                        "root_type": "PAIR",
                        "generic_parameter_defs": {
                            "V": {"_type": "P_BMM_SIMPLE_TYPE", "type": "Integer"},
                            "K": {"_type": "P_BMM_SIMPLE_TYPE", "type": "String"}
                        }
                    }
                }
            }
        }
    }
}"#;

/// The class named `name` in `schema`'s class definitions.
fn class<'a>(schema: &'a PBmmSchema, name: &str) -> &'a PBmmClass {
    schema
        .class_definitions
        .as_ref()
        .and_then(|classes| classes.get(name))
        .unwrap_or_else(|| panic!("the fixture defines {name}"))
}

/// `PAIR`'s formal parameter names and `HOLDER.pair`'s actual parameter keys,
/// each in stored order.
fn orders(schema: &PBmmSchema) -> (Vec<String>, Vec<String>) {
    let formals = class(schema, "PAIR")
        .generic_parameter_defs()
        .expect("PAIR is generic")
        .keys()
        .cloned()
        .collect();
    let property = class(schema, "HOLDER")
        .properties()
        .and_then(|properties| properties.get("pair"))
        .expect("HOLDER declares pair");
    let PBmmProperty::PBmmGenericProperty(generic) = property else {
        panic!("pair is a P_BMM_GENERIC_PROPERTY");
    };
    let actuals = generic
        .type_def
        .as_ref()
        .and_then(|type_def| type_def.generic_parameter_defs.as_ref())
        .expect("pair states its generic_parameter_defs")
        .keys()
        .cloned()
        .collect();
    (formals, actuals)
}

/// The declaration order, `V` before `K`.
fn declared() -> (Vec<String>, Vec<String>) {
    let order = vec!["V".to_owned(), "K".to_owned()];
    (order.clone(), order)
}

#[test]
fn both_serialisations_read_the_declaration_order() -> Result<(), PBmmReadError> {
    let odin = read_schema(ODIN_SCHEMA)?;
    let json = read_schema_json(JSON_SCHEMA)?;
    assert_eq!(orders(&odin), declared(), "ODIN");
    assert_eq!(orders(&json), declared(), "JSON");
    assert_eq!(odin, json);
    Ok(())
}

#[test]
fn the_canonical_json_codec_keeps_the_declaration_order() -> Result<(), PBmmReadError> {
    let read = read_schema(ODIN_SCHEMA)?;
    let written = serde_json::to_string(&read).expect("a P_BMM_SCHEMA serialises");
    let reread: PBmmSchema = serde_json::from_str(&written).expect("the written form reads back");
    assert_eq!(orders(&reread), declared());
    assert_eq!(reread, read);
    Ok(())
}

#[test]
fn actual_parameters_bind_to_formals_by_name() -> Result<(), PBmmReadError> {
    // The keys written against the declaration order still bind by name, so
    // the materialised type lists its parameters as `PAIR` declares them.
    let swapped = ODIN_SCHEMA.replacen(
        r#"["V"] = (P_BMM_SIMPLE_TYPE) <
                            type = <"Integer">
                        >
                        ["K"] = (P_BMM_SIMPLE_TYPE) <
                            type = <"String">
                        >"#,
        r#"["K"] = (P_BMM_SIMPLE_TYPE) <
                            type = <"String">
                        >
                        ["V"] = (P_BMM_SIMPLE_TYPE) <
                            type = <"Integer">
                        >"#,
        1,
    );
    assert_ne!(swapped, ODIN_SCHEMA, "the fixture edit applies");
    for source in [ODIN_SCHEMA, swapped.as_str()] {
        let model = create_bmm_model(&read_schema(source)?)?;
        let parameters: Vec<String> = model
            .class_definition("PAIR")
            .and_then(|pair| pair.generic_parameters())
            .expect("PAIR materialises as a generic class")
            .keys()
            .cloned()
            .collect();
        assert_eq!(parameters, ["V", "K"]);
        assert_eq!(
            model
                .property_definition("HOLDER", "pair")
                .map(BmmProperty::type_name),
            Some("PAIR<Integer,String>".to_owned())
        );
    }
    Ok(())
}

#[test]
fn a_key_naming_no_formal_parameter_is_refused() -> Result<(), PBmmReadError> {
    let text = ODIN_SCHEMA.replacen(
        r#"["K"] = (P_BMM_SIMPLE_TYPE)"#,
        r#"["X"] = (P_BMM_SIMPLE_TYPE)"#,
        1,
    );
    assert_ne!(text, ODIN_SCHEMA, "the fixture edit applies");
    let error = create_bmm_model(&read_schema(&text)?).expect_err("X is no formal of PAIR");
    assert!(
        matches!(
            &error,
            PBmmReadError::UnknownGenericParameterKey { root_type, key, .. }
                if root_type == "PAIR" && key == "X"
        ),
        "{error}"
    );
    Ok(())
}

/// The schema header the `ancestor_defs` fixtures share.
const HEADER: &str = r#"
    bmm_version = <"2.4">
    rm_publisher = <"openehr">
    schema_name = <"adltest">
    rm_release = <"1.0.2">
"#;

/// Reads [`HEADER`] plus `body`.
fn read_with_header(body: &str) -> Result<PBmmSchema, PBmmReadError> {
    read_schema(&format!("{HEADER}{body}"))
}

#[test]
fn a_simple_ancestor_def_reads_under_its_signature() -> Result<(), PBmmReadError> {
    // `P_BMM_CLASS.ancestor_defs` is Hash<String, P_BMM_BASE_TYPE>, and a
    // non-generic ancestor "is then also listed here, as a
    // `P_BMM_SIMPLE_TYPE`" (class doc §Attributes, SPECLANG-16).
    let schema = read_with_header(
        r#"
        class_definitions = <
            ["ParentType1"] = <
                name = <"ParentType1">
                ancestor_defs = <
                    ["UNKNOWN"] = (P_BMM_SIMPLE_TYPE) <
                        type = <"UNKNOWN">
                    >
                >
            >
        >
    "#,
    )?;
    let defs = class(&schema, "ParentType1")
        .ancestor_defs()
        .expect("the class declares ancestor_defs");
    assert_eq!(
        defs.get("UNKNOWN").map(PBmmBaseType::base_name),
        Some("UNKNOWN")
    );
    assert!(matches!(
        defs.get("UNKNOWN"),
        Some(PBmmBaseType::PBmmSimpleType(_))
    ));
    Ok(())
}

#[test]
fn an_ancestor_def_that_is_not_a_base_type_is_refused() {
    // A container type is no `P_BMM_BASE_TYPE` (`…p_bmm_base_type.adoc`
    // §Description: "a proper (non-container) BMM type").
    let error = read_with_header(
        r#"
        class_definitions = <
            ["ParentType1"] = <
                name = <"ParentType1">
                ancestor_defs = <
                    ["List<Any>"] = (P_BMM_CONTAINER_TYPE) <
                        container_type = <"List">
                        type = <"Any">
                    >
                >
            >
        >
    "#,
    )
    .expect_err("ancestor_defs admits only P_BMM_BASE_TYPE");
    assert_eq!(
        error,
        PBmmReadError::UnexpectedTypeMarker {
            path: "class_definitions/ParentType1/ancestor_defs/List<Any>".to_owned(),
            marker: "P_BMM_CONTAINER_TYPE".to_owned(),
            expected: "P_BMM_BASE_TYPE",
        }
    );
}

/// A class with a generic ancestor lists all its ancestors in `ancestor_defs`.
const ANCESTRY_SCHEMA: &str = r#"
    packages = <
        ["org.test"] = <
            name = <"org.test">
            classes = <"Any", "Integer", "PARENT", "CHILD">
        >
    >
    primitive_types = <
        ["Any"] = < name = <"Any"> >
        ["Integer"] = < name = <"Integer"> ancestors = <"Any"> >
    >
    class_definitions = <
        ["PARENT"] = <
            name = <"PARENT">
            ancestors = <"Any">
            generic_parameter_defs = <
                ["T"] = < name = <"T"> >
            >
        >
        ["CHILD"] = <
            name = <"CHILD">
            ancestor_defs = <
                ["PARENT<Integer>"] = (P_BMM_GENERIC_TYPE) <
                    root_type = <"PARENT">
                    generic_parameters = <"Integer">
                >
                ["Any"] = (P_BMM_SIMPLE_TYPE) <
                    type = <"Any">
                >
            >
        >
    >
"#;

#[test]
fn every_ancestor_def_form_builds_an_ancestor() -> Result<(), PBmmReadError> {
    // "any non-generic ancestor of the same class is then also listed here, as
    // a `P_BMM_SIMPLE_TYPE`" (`…p_bmm_class.adoc` §Attributes, SPECLANG-16).
    let schema = read_with_header(ANCESTRY_SCHEMA)?;
    let v2 = create_bmm_model(&schema)?;
    let v2_ancestors: Vec<&String> = v2
        .class_definition("CHILD")
        .and_then(|child| child.ancestors())
        .expect("CHILD has ancestors")
        .keys()
        .collect();
    assert_eq!(v2_ancestors, ["Any", "PARENT"]);
    let v3 = create_bmm3_model(&schema)?;
    let v3_ancestors: Vec<&String> = v3
        .class_definition("CHILD")
        .and_then(|child| child.ancestors())
        .expect("CHILD has ancestors")
        .keys()
        .collect();
    assert_eq!(v3_ancestors, ["Any", "PARENT"]);
    Ok(())
}
