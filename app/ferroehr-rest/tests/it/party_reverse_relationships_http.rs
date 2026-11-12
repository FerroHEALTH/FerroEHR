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

// ── Version-addressed and time-travelled reads (#5473) ───────────────────────

/// The `LOCATABLE_REF` a target lists for the `parent-of` relationship the
/// source container `source_vo` holds.
fn reference(source_vo: &str) -> Value {
    json!([{
        "_type": "LOCATABLE_REF",
        "namespace": "local",
        "type": "PARTY_RELATIONSHIP",
        "id": { "_type": "HIER_OBJECT_ID", "value": source_vo },
        "path": "/relationships[openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1 \
                 and name/value='parent-of']"
    }])
}

/// Sends a canonical-JSON request and returns the `OBJECT_VERSION_ID` the
/// response body carries, with the body.
async fn written(app: &Router, request: http::Request<axum::body::Body>) -> (String, Value) {
    let (status, text) = send_body(app, request).await;
    assert!(status.is_success(), "{status}: {text}");
    let body: Value = serde_json::from_str(&text).expect("a JSON body");
    let ovid = body
        .pointer("/uid/value")
        .and_then(Value::as_str)
        .expect("uid.value")
        .to_owned();
    (ovid, body)
}

/// Updates the PERSON `previous` names with `body`, asking for its
/// representation.
async fn update(app: &Router, previous: &str, body: &Value) -> (String, Value) {
    written(
        app,
        common::req(
            "PUT",
            format!("{BASE}/demographic/person/{}", vo_of(previous)),
        )
        .accept("application/json")
        .prefer_representation()
        .if_match(format!("\"{previous}\""))
        .json(body.to_string())
        .build(),
    )
    .await
}

