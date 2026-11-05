// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The catalogue of RFC 7643 §4 against the schema representation RFC 7643
//! §8.7.1 prints: every attribute, sub-attribute and characteristic, with the
//! two corrections the catalogue makes named and tested on their own.

use ferroehr_rest::scim::protocol::schema::{AttributeDefinition, Schema, catalogue};
use serde_json::Value;

use super::source::{self, Doc};

/// The RFC 7643 §8.7.1 representation: User, Group and `EnterpriseUser`.
fn printed() -> Vec<Value> {
    match source::json(Doc::Rfc7643, 2598..=4077, &[]) {
        Value::Array(schemas) => schemas,
        other => panic!("§8.7.1 prints an array: {other}"),
    }
}

/// The characteristics of an attribute that are facts of the schema, without
/// the description, which the catalogue words itself.
fn characteristics(attribute: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for key in [
        "name",
        "type",
        "multiValued",
        "required",
        "caseExact",
        "mutability",
        "returned",
        "uniqueness",
        "canonicalValues",
        "referenceTypes",
    ] {
        if let Some(value) = attribute.get(key) {
            out.insert(key.to_owned(), value.clone());
        }
    }
    if let Some(Value::Array(subs)) = attribute.get("subAttributes") {
        out.insert(
            "subAttributes".to_owned(),
            Value::Array(subs.iter().map(characteristics).collect()),
        );
    }
    Value::Object(out)
}

fn catalogued(schema: &Schema) -> Value {
    let value = serde_json::to_value(schema).expect("a schema serializes");
    Value::Array(
        value["attributes"]
            .as_array()
            .expect("attributes")
            .iter()
            .map(characteristics)
            .collect(),
    )
}

/// Drops the sub-attribute `name` of the attribute `attribute` from a
/// characteristics list, the one way the catalogue departs from §8.7.1.
fn without_sub(mut attributes: Value, attribute: &str, name: &str) -> Value {
    for entry in attributes.as_array_mut().expect("attributes") {
        if entry["name"] == attribute {
            let subs = entry["subAttributes"]
                .as_array_mut()
                .expect("sub-attributes");
            let before = subs.len();
            subs.retain(|sub| sub["name"] != name);
            assert_eq!(
                subs.len(),
                before - 1,
                "{attribute}.{name} is the catalogue's correction"
            );
        }
    }
    attributes
}

#[test]
fn the_user_schema_carries_every_characteristic_of_section_8_7_1() {
    let printed = printed();
    let user = &printed[0];
    assert_eq!(user["id"], "urn:ietf:params:scim:schemas:core:2.0:User");
    let schema = catalogue::core_user();
    assert_eq!(schema.id, "urn:ietf:params:scim:schemas:core:2.0:User");
    assert_eq!(schema.name.as_deref(), Some("User"));
    let expected: Vec<Value> = user["attributes"]
        .as_array()
        .expect("attributes")
        .iter()
        .map(characteristics)
        .collect();
    assert_eq!(
        without_sub(catalogued(&schema), "addresses", "primary"),
        Value::Array(expected)
    );
}

#[test]
fn the_group_schema_carries_every_characteristic_of_section_8_7_1() {
    let printed = printed();
    let group = &printed[1];
    assert_eq!(group["id"], "urn:ietf:params:scim:schemas:core:2.0:Group");
    let schema = catalogue::core_group();
    let mut expected: Vec<Value> = group["attributes"]
        .as_array()
        .expect("attributes")
        .iter()
        .map(characteristics)
        .collect();
    // RFC 7643 §4.2 makes `displayName` "REQUIRED"; the §8.7.1 example, which
    // "MAY change", prints `"required" : false`. The normative text wins.
    assert_eq!(expected[0]["name"], "displayName");
    assert_eq!(expected[0]["required"], false);
    expected[0]["required"] = Value::Bool(true);
    assert_eq!(
        without_sub(catalogued(&schema), "members", "display"),
        Value::Array(expected)
    );
}

#[test]
fn the_enterprise_schema_carries_every_characteristic_of_section_8_7_1() {
    let printed = printed();
    let enterprise = &printed[2];
    assert_eq!(
        enterprise["id"],
        "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User"
    );
    let schema = catalogue::enterprise_user();
    let expected: Vec<Value> = enterprise["attributes"]
        .as_array()
        .expect("attributes")
        .iter()
        .map(characteristics)
        .collect();
    assert_eq!(catalogued(&schema), Value::Array(expected));
}

#[test]
fn group_members_carry_the_display_the_examples_send() {
    // RFC 7643 §8.4 and the RFC 7644 §3.5.2.1 PATCH examples carry
    // `members[].display`; §8.7.1 omits it.
    let schema = catalogue::core_group();
    let members = schema
        .attributes
        .iter()
        .find(|a| a.name == "members")
        .expect("members");
    let display = sub(members, "display");
    assert_eq!(
        serde_json::to_value(display.mutability).expect("serializes"),
        "immutable",
        "RFC 7643 §4.2"
    );
}

#[test]
fn addresses_carry_the_primary_the_examples_send() {
    // RFC 7643 §8.2 and RFC 7644 §3.5.2.3 set `addresses[].primary`; §8.7.1
    // omits it, while §2.4 lists it among the multi-valued sub-attributes.
    let schema = catalogue::core_user();
    let addresses = schema
        .attributes
        .iter()
        .find(|a| a.name == "addresses")
        .expect("addresses");
    let primary = sub(addresses, "primary");
    assert_eq!(
        serde_json::to_value(primary.kind).expect("serializes"),
        "boolean"
    );
}

fn sub<'a>(attribute: &'a AttributeDefinition, name: &str) -> &'a AttributeDefinition {
    attribute
        .sub_attributes
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find(|s| s.name == name)
        .expect("the sub-attribute")
}

#[test]
fn every_catalogued_attribute_has_a_description() {
    // RFC 7643 §7: "When applicable, service providers MUST specify the description."
    for schema in [
        catalogue::core_user(),
        catalogue::core_group(),
        catalogue::enterprise_user(),
    ] {
        for attribute in &schema.attributes {
            assert!(!attribute.description.is_empty(), "{}", attribute.name);
            for sub in attribute.sub_attributes.as_deref().unwrap_or_default() {
                assert!(
                    !sub.description.is_empty(),
                    "{}.{}",
                    attribute.name,
                    sub.name
                );
            }
        }
    }
}

#[test]
fn the_service_provider_schemas_of_section_8_7_2_read_as_schemas() {
    let printed = source::json(Doc::Rfc7643, 4101..=5082, &[]);
    let schemas: Vec<Schema> =
        serde_json::from_value(printed.clone()).expect("the schema model reads §8.7.2");
    let ids: Vec<&str> = schemas.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig",
            "urn:ietf:params:scim:schemas:core:2.0:ResourceType",
            "urn:ietf:params:scim:schemas:core:2.0:Schema",
        ]
    );
    assert_eq!(
        serde_json::to_value(&schemas).expect("serializes"),
        printed,
        "the model loses nothing"
    );
}

#[test]
fn the_resource_schemas_of_section_8_7_1_read_as_schemas() {
    let printed = source::json(Doc::Rfc7643, 2598..=4077, &[]);
    let schemas: Vec<Schema> =
        serde_json::from_value(printed.clone()).expect("the schema model reads §8.7.1");
    assert_eq!(
        serde_json::to_value(&schemas).expect("serializes"),
        printed,
        "the model loses nothing"
    );
}
