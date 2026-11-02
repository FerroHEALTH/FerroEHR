// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests of `PARTY.reverse_relationships` per specification
//! profile, through the assembled router over a real `FerroEhrService` on a
//! real `PostgreSQL` 18 (auth disabled).
//!
//! RM 1.1.0 demographic `master02-demographic_package.adoc` §Party
//! Relationships: "each Party involved in a relationship includes it in its
//! relationships list, if it is at the source end, or in the
//! `_reverse_relationships_` list, if at the target end." RM 1.2.0 turns the
//! attribute into a function (SPECRM-124), so the development profile serves
//! no such member.

use axum::Router;
use http::{StatusCode, header};
use serde_json::{Value, json};

use ferroehr::config::profile::SpecProfile;

use crate::common;
use crate::common::send_body;

const BASE: &str = "/ferroehr/rest/openehr/v1";

/// A spec-valid PERSON named `name`, optionally carrying one relationship
/// from `source` to `target`.
fn person(name: &str, relationship: Option<(&str, &str)>) -> Value {
    let mut body = json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": name },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "legal name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "structure" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0003",
                    "name": { "_type": "DV_TEXT", "value": "family" },
                    "value": { "_type": "DV_TEXT", "value": name }
                }]
            }
        }]
    });
    if let Some((source, target)) = relationship {
        let relationships = json!([{
            "_type": "PARTY_RELATIONSHIP",
            "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1",
            "name": { "_type": "DV_TEXT", "value": "parent-of" },
            "source": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                        "id": { "_type": "HIER_OBJECT_ID", "value": source } },
            "target": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                        "id": { "_type": "HIER_OBJECT_ID", "value": target } }
        }]);
        body.as_object_mut()
            .expect("a PERSON body is an object")
            .insert("relationships".to_owned(), relationships);
    }
    body
}

