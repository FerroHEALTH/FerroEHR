// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `unique` cardinality modifier, evaluated as value equality (SPECAM-111;
//! ADL 1.4 `master05-cadl.adoc`, ADL2 `master04.3` §Container attributes), over
//! `Demo Vitals term mapping.opt` with every `items` read as `unique`.

#![expect(
    clippy::expect_used,
    reason = "fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests \
              scoping does not reach"
)]

use openehr_its::opt14;
use openehr_its::opt14::types::{CAttribute, CObject};
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::WebTemplate;
use openehr_sdt::rm_instance::{ValidationKind, ValidationMessage, validate_composition};
use serde_json::{Value, json};

const ITEMS: &str = "/data[at0002]/events[at0003]/data[at0001]/items";

fn template(unique: bool) -> WebTemplate {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/better/Demo Vitals term mapping.opt"
    );
    let xml = std::fs::read_to_string(path).expect("read the vendored OPT");
    let mut opt = opt14::from_xml(&xml).expect("the OPT parses");
    if unique {
        mark_unique(&mut opt.definition.attributes);
    }
    build_web_template(&opt).expect("build the web template")
}

fn mark_unique(attributes: &mut [CAttribute]) {
    for attribute in attributes {
        let children = match attribute {
            CAttribute::CMultipleAttribute(m) => {
                if m.rm_attribute_name == "items" {
                    m.cardinality.is_unique = true;
                }
                &mut m.children
            }
            CAttribute::CSingleAttribute(s) => &mut s.children,
        };
        for child in children {
            match child {
                CObject::CComplexObject(c) => mark_unique(&mut c.attributes),
                CObject::CArchetypeRoot(r) => mark_unique(&mut r.attributes),
                _ => {}
            }
        }
    }
}

fn composition() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../openehr-its/tests/vendor/openehr_sdk/composition/canonical_json/demo_vitals_352.json"
    );
    let text = std::fs::read_to_string(path).expect("read the vendored composition");
    serde_json::from_str(&text).expect("the composition is JSON")
}

/// The composition with a second temperature reading equal by value to the
/// first: the same node and value, another name and its own uid.
fn with_a_repeated_reading() -> Value {
    let mut c = composition();
    let items = c
        .pointer_mut("/content/0/items/0/data/events/0/data/items")
        .and_then(Value::as_array_mut)
        .expect("the temperature item list");
    let mut repeat = items.first().cloned().expect("a first reading");
    let member = repeat.as_object_mut().expect("a member object");
    member.insert(
        "name".to_owned(),
        json!({"_type": "DV_TEXT", "value": "Temperature (repeat)"}),
    );
    member.insert(
        "uid".to_owned(),
        json!({"_type": "HIER_OBJECT_ID", "value": "4f6b2c1e-3d5a-4e7b-9c8d-0a1b2c3d4e5f"}),
    );
    items.push(repeat);
    c
}

fn unique_refusals(messages: &[ValidationMessage]) -> Vec<&ValidationMessage> {
    messages
        .iter()
        .filter(|m| m.kind == ValidationKind::Cardinality && m.message.contains("unique"))
        .collect()
}

#[test]
fn a_unique_container_refuses_two_value_equal_members() {
    let messages = validate_composition(&with_a_repeated_reading(), &template(true));
    let refusals = unique_refusals(&messages);
    assert_eq!(refusals.len(), 1, "one unique refusal, got {messages:?}");
    assert!(
        refusals.iter().all(|m| m.path.ends_with(ITEMS)),
        "the refusal names the items path, got {refusals:?}"
    );
}

#[test]
fn a_unique_container_accepts_members_that_differ_by_value() {
    let messages = validate_composition(&composition(), &template(true));
    assert!(
        messages.is_empty(),
        "the vendored composition stays clean under unique, got {messages:?}"
    );
}

#[test]
fn a_container_without_unique_accepts_value_equal_members() {
    let messages = validate_composition(&with_a_repeated_reading(), &template(false));
    assert!(
        unique_refusals(&messages).is_empty(),
        "no unique refusal without the modifier, got {messages:?}"
    );
}
