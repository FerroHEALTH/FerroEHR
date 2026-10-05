// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The `BMM_ENUMERATION` validity rules, driven through the public `P_BMM`
//! pipeline (`read_schema` → `create_bmm_model`).
//!
//! Two rules, each with BOTH twins — the accepted valid shape and the refused
//! invalid one:
//!
//! * an enumeration "may have only one ancestor"
//!   (`docs/specs/openehr/LANG/docs/bmm3/master07-core-classes.adoc`
//!   §Range-Constrained Classes; `org.openehr.lang.bmm3.bmm_enumeration.adoc`
//!   §Description: "Only one inheritance ancestor is allowed in order to provide
//!   the base type to which the range constraint is applied");
//! * `item_values` is "Optional list of specific values. Must be 1:1 with
//!   `item_names` list" (`org.openehr.lang.bmm.bmm_enumeration.adoc`
//!   §Attributes), while omitting the values entirely stays legal ("If no values
//!   are supplied, the integer values 0, 1, 2, ... are assumed").

#![expect(
    clippy::panic_in_result_fn,
    reason = "the Book ch11 test shape: `?` propagates the read/model plumbing while the assertions ARE the test — an assertion panic is how these tests fail"
)]

use openehr_lang::v1_1::bmm_persistence::create_model::create_bmm_model;
use openehr_lang::v1_1::bmm_persistence::error::PBmmReadError;
use openehr_lang::v1_1::bmm_persistence::p_bmm_schema::PBmmSchema;
use openehr_lang::v1_1::bmm_persistence::reader::read_schema;

/// `master04-syntax.adoc` §Header Items — the four mandatory header items.
const HEADER: &str = r#"
    bmm_version = <"2.4">
    rm_publisher = <"openehr">
    schema_name = <"enum_validity">
    rm_release = <"1.0.2">
"#;

/// A schema whose single package lists the two primitive base classes plus the
/// enumeration under test, with `enumeration` supplying the enumeration's own
/// `class_definitions` block.
fn schema(enumeration: &str) -> Result<PBmmSchema, PBmmReadError> {
    read_schema(&format!(
        r#"{HEADER}
        packages = <
            ["test"] = <
                name = <"test">
                classes = <"Integer", "String", "TASK_LIFECYCLE">
            >
        >
        class_definitions = <
            ["Integer"] = <
                name = <"Integer">
            >
            ["String"] = <
                name = <"String">
            >
            {enumeration}
        >
        "#
    ))
}

/// The `master07-core-classes.adoc` §Range-Constrained Classes `TASK_LIFECYCLE`
/// example: one ancestor, names and values 1:1.
#[test]
fn a_single_ancestor_enumeration_with_paired_values_materialises()
-> Result<(), Box<dyn std::error::Error>> {
    let parsed = schema(
        r#"["TASK_LIFECYCLE"] = (P_BMM_ENUMERATION_INTEGER) <
                name = <"TASK_LIFECYCLE">
                ancestors = <"Integer">
                item_names = <"planned", "available", "cancelled">
                item_values = <0, 1, 2>
            >"#,
    )?;
    let model = create_bmm_model(&parsed)?;
    let classes = model
        .class_definitions
        .as_ref()
        .ok_or("the model defines no classes")?;
    let enumeration = classes
        .get("TASK_LIFECYCLE")
        .ok_or("TASK_LIFECYCLE is missing from the model")?;
    assert_eq!(
        enumeration.ancestors().map(std::collections::BTreeMap::len),
        Some(1),
    );
    Ok(())
}

/// "If no values are supplied, the integer values 0, 1, 2, ... are assumed" — an
/// enumeration stating names only is valid.
#[test]
fn an_enumeration_stating_names_only_materialises() -> Result<(), Box<dyn std::error::Error>> {
    let parsed = schema(
        r#"["TASK_LIFECYCLE"] = (P_BMM_ENUMERATION_STRING) <
                name = <"TASK_LIFECYCLE">
                ancestors = <"String">
                item_names = <"planned", "available", "cancelled">
            >"#,
    )?;
    create_bmm_model(&parsed)?;
    Ok(())
}

/// An enumeration with NO ancestor is still valid — the v2 class doc supplies
/// the base type ("It is designed so that the default type is Integer",
/// `org.openehr.lang.bmm.bmm_enumeration.adoc` §Description).
#[test]
fn an_enumeration_without_an_ancestor_materialises() -> Result<(), Box<dyn std::error::Error>> {
    let parsed = schema(
        r#"["TASK_LIFECYCLE"] = (P_BMM_ENUMERATION) <
                name = <"TASK_LIFECYCLE">
                item_names = <"planned", "available">
            >"#,
    )?;
    create_bmm_model(&parsed)?;
    Ok(())
}

/// The invalid twin of the one-ancestor rule: two ancestors leave the base type
/// the range constraint applies to ambiguous, so materialisation refuses.
#[test]
fn two_ancestors_are_refused() -> Result<(), PBmmReadError> {
    let parsed = schema(
        r#"["TASK_LIFECYCLE"] = (P_BMM_ENUMERATION) <
                name = <"TASK_LIFECYCLE">
                ancestors = <"Integer", "String">
                item_names = <"planned", "available">
            >"#,
    )?;
    assert_eq!(
        create_bmm_model(&parsed).err(),
        Some(PBmmReadError::EnumerationAncestorCount {
            class: "TASK_LIFECYCLE".to_owned(),
            ancestors: vec!["Integer".to_owned(), "String".to_owned()],
        }),
    );
    Ok(())
}

/// The invalid twin of the 1:1 rule: three names against two stated values.
#[test]
fn item_values_that_are_not_one_to_one_are_refused() -> Result<(), PBmmReadError> {
    let parsed = schema(
        r#"["TASK_LIFECYCLE"] = (P_BMM_ENUMERATION_INTEGER) <
                name = <"TASK_LIFECYCLE">
                ancestors = <"Integer">
                item_names = <"planned", "available", "cancelled">
                item_values = <0, 1>
            >"#,
    )?;
    assert_eq!(
        create_bmm_model(&parsed).err(),
        Some(PBmmReadError::EnumerationItemListsNotOneToOne {
            class: "TASK_LIFECYCLE".to_owned(),
            names: 3,
            values: 2,
        }),
    );
    Ok(())
}

/// More values than names is the same violation from the other side.
#[test]
fn more_values_than_names_are_refused() -> Result<(), PBmmReadError> {
    let parsed = schema(
        r#"["TASK_LIFECYCLE"] = (P_BMM_ENUMERATION_INTEGER) <
                name = <"TASK_LIFECYCLE">
                ancestors = <"Integer">
                item_names = <"planned">
                item_values = <0, 1, 2>
            >"#,
    )?;
    assert_eq!(
        create_bmm_model(&parsed).err(),
        Some(PBmmReadError::EnumerationItemListsNotOneToOne {
            class: "TASK_LIFECYCLE".to_owned(),
            names: 1,
            values: 3,
        }),
    );
    Ok(())
}