/// Creates a PERSON and returns its `OBJECT_VERSION_ID`.
async fn create(app: &Router, body: &Value) -> String {
    let (status, text) = send_body(
        app,
        common::req("POST", format!("{BASE}/demographic/person"))
            .accept("application/json")
            .prefer_representation()
            .json(body.to_string())
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{text}");
    serde_json::from_str::<Value>(&text)
        .expect("person body")
        .pointer("/uid/value")
        .and_then(Value::as_str)
        .expect("uid.value")
        .to_owned()
}

/// The versioned-object uuid of an `OBJECT_VERSION_ID`.
fn vo_of(ovid: &str) -> String {
    ovid.split("::").next().expect("vo uuid").to_owned()
}

/// Creates two PERSONs, then updates the first with a relationship whose
/// target is the second. Returns `(source ovid after the update, target vo)`.
async fn related_pair(app: &Router) -> (String, String) {
    let source = create(app, &person("Source", None)).await;
    let target = vo_of(&create(app, &person("Target", None)).await);
    let source_vo = vo_of(&source);
    let (status, text) = send_body(
        app,
        common::req("PUT", format!("{BASE}/demographic/person/{source_vo}"))
            .accept("application/json")
            .prefer_representation()
            .if_match(format!("\"{source}\""))
            .json(person("Source", Some((&source_vo, &target))).to_string())
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let updated = serde_json::from_str::<Value>(&text)
        .expect("person body")
        .pointer("/uid/value")
        .and_then(Value::as_str)
        .expect("uid.value")
        .to_owned();
    (updated, target)
}

/// Reads a PERSON in canonical JSON.
async fn read(app: &Router, vo: &str) -> Value {
    let (status, text) = send_body(
        app,
        common::req("GET", format!("{BASE}/demographic/person/{vo}"))
            .accept("application/json")
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    serde_json::from_str(&text).expect("person body")
}

#[tokio::test]
async fn the_stable_profile_lists_the_relationship_at_its_target() {
    let db = common::test_db().await;
    let app = common::router_for_profile(SpecProfile::Stable, &db.pool()).await;
    let (source, target) = related_pair(&app).await;

    let body = read(&app, &target).await;
    assert_eq!(
        body["reverse_relationships"],
        json!([{
            "_type": "LOCATABLE_REF",
            "namespace": "local",
            "type": "PARTY_RELATIONSHIP",
            "id": { "_type": "HIER_OBJECT_ID", "value": vo_of(&source) },
            "path": "/relationships[openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1 \
                     and name/value='parent-of']"
        }]),
        "{body}"
    );
    // The source end holds the relationship by value and is no target.
    let source_body = read(&app, &vo_of(&source)).await;
    assert!(
        source_body.get("reverse_relationships").is_none(),
        "{source_body}"
    );
}

#[tokio::test]
async fn the_stable_profile_drops_the_reference_once_the_source_lets_go() {
    let db = common::test_db().await;
    let app = common::router_for_profile(SpecProfile::Stable, &db.pool()).await;
    let (source, target) = related_pair(&app).await;

    // A new source version without the relationship.
    let (status, text) = send_body(
        &app,
        common::req(
            "PUT",
            format!("{BASE}/demographic/person/{}", vo_of(&source)),
        )
        .accept("application/json")
        .if_match(format!("\"{source}\""))
        .json(person("Source", None).to_string())
        .build(),
    )
    .await;
    assert!(status.is_success(), "{status}: {text}");
    let body = read(&app, &target).await;
    assert!(body.get("reverse_relationships").is_none(), "{body}");
}

#[tokio::test]
async fn the_stable_profile_drops_the_reference_of_a_deleted_source() {
    let db = common::test_db().await;
    let app = common::router_for_profile(SpecProfile::Stable, &db.pool()).await;
    let (source, target) = related_pair(&app).await;

    let (status, text) = send_body(
        &app,
        common::req("DELETE", format!("{BASE}/demographic/person/{source}")).build(),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{text}");
    let body = read(&app, &target).await;
    assert!(body.get("reverse_relationships").is_none(), "{body}");
}

#[tokio::test]
async fn the_development_profile_serves_no_reverse_relationships() {
    let db = common::test_db().await;
    let app = common::router_for_profile(SpecProfile::Development, &db.pool()).await;
    let (_source, target) = related_pair(&app).await;

    let body = read(&app, &target).await;
    assert!(body.get("reverse_relationships").is_none(), "{body}");
}

#[tokio::test]
async fn a_stable_profile_xml_read_of_a_target_still_serves() {
    let db = common::test_db().await;
    let app = common::router_for_profile(SpecProfile::Stable, &db.pool()).await;
    let (_source, target) = related_pair(&app).await;

    let (status, text) = send_body(
        &app,
        common::req("GET", format!("{BASE}/demographic/person/{target}"))
            .header(header::ACCEPT, "application/xml")
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
}

/// Reads a PERSON in canonical XML.
async fn read_xml(app: &Router, vo: &str) -> String {
    let (status, text) = send_body(
        app,
        common::req("GET", format!("{BASE}/demographic/person/{vo}"))
            .header(header::ACCEPT, "application/xml")
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    text
}

#[tokio::test]
async fn a_stable_profile_xml_read_lists_the_relationship_at_its_target() {
    let db = common::test_db().await;
    let app = common::router_for_profile(SpecProfile::Stable, &db.pool()).await;
    let (source, target) = related_pair(&app).await;

    let xml = read_xml(&app, &target).await;
    let reverse = xml
        .split("<reverse_relationships")
        .nth(1)
        .and_then(|rest| rest.split("</reverse_relationships>").next())
        .unwrap_or_else(|| panic!("no reverse_relationships element: {xml}"));
    assert!(reverse.contains(&vo_of(&source)), "{xml}");
    assert!(reverse.contains("PARTY_RELATIONSHIP"), "{xml}");
    assert_eq!(xml.matches("<reverse_relationships").count(), 1, "{xml}");
}

#[tokio::test]
async fn a_development_profile_xml_read_serves_no_reverse_relationships() {
    let db = common::test_db().await;
    let app = common::router_for_profile(SpecProfile::Development, &db.pool()).await;
    let (_source, target) = related_pair(&app).await;

    let xml = read_xml(&app, &target).await;
    assert!(!xml.contains("reverse_relationships"), "{xml}");
}
