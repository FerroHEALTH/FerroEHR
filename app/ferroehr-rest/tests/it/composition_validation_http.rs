// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the COMPOSITION semantic-validation response —
//! the `422` an RM class-invariant violation produces, driven through the
//! assembled router over a **real** `FerroEhrService` on a real `PostgreSQL`.
//!
//! Wire oracle: ITS-REST 1.1.0
//! `specifications/responses/422.yaml` ("content could be
//! converted to a COMPOSITION, but there are semantic validation errors"),
//! answered as RFC 9457 problem details whose `errors` member, the additional
//! detail a `Prefer: return=representation` request receives
//! (`Requests_and_responses.md` §HTTP status codes), lists the violations. The RM rule under test is `data_structures` §ELEMENT
//! `Inv_null_flavour_indicated`.
#![expect(
    clippy::unwrap_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};

use crate::common;
use crate::common::{etag_uid, ips_canonical_composition, ips_opt_xml, send};

const BASE: &str = "/ferroehr/rest/openehr/v1";

/// A fresh EHR with the IPS operational template uploaded.
async fn app_with_template() -> (testkit::TestDb, Router, String) {
    let (pg, app) = common::test_router().await;
    let (status, h, body) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/ehr"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "ehr create: {body}");
    let ehr_id = etag_uid(&h);

    let (status, _h, body) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/definition/template/adl1.4"))
            .header(header::CONTENT_TYPE, "application/xml")
            .body(Body::from(ips_opt_xml()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");
    (pg, app, ehr_id)
}

/// Strip the `value` of the first ELEMENT that has one, leaving no
/// `null_flavour` behind — the shape RM `data_structures` §ELEMENT
/// `Inv_null_flavour_indicated` forbids.
fn strip_first_element_value(v: &mut Value) -> bool {
    match v {
        Value::Object(map) => {
            if map.get("_type").and_then(Value::as_str) == Some("ELEMENT")
                && map.get("null_flavour").is_none()
                && map.remove("value").is_some()
            {
                return true;
            }
            let keys: Vec<String> = map.keys().cloned().collect();
            keys.into_iter()
                .any(|k| map.get_mut(&k).is_some_and(strip_first_element_value))
        }
        Value::Array(items) => items.iter_mut().any(strip_first_element_value),
        _ => false,
    }
}

/// A COMPOSITION holding an ELEMENT with neither `value` nor `null_flavour`
/// violates RM `data_structures` §ELEMENT:
///
/// > `Inv_null_flavour_indicated`: `is_null() xor null_flavour = Void`
/// > (`RM/docs/UML/classes/org.openehr.rm.data_structures.element.adoc`)
///
/// The commit is a `422` (ITS-REST `responses/422.yaml`: converts,
/// but does not validate) whose problem details carry the violation in
/// `errors`, keyed by the offending RM path and naming the
/// invariant — so the client learns which node is wrong and why, instead of
/// the element being silently dropped.
#[tokio::test]
async fn composition_with_a_value_less_element_is_422_naming_the_invariant() {
    let (_pg, app, ehr_id) = app_with_template().await;

    let mut broken = ips_canonical_composition();
    assert!(
        strip_first_element_value(&mut broken),
        "the IPS composition carries a valued ELEMENT to empty"
    );

    let (status, _h, body) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/ehr/{ehr_id}/composition"))
            .header(header::CONTENT_TYPE, "application/json")
            .header("prefer", "return=representation")
            .body(Body::from(broken.to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an RM-invalid ELEMENT must not commit: {body}"
    );

    let error: Value = serde_json::from_str(&body).expect("the 422 carries problem details");
    assert!(
        error["type"]
            .as_str()
            .is_some_and(|t| t.ends_with("#validation-failed")),
        "{body}"
    );
    let entries = error["errors"]
        .as_array()
        .expect("the problem lists its violations in errors");
    let named = entries
        .iter()
        .find(|e| {
            e["detail"]
                .as_str()
                .is_some_and(|d| d.contains("Inv_null_flavour_indicated"))
        })
        .unwrap_or_else(|| panic!("no entry names the invariant: {entries:?}"));
    assert!(
        named["path"].is_string(),
        "each errors entry is `{{path, detail}}`, got {named:?}"
    );
}

/// An operational template whose `ITEM_TREE.items` holds one object
/// constraint of RM type `slot` at `at0004` (AM ADL 1.4 `C_OBJECT`
/// `rm_type_name`), the shape of the CNF item type narrowing case.
fn item_narrowing_opt(template_id: &str, uid: &str, slot: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<template xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema" xmlns="http://schemas.openehr.org/v1">
  <language><terminology_id><value>ISO_639-1</value></terminology_id><code_string>en</code_string></language>
  <description><original_author id="Original Author">CNF corpus</original_author><lifecycle_state>Initial</lifecycle_state><details><language><terminology_id><value>ISO_639-1</value></terminology_id><code_string>en</code_string></language><purpose>CNF content constraint template</purpose></details></description>
  <uid><value>{uid}</value></uid>
  <template_id><value>{template_id}</value></template_id>
  <concept>CNF content constraint</concept>
  <definition>
    <rm_type_name>COMPOSITION</rm_type_name>
    <occurrences><lower_included>true</lower_included><upper_included>true</upper_included><lower_unbounded>false</lower_unbounded><upper_unbounded>false</upper_unbounded><lower>1</lower><upper>1</upper></occurrences>
    <node_id>at0000</node_id>
    <attributes xsi:type="C_SINGLE_ATTRIBUTE"><rm_attribute_name>category</rm_attribute_name><existence><lower_included>true</lower_included><upper_included>true</upper_included><lower_unbounded>false</lower_unbounded><upper_unbounded>false</upper_unbounded><lower>1</lower><upper>1</upper></existence><children xsi:type="C_COMPLEX_OBJECT"><rm_type_name>DV_CODED_TEXT</rm_type_name><occurrences><lower_included>true</lower_included><upper_included>true</upper_included><lower_unbounded>false</lower_unbounded><upper_unbounded>false</upper_unbounded><lower>1</lower><upper>1</upper></occurrences><node_id /><attributes xsi:type="C_SINGLE_ATTRIBUTE"><rm_attribute_name>defining_code</rm_attribute_name><existence><lower_included>true</lower_included><upper_included>true</upper_included><lower_unbounded>false</lower_unbounded><upper_unbounded>false</upper_unbounded><lower>1</lower><upper>1</upper></existence><children xsi:type="C_CODE_PHRASE"><rm_type_name>CODE_PHRASE</rm_type_name><occurrences><lower_included>true</lower_included><upper_included>true</upper_included><lower_unbounded>false</lower_unbounded><upper_unbounded>false</upper_unbounded><lower>1</lower><upper>1</upper></occurrences><node_id /><terminology_id><value>openehr</value></terminology_id><code_list>433</code_list></children></attributes></children></attributes>
    
    <attributes xsi:type="C_MULTIPLE_ATTRIBUTE"><rm_attribute_name>content</rm_attribute_name><existence><lower_included>true</lower_included><upper_included>true</upper_included><lower_unbounded>false</lower_unbounded><upper_unbounded>false</upper_unbounded><lower>0</lower><upper>1</upper></existence><children xsi:type="C_ARCHETYPE_ROOT"><rm_type_name>EVALUATION</rm_type_name><occurrences><lower_included>true</lower_included><lower_unbounded>false</lower_unbounded><upper_unbounded>true</upper_unbounded><lower>0</lower></occurrences><node_id>at0000</node_id><attributes xsi:type="C_SINGLE_ATTRIBUTE"><rm_attribute_name>data</rm_attribute_name><existence><lower_included>true</lower_included><upper_included>true</upper_included><lower_unbounded>false</lower_unbounded><upper_unbounded>false</upper_unbounded><lower>1</lower><upper>1</upper></existence><children xsi:type="C_COMPLEX_OBJECT"><rm_type_name>ITEM_TREE</rm_type_name><occurrences><lower_included>true</lower_included><upper_included>true</upper_included><lower_unbounded>false</lower_unbounded><upper_unbounded>false</upper_unbounded><lower>1</lower><upper>1</upper></occurrences><node_id>at0003</node_id><attributes xsi:type="C_MULTIPLE_ATTRIBUTE"><rm_attribute_name>items</rm_attribute_name><existence><lower_included>true</lower_included><upper_included>true</upper_included><lower_unbounded>false</lower_unbounded><upper_unbounded>false</upper_unbounded><lower>0</lower><upper>1</upper></existence><children xsi:type="C_COMPLEX_OBJECT"><rm_type_name>{slot}</rm_type_name><occurrences><lower_included>true</lower_included><lower_unbounded>false</lower_unbounded><upper_unbounded>true</upper_unbounded><lower>0</lower></occurrences><node_id>at0004</node_id></children><cardinality><is_ordered>false</is_ordered><is_unique>false</is_unique><interval><lower_included>true</lower_included><lower_unbounded>false</lower_unbounded><upper_unbounded>true</upper_unbounded><lower>0</lower></interval></cardinality></attributes></children></attributes><archetype_id><value>openEHR-EHR-EVALUATION.minimal.v1</value></archetype_id><term_definitions code="at0000"><items id="description">unknown</items><items id="text">Minimal</items></term_definitions><term_definitions code="at0003"><items id="description">@ internal @</items><items id="text">structure</items></term_definitions><term_definitions code="at0004"><items id="description">@ internal @</items><items id="text">item</items></term_definitions></children><cardinality><is_ordered>false</is_ordered><is_unique>false</is_unique><interval><lower_included>true</lower_included><lower_unbounded>false</lower_unbounded><upper_unbounded>true</upper_unbounded><lower>0</lower></interval></cardinality></attributes>
    <archetype_id><value>openEHR-EHR-COMPOSITION.minimal.v1</value></archetype_id>
    <template_id><value>{template_id}</value></template_id>
    <term_definitions code="at0000"><items id="description">unknown</items><items id="text">Minimal</items></term_definitions>
  </definition>
</template>
"#
    )
}

/// A COMPOSITION of `template_id` whose `ITEM_TREE` holds `item`.
fn item_narrowing_composition(template_id: &str, item: &Value) -> Value {
    serde_json::json!({
        "_type": "COMPOSITION",
        "name": { "_type": "DV_TEXT", "value": "Minimal" },
        "archetype_node_id": "openEHR-EHR-COMPOSITION.minimal.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-COMPOSITION.minimal.v1" },
            "template_id": { "_type": "TEMPLATE_ID", "value": template_id },
            "rm_version": "1.1.0"
        },
        "language": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" }, "code_string": "en" },
        "territory": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_3166-1" }, "code_string": "NL" },
        "category": { "_type": "DV_CODED_TEXT", "value": "persistent",
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "433" } },
        "composer": { "_type": "PARTY_IDENTIFIED", "name": "Composer" },
        "content": [{
            "_type": "EVALUATION",
            "name": { "_type": "DV_TEXT", "value": "Minimal" },
            "archetype_node_id": "openEHR-EHR-EVALUATION.minimal.v1",
            "archetype_details": {
                "_type": "ARCHETYPED",
                "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-EVALUATION.minimal.v1" },
                "rm_version": "1.1.0"
            },
            "language": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" }, "code_string": "en" },
            "encoding": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "IANA_character-sets" }, "code_string": "UTF-8" },
            "subject": { "_type": "PARTY_SELF" },
            "data": {
                "_type": "ITEM_TREE",
                "name": { "_type": "DV_TEXT", "value": "structure" },
                "archetype_node_id": "at0003",
                "items": [item]
            }
        }]
    })
}

