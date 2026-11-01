// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![expect(
    clippy::panic_in_result_fn,
    reason = "the Book ch11 test shape: `?` propagates the read/model plumbing while a let-else over the materialised class shape IS the assertion"
)]

use crate::v1_1::bmm_persistence::create_bmm3_model::create_bmm3_model;
use crate::v1_1::bmm_persistence::create_bmm3_model::create_bmm3_model_reporting;
use crate::v1_1::bmm_persistence::error::PBmmReadError;
use crate::v1_1::bmm_persistence::reader::read_schema;
use crate::v1_1::bmm_persistence::validate::PBmmValidityFinding;
use crate::v1_1::bmm3::core::entity::bmm_class::BmmClass;
use crate::v1_1::bmm3::core::entity::bmm_simple_class::BmmSimpleClass;
use crate::v1_1::bmm3::core::entity::range_constrained::bmm_enumeration::BmmEnumeration;
use crate::v1_1::bmm3::core::feature::bmm_static::BmmStatic;
use crate::v1_1::bmm3::core::literal_value::bmm_literal_value::BmmLiteralValue;
use crate::v1_1::bmm3::core::literal_value::bmm_primitive_value::BmmPrimitiveValue;
use crate::v1_1::bmm3::core::model::bmm_model::BmmModel;

/// A schema whose single package lists the primitive base classes plus the
/// class under test, in the `master04-syntax.adoc` §Header Items shape.
fn schema_src(definitions: &str) -> String {
    format!(
        r#"
        bmm_version = <"2.4">
        rm_publisher = <"openehr">
        schema_name = <"bmm3_literal_values">
        rm_release = <"1.0.2">
        packages = <
            ["test"] = <
                name = <"test">
                classes = <"Integer", "String", "SUBJECT">
            >
        >
        class_definitions = <
            ["Integer"] = < name = <"Integer"> >
            ["String"] = < name = <"String"> >
            {definitions}
        >
        "#
    )
}

/// The v3 model of a schema built by [`schema_src`].
fn model_of(definitions: &str) -> Result<BmmModel, PBmmReadError> {
    create_bmm3_model(&read_schema(&schema_src(definitions))?)
}

/// The `BMM_ENUMERATION_INTEGER` item values of the class `SUBJECT` in a
/// model built from `definitions`.
fn integer_items(definitions: &str) -> Result<Vec<i32>, PBmmReadError> {
    let model = model_of(definitions)?;
    let classes = model
        .class_definitions
        .as_ref()
        .expect("the model defines classes");
    let subject = classes.get("SUBJECT").expect("SUBJECT materialised");
    let BmmClass::BmmSimpleClass(BmmSimpleClass::BmmEnumeration(
        BmmEnumeration::BmmEnumerationInteger(enumeration),
    )) = subject
    else {
        panic!("SUBJECT is not an integer enumeration");
    };
    Ok(enumeration
        .item_values
        .iter()
        .flatten()
        .map(|item| item.value)
        .collect())
}

/// `org.openehr.lang.bmm3.bmm_enumeration_integer.adoc` §Attributes: the
/// item values are `BMM_INTEGER_VALUE`s, so distinct persisted scalars stay
/// distinct native values.
#[test]
fn integer_enumeration_values_reach_the_model_unchanged() {
    let items = integer_items(
        r#"["SUBJECT"] = (P_BMM_ENUMERATION_INTEGER) <
                name = <"SUBJECT">
                ancestors = <"Integer">
                item_names = <"first", "second">
                item_values = <1001, 1002>
            >"#,
    )
    .expect("the enumeration materialises");
    assert_eq!(items, vec![1001, 1002]);
}