/// A JSON `GET` of `path`, which must answer `200`.
async fn get_json(app: &Router, path: &str) -> Value {
    let (status, text) = send_body(
        app,
        common::req("GET", path).accept("application/json").build(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{path}: {text}");
    serde_json::from_str(&text).expect("a JSON body")
}

/// The commit instant of the version `ovid`, as its `VERSION` read serves it.
async fn committed_at(app: &Router, ovid: &str) -> String {
    let version = get_json(
        app,
        &format!(
            "{BASE}/demographic/versioned_party/{}/version/{ovid}",
            vo_of(ovid)
        ),
    )
    .await;
    version
        .pointer("/commit_audit/time_committed/value")
        .and_then(Value::as_str)
        .expect("commit_audit.time_committed")
        .to_owned()
}

/// The history every test below reads: the target's version 1, then the
/// source naming the target, then the target's version 2.
struct History {
    /// The source's version that names the target.
    source: String,
    /// The target's version 1, committed before the source named it.
    target_v1: String,
    /// The target's version 2, committed after.
    target_v2: String,
    /// The representation the update to version 2 answered with.
    target_v2_written: Value,
}

async fn history(app: &Router) -> History {
    let target_v1 = create(app, &person("Target", None)).await;
    let target = vo_of(&target_v1);
    let first = create(app, &person("Source", None)).await;
    let container = vo_of(&first);
    let (source, _) = update(app, &first, &person("Source", Some((&container, &target)))).await;
    let (target_v2, target_v2_written) = update(app, &target_v1, &person("Target", None)).await;
    History {
        source,
        target_v1,
        target_v2,
        target_v2_written,
    }
}

#[tokio::test]
async fn an_old_version_serves_the_relationships_of_its_own_commit() {
    let db = common::test_db().await;
    let app = common::router_for_profile(SpecProfile::Stable, &db.pool()).await;
    let h = history(&app).await;
    let at = |ovid: &str| format!("{BASE}/demographic/person/{ovid}");

    let v1 = get_json(&app, &at(&h.target_v1)).await;
    assert!(v1.get("reverse_relationships").is_none(), "{v1}");
    let v2 = get_json(&app, &at(&h.target_v2)).await;
    assert_eq!(
        v2["reverse_relationships"],
        reference(&vo_of(&h.source)),
        "{v2}"
    );

    // The source lets go: the live read loses the reference, version 2 keeps
    // the list it was committed with.
    update(&app, &h.source, &person("Source", None)).await;
    let live = read(&app, &vo_of(&h.target_v2)).await;
    assert!(live.get("reverse_relationships").is_none(), "{live}");
    let v2 = get_json(&app, &at(&h.target_v2)).await;
    assert_eq!(
        v2["reverse_relationships"],
        reference(&vo_of(&h.source)),
        "{v2}"
    );
}

#[tokio::test]
async fn a_time_travelled_read_serves_the_relationships_of_the_version_it_finds() {
    let db = common::test_db().await;
    let app = common::router_for_profile(SpecProfile::Stable, &db.pool()).await;
    let h = history(&app).await;
    let target = vo_of(&h.target_v1);
    let at_time = |instant: &str| {
        format!(
            "{BASE}/demographic/person/{target}?version_at_time={}",
            urlencoding::encode(instant)
        )
    };

    // The instant the source named the target finds version 1, committed
    // before it.
    let named = committed_at(&app, &h.source).await;
    let found = get_json(&app, &at_time(&named)).await;
    assert_eq!(found["uid"]["value"], h.target_v1.as_str(), "{found}");
    assert!(found.get("reverse_relationships").is_none(), "{found}");

    let later = committed_at(&app, &h.target_v2).await;
    let found = get_json(&app, &at_time(&later)).await;
    assert_eq!(
        found["reverse_relationships"],
        reference(&vo_of(&h.source)),
        "{found}"
    );
}

#[tokio::test]
async fn a_version_read_carries_the_reverse_relationships_of_its_commit() {
    let db = common::test_db().await;
    let app = common::router_for_profile(SpecProfile::Stable, &db.pool()).await;
    let h = history(&app).await;
    let target = vo_of(&h.target_v1);
    let versions = format!("{BASE}/demographic/versioned_party/{target}/version");

    let v2 = get_json(&app, &format!("{versions}/{}", h.target_v2)).await;
    assert_eq!(
        v2["data"]["reverse_relationships"],
        reference(&vo_of(&h.source)),
        "{v2}"
    );
    let latest = get_json(&app, &versions).await;
    assert_eq!(latest["data"], v2["data"], "{latest}");

    let v1 = get_json(&app, &format!("{versions}/{}", h.target_v1)).await;
    assert!(v1["data"].get("reverse_relationships").is_none(), "{v1}");
    let named = committed_at(&app, &h.source).await;
    let at_time = get_json(
        &app,
        &format!("{versions}?version_at_time={}", urlencoding::encode(&named)),
    )
    .await;
    assert_eq!(at_time["data"], v1["data"], "{at_time}");
}

#[tokio::test]
async fn the_update_representation_carries_the_reverse_relationships() {
    let db = common::test_db().await;
    let app = common::router_for_profile(SpecProfile::Stable, &db.pool()).await;
    let h = history(&app).await;

    assert_eq!(
        h.target_v2_written["reverse_relationships"],
        reference(&vo_of(&h.source)),
        "{}",
        h.target_v2_written
    );
    // The same version read back serves the same document.
    let v2 = get_json(&app, &format!("{BASE}/demographic/person/{}", h.target_v2)).await;
    assert_eq!(h.target_v2_written, v2);

    // The XML representation carries the RM 1.1.0 element as well.
    let (status, xml) = send_body(
        &app,
        common::req(
            "PUT",
            format!("{BASE}/demographic/person/{}", vo_of(&h.target_v2)),
        )
        .header(header::ACCEPT, "application/xml")
        .prefer_representation()
        .if_match(format!("\"{}\"", h.target_v2))
        .json(person("Target", None).to_string())
        .build(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{xml}");
    let reverse = xml
        .split("<reverse_relationships")
        .nth(1)
        .unwrap_or_else(|| panic!("no reverse_relationships element: {xml}"));
    assert!(reverse.contains(&vo_of(&h.source)), "{xml}");
}

#[tokio::test]
async fn the_create_representation_is_the_document_its_version_read_serves() {
    let db = common::test_db().await;
    let app = common::router_for_profile(SpecProfile::Stable, &db.pool()).await;

    let (created, written) = written(
        &app,
        common::req("POST", format!("{BASE}/demographic/person"))
            .accept("application/json")
            .prefer_representation()
            .json(person("Created", None).to_string())
            .build(),
    )
    .await;
    let read_back = get_json(&app, &format!("{BASE}/demographic/person/{created}")).await;
    assert_eq!(written, read_back);
    let version = get_json(
        &app,
        &format!(
            "{BASE}/demographic/versioned_party/{}/version/{created}",
            vo_of(&created)
        ),
    )
    .await;
    assert_eq!(written, version["data"]);
}

/// `reverse_relationships()` evaluated through FerroEHR's own extension
/// route: our own design, since RM 1.2.0 makes it a function (SPECRM-124)
/// that no released REST resource serves.
async fn evaluated(app: &Router, uid_based_id: &str, query: &str) -> Value {
    get_json(
        app,
        &format!(
            "{}/demographic/party/{uid_based_id}/reverse_relationships{query}",
            common::EXT
        ),
    )
    .await
}

#[tokio::test]
async fn the_development_profile_evaluates_the_function_on_the_extension_route() {
    let db = common::test_db().await;
    let development = common::router_for_profile(SpecProfile::Development, &db.pool()).await;
    let stable = common::router_for_profile(SpecProfile::Stable, &db.pool()).await;
    let h = history(&development).await;
    let target = vo_of(&h.target_v1);

    let live = evaluated(&development, &target, "").await;
    assert_eq!(live, reference(&vo_of(&h.source)), "{live}");
    assert_eq!(live, read(&stable, &target).await["reverse_relationships"]);

    assert_eq!(evaluated(&development, &h.target_v1, "").await, json!([]));
    assert_eq!(
        evaluated(&development, &h.target_v2, "").await,
        reference(&vo_of(&h.source))
    );
    let named = committed_at(&development, &h.source).await;
    let query = format!("?version_at_time={}", urlencoding::encode(&named));
    assert_eq!(evaluated(&development, &target, &query).await, json!([]));

    // The party body itself stays the RM 1.2.0 class, which has no such
    // attribute.
    let body = read(&development, &target).await;
    assert!(body.get("reverse_relationships").is_none(), "{body}");
}