/// An item whose RM type conforms to none of the object constraints of its
/// attribute is a template validation failure, `422`, in both directions:
/// AM ADL 1.4 `aom14.c_object.adoc` `rm_type_name`, "Reference model type
/// that this node corresponds to", and `responses/422.yaml`, "the underlying
/// template ... is not validating the supplied resource".
#[tokio::test]
async fn an_item_of_a_type_the_template_does_not_allow_is_422() {
    let element = serde_json::json!({
        "_type": "ELEMENT",
        "name": { "_type": "DV_TEXT", "value": "item" },
        "archetype_node_id": "at0004",
        "value": { "_type": "DV_TEXT", "value": "x" }
    });
    let cluster = serde_json::json!({
        "_type": "CLUSTER",
        "name": { "_type": "DV_TEXT", "value": "item" },
        "archetype_node_id": "at0004",
        "items": [{
            "_type": "ELEMENT",
            "name": { "_type": "DV_TEXT", "value": "inner" },
            "archetype_node_id": "at0005",
            "value": { "_type": "DV_TEXT", "value": "x" }
        }]
    });
    let (_pg, app) = common::test_router().await;
    let (status, h, body) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/ehr"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "ehr create: {body}");
    let ehr_id = etag_uid(&h);
    let cases = [
        (
            "cnf.tpl.item_narrowing_cluster",
            "5c2f1a6e-3b8d-4f0a-9e71-0d5b7c2a4e10",
            "CLUSTER",
            &element,
            &cluster,
        ),
        (
            "cnf.tpl.item_narrowing_element",
            "8e4d2b7f-6c1a-4d3e-b9f0-2a7c5e1d3f20",
            "ELEMENT",
            &cluster,
            &element,
        ),
    ];
    for (template_id, uid, slot, refused, admitted) in cases {
        let (status, _h, body) = send(
            &app,
            Request::builder()
                .method("POST")
                .uri(format!("{BASE}/definition/template/adl1.4"))
                .header(header::CONTENT_TYPE, "application/xml")
                .body(Body::from(item_narrowing_opt(template_id, uid, slot)))
                .unwrap(),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "OPT upload {template_id}: {body}"
        );
        for (item, expected) in [
            (admitted, StatusCode::CREATED),
            (refused, StatusCode::UNPROCESSABLE_ENTITY),
        ] {
            let composition = item_narrowing_composition(template_id, item);
            let (status, _h, body) = send(
                &app,
                Request::builder()
                    .method("POST")
                    .uri(format!("{BASE}/ehr/{ehr_id}/composition"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(composition.to_string()))
                    .unwrap(),
            )
            .await;
            assert_eq!(
                status, expected,
                "{slot} slot, {} committed: {body}",
                item["_type"]
            );
        }
    }
}

/// Empty the `events` of the first HISTORY in `v`.
fn empty_first_history_events(v: &mut Value) -> bool {
    match v {
        Value::Object(map) => {
            if map.get("_type").and_then(Value::as_str) == Some("HISTORY")
                && map.contains_key("events")
            {
                map.insert("events".to_owned(), Value::Array(Vec::new()));
                return true;
            }
            map.values_mut().any(empty_first_history_events)
        }
        Value::Array(items) => items.iter_mut().any(empty_first_history_events),
        _ => false,
    }
}

/// A HISTORY carrying `events: []` is a body the ITS-JSON schema refuses
/// (`ITS-JSON/components/RM/Release-1.1.0/Data_structures/HISTORY.json`:
/// `events` has `minItems: 1`; ITS-REST `overview/Resources.md` §JSON Format),
/// so the commit is `400` and the EHR holds no composition.
#[tokio::test]
async fn a_history_with_an_empty_events_array_is_400() {
    let (_pg, app, ehr_id) = app_with_template().await;
    let mut broken = ips_canonical_composition();
    assert!(
        empty_first_history_events(&mut broken),
        "the IPS composition carries a HISTORY"
    );
    let (status, _h, body) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/ehr/{ehr_id}/composition"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(broken.to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("events"), "names the member: {body}");

    let query = json!({
        "q": format!(
            "SELECT c/uid/value FROM EHR e[ehr_id/value='{ehr_id}'] CONTAINS COMPOSITION c"
        )
    });
    let (status, _h, body) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/query/aql"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(query.to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result: Value = serde_json::from_str(&body).expect("a result set");
    assert_eq!(result["rows"], json!([]), "nothing committed: {body}");
}