/// An item value outside `Integer`'s 32-bit range
/// (`BASE/docs/foundation_types/master03-primitive_types.adoc` §Overview)
/// has no `BMM_INTEGER_VALUE.value`, so it is refused with the item named
/// rather than collapsed onto another item's value.
#[test]
fn an_out_of_range_integer_enumeration_value_is_refused() {
    let refusal = integer_items(
        r#"["SUBJECT"] = (P_BMM_ENUMERATION_INTEGER) <
                name = <"SUBJECT">
                ancestors = <"Integer">
                item_names = <"first", "second">
                item_values = <3000000000, 4000000000>
            >"#,
    )
    .expect_err("an out-of-range item value is refused");
    assert_eq!(
        refusal,
        PBmmReadError::EnumerationItemValueNotAnInteger {
            class: "SUBJECT".to_owned(),
            index: 0,
            item: Some("first".to_owned()),
            value: "3000000000".to_owned(),
        },
    );
}

/// A persisted item value of another JSON kind names no `Integer` either.
#[test]
fn a_non_numeric_integer_enumeration_value_is_refused() {
    let refusal = integer_items(
        r#"["SUBJECT"] = (P_BMM_ENUMERATION_INTEGER) <
                name = <"SUBJECT">
                ancestors = <"Integer">
                item_names = <"first", "second">
                item_values = <"one", "two">
            >"#,
    )
    .expect_err("a non-integer item value is refused");
    assert_eq!(
        refusal,
        PBmmReadError::EnumerationItemValueNotAnInteger {
            class: "SUBJECT".to_owned(),
            index: 0,
            item: Some("first".to_owned()),
            value: "one".to_owned(),
        },
    );
}

/// `org.openehr.lang.bmm3.bmm_constant.adoc` §Attributes: `generator` is
/// `1..1`, and its `value_literal` carries the persisted serial form.
#[test]
fn a_persisted_constant_value_becomes_the_generator_literal() {
    let model = model_of(
        r#"["SUBJECT"] = <
                name = <"SUBJECT">
                constants = <
                    ["Max_retries"] = < name = <"Max_retries"> type = <"Integer"> value = <"3"> >
                >
            >"#,
    )
    .expect("the class materialises");
    let classes = model
        .class_definitions
        .as_ref()
        .expect("the model defines classes");
    let subject = classes.get("SUBJECT").expect("SUBJECT materialised");
    let statics = subject.static_properties().expect("SUBJECT has constants");
    let BmmStatic::BmmConstant(constant) =
        statics.get("Max_retries").expect("the constant is keyed")
    else {
        panic!("Max_retries is not a constant");
    };
    let BmmLiteralValue::BmmPrimitiveValue(BmmPrimitiveValue::BmmPrimitiveValue(literal)) =
        &constant.generator
    else {
        panic!("the generator is not a primitive value");
    };
    assert_eq!(literal.value_literal, "3");
}

/// `P_BMM_CONSTANT.value` is `0..1` while `BMM_LITERAL_VALUE.value_literal`
/// is `1..1`, so a constant stating no value is omitted with a finding —
/// never materialised with an empty serial form, and never a refusal of a
/// persisted form the P_BMM spec admits.
#[test]
fn a_constant_without_a_persisted_value_is_omitted_with_a_finding() {
    let (model, findings) = create_bmm3_model_reporting(
        &read_schema(&schema_src(
            r#"["SUBJECT"] = <
                name = <"SUBJECT">
                constants = <
                    ["Max_retries"] = < name = <"Max_retries"> type = <"Integer"> >
                    ["Min_retries"] = < name = <"Min_retries"> type = <"Integer"> value = <"1"> >
                >
            >"#,
        ))
        .expect("the fixture reads"),
    )
    .expect("the class materialises");
    assert_eq!(
        findings,
        vec![PBmmValidityFinding::ConstantNotMaterialised {
            class: "SUBJECT".to_owned(),
            constant: "Max_retries".to_owned(),
        }],
    );
    let classes = model
        .class_definitions
        .as_ref()
        .expect("the model defines classes");
    let subject = classes.get("SUBJECT").expect("SUBJECT materialised");
    let statics = subject.static_properties().expect("SUBJECT has constants");
    assert!(!statics.contains_key("Max_retries"));
    assert!(statics.contains_key("Min_retries"));
    // The omission reaches the feature list too, which is derived from the
    // same map (`master07-core-classes.adoc` §Overview).
    assert!(!subject.features().iter().any(|f| f.name() == "Max_retries"));
}
