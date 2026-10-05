// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the demographic `ITEM_TAG` wire (group-13 audit,
//! #389): the released dual-form `uid_based_id` addresses DISTINCT tag
//! collections (the `VERSIONED_PARTY` container vs ONE of its VERSIONs — the
//! disjointness law of RM `common.item_tag` `ITEM_TAG.target`), the tags
//! GET/DELETE 404 on a nonexistent / wrong-kind target
//! (`404_unknown_uid_based_id.yaml`; the kind-checked-routes law), both
//! write-wrapper request headers land on their own collections, the RM target
//! shape is the bare `UID_BASED_ID`, and the `owner_id` follows the released
//! examples' `{namespace: local, type: SYSTEM}` shape.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use ferroehr::config::auth::AuthConfig;
use ferroehr::config::server::ServerConfig;
use ferroehr_rest::config::AppConfig;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";

fn person_body() -> Value {
    json!({
        "_type": "PERSON",
        "name": { "_type": "DV_TEXT", "value": "PERSON" },
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
            "rm_version": "1.1.0"
        },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "name": { "_type": "DV_TEXT", "value": "legal identity" },
            "archetype_node_id": "at0002",
            "details": {
                "_type": "ITEM_TREE",
                "name": { "_type": "DV_TEXT", "value": "tree" },
                "archetype_node_id": "at0003",
                "items": [{
                    "_type": "ELEMENT",
                    "name": { "_type": "DV_TEXT", "value": "name" },
                    "archetype_node_id": "at0004",
                    "value": { "_type": "DV_TEXT", "value": "Jane Doe" }
                }]
            }
        }]
    })
}

fn config() -> AppConfig {
    AppConfig {
        server: ServerConfig {
            bind: "127.0.0.1:0".to_owned(),
            base_path: BASE.to_owned(),
            max_in_flight: 1024,
            swagger_ui: ferroehr::config::management::AccessLevel::Off,
            cors_permissive: false,
            ..Default::default()
        },
        auth: AuthConfig {
            enabled: false,
            basic: None,
            oidc: None,
            ..AuthConfig::default()
        },
        ..Default::default()
    }
}

async fn app() -> (testkit::TestDb, Router) {
    let (pg, service) = common::test_service().await;
    (pg, common::router_with(config(), service))
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, header::HeaderMap, String) {
    let resp = app.clone().oneshot(req).await.expect("response");
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

/// Create a person, returning its full `version_uid` (from the create `ETag`).
async fn create_person(app: &Router) -> String {
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/demographic/person"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(person_body().to_string()))
        .unwrap();
    let (status, h, body) = send(app, req).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    h.get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag")
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_owned()
}

fn vo_of(ovid: &str) -> &str {
    ovid.split("::").next().expect("vo uuid")
}

async fn put_tags(app: &Router, uid: &str, tags: Value) -> (StatusCode, String) {
    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/demographic/person/{uid}/tags"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("Prefer", "return=representation")
        .body(Body::from(tags.to_string()))
        .unwrap();
    let (status, _h, body) = send(app, req).await;
    (status, body)
}

async fn get_tags(app: &Router, kind: &str, uid: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/demographic/{kind}/{uid}/tags"))
        .header(header::ACCEPT, "application/json")
        .body(Body::empty())
        .unwrap();
    let (status, _h, body) = send(app, req).await;
    (status, body)
}

fn keys(body: &str) -> Vec<String> {
    serde_json::from_str::<Vec<Value>>(body)
        .unwrap_or_default()
        .iter()
        .filter_map(|t| t.get("key").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

// ── the dual-form uid addresses DISTINCT collections ─────────────────────────

#[tokio::test]
async fn container_and_version_tag_collections_are_disjoint() {
    let (_pg, app) = app().await;
    let ovid = create_person(&app).await;
    let vo = vo_of(&ovid).to_owned();

    // Container-addressed PUT and version-addressed PUT write different sets.
    let (status, body) = put_tags(&app, &vo, json!([{ "key": "container-tag" }])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = put_tags(&app, &ovid, json!([{ "key": "version-tag" }])).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Each addressing form reads back exactly its own collection (the released
    // dual-form sentence: the container form addresses the VERSIONED_PARTY
    // container, the version form that VERSION).
    let (status, body) = get_tags(&app, "person", &vo).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(keys(&body), vec!["container-tag"], "container collection");
    let (status, body) = get_tags(&app, "person", &ovid).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(keys(&body), vec!["version-tag"], "version collection");

    // A version-addressed tag's target is the OBJECT_VERSION_ID; the
    // container's is the HIER_OBJECT_ID (RM item_tag.adoc target shape).
    let tags: Vec<Value> = serde_json::from_str(&body).unwrap();
    assert_eq!(
        tags[0]["target"]["_type"], "OBJECT_VERSION_ID",
        "version-addressed target is an OBJECT_VERSION_ID: {tags:?}"
    );
    // owner_id follows the released examples' shape.
    assert_eq!(tags[0]["owner_id"]["namespace"], "local");
    assert_eq!(tags[0]["owner_id"]["type"], "SYSTEM");

    // Deleting by key on the VERSION leaves the container collection intact.
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("{BASE}/demographic/person/{ovid}/tags/version-tag"))
        .body(Body::empty())
        .unwrap();
    let (status, _h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (_s, body) = get_tags(&app, "person", &vo).await;
    assert_eq!(keys(&body), vec!["container-tag"], "container survives");
    let (_s, body) = get_tags(&app, "person", &ovid).await;
    assert!(keys(&body).is_empty(), "version collection cleared");
}

// ── the tags GET/DELETE gate the addressed target ────────────────────────────

#[tokio::test]
async fn tags_get_on_unknown_party_is_404() {
    let (_pg, app) = app().await;
    let (status, body) = get_tags(&app, "person", "00000000-0000-4000-8000-000000000000").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "404_unknown_uid_based_id: 'returned when the uid_based_id does not \
         exist' — never an empty 200 list: {body}"
    );
}

#[tokio::test]
async fn tags_routes_are_kind_checked() {
    let (_pg, app) = app().await;
    let ovid = create_person(&app).await;
    let vo = vo_of(&ovid).to_owned();
    // A person's container addressed through the agent tag route → 404
    // (the kind-checked-routes law; five kinds).
    let (status, body) = get_tags(&app, "agent", &vo).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    // DELETE through the wrong kind must not reach the tags either.
    let (status, body) = put_tags(&app, &vo, json!([{ "key": "t" }])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("{BASE}/demographic/agent/{vo}/tags/t"))
        .body(Body::empty())
        .unwrap();
    let (status, _h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (_s, body) = get_tags(&app, "person", &vo).await;
    assert_eq!(keys(&body), vec!["t"], "the tag survived the foreign route");
}

#[tokio::test]
async fn tags_get_on_unknown_version_is_404() {
    let (_pg, app) = app().await;
    let ovid = create_person(&app).await;
    let vo = vo_of(&ovid);
    let system = ovid.split("::").nth(1).expect("system id");
    let (status, body) = get_tags(&app, "person", &format!("{vo}::{system}::99")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a version-addressed target must name an existing version: {body}"
    );
}

// ── the write-wrapper headers land on their own collections ─────────────────

#[tokio::test]
async fn wrapper_headers_write_distinct_collections_on_update() {
    let (_pg, app) = app().await;
    let v1 = create_person(&app).await;
    let vo = vo_of(&v1).to_owned();
    // The released update declares `openehr-version-item-tag`; its prose says
    // "`openehr-item-tag` or `openehr-version-item-tag`" — both are accepted,
    // each addressing its own target.
    let put = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/demographic/person/{vo}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("If-Match", format!("\"{v1}\""))
        .header("openehr-item-tag", "key=\"container-side\"")
        .header("openehr-version-item-tag", "key=\"version-side\"")
        .body(Body::from(person_body().to_string()))
        .unwrap();
    let (status, h, body) = send(&app, put).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let v2 = h
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag")
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_owned();
    // The response echoes each header with its own stored set.
    let item = h
        .get("openehr-item-tag")
        .and_then(|v| v.to_str().ok())
        .expect("openehr-item-tag echoed");
    let version = h
        .get("openehr-version-item-tag")
        .and_then(|v| v.to_str().ok())
        .expect("openehr-version-item-tag echoed");
    assert!(item.contains("container-side"), "{item}");
    assert!(version.contains("version-side"), "{version}");
    // And the dedicated routes see the same split.
    let (_s, body) = get_tags(&app, "person", &vo).await;
    assert_eq!(keys(&body), vec!["container-side"]);
    let (_s, body) = get_tags(&app, "person", &v2).await;
    assert_eq!(keys(&body), vec!["version-side"]);
}

// ── the extension relationship delete echoes the latest version on 409 ──────

#[tokio::test]
async fn relationship_stale_delete_conflict_echoes_latest_etag() {
    let (_pg, app) = app().await;
    let source = create_person(&app).await;
    let target = create_person(&app).await;
    let rel = json!({
        "_type": "PARTY_RELATIONSHIP",
        "name": { "_type": "DV_TEXT", "value": "PARTY_RELATIONSHIP" },
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                "value": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1" },
            "rm_version": "1.1.0"
        },
        "source": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": vo_of(&source) } },
        "target": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": vo_of(&target) } }
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/demographic/party_relationship"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(rel.to_string()))
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let r1 = h
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag")
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_owned();
    // Supersede r1.
    let put = Request::builder()
        .method("PUT")
        .uri(format!(
            "{BASE}/demographic/party_relationship/{}",
            vo_of(&r1)
        ))
        .header(header::CONTENT_TYPE, "application/json")
        .header("If-Match", format!("\"{r1}\""))
        .body(Body::from(rel.to_string()))
        .unwrap();
    let (status, h, body) = send(&app, put).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let r2 = h
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag")
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_owned();
    // Delete at the superseded uid → 409 + the latest version's ETag,
    // matching the party delete this extension mirrors.
    let del = Request::builder()
        .method("DELETE")
        .uri(format!("{BASE}/demographic/party_relationship/{r1}"))
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, del).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let echoed = h
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("409 echoes ETag")
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_owned();
    assert_eq!(echoed, r2, "the latest version_uid rides the 409 ETag");
}

// ── the ITEM_TAG identity is the (key, target_path) PAIR ─────────────────────

/// Two same-key tags on different `target_paths` coexist on one party target and
/// the PUT round-trips both — ITS-REST overview `Requests_and_responses.md`
/// §openehr-item-tag and openehr-version-item-tag ("uniquely identified by
/// their `key` and `target_path` pair attributes"); RM common master07-tags.
/// The run-2 triage regression (2026-07-28): the demographic PUT deduped by
/// key alone and silently collapsed the pair (the EHR side had the identity
/// fix since #369; this seam did not).
#[tokio::test]
async fn same_key_tags_on_different_target_paths_coexist() {
    let (_pg, app) = app().await;
    let ovid = create_person(&app).await;
    let vo = vo_of(&ovid).to_owned();
    let (status, body) = put_tags(
        &app,
        &vo,
        serde_json::json!([
            { "key": "flag", "value": "a" },
            { "key": "flag", "value": "b", "target_path": "/details" }
        ]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = get_tags(&app, "person", &vo).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let tags: Vec<Value> = serde_json::from_str(&body).unwrap();
    assert_eq!(
        tags.len(),
        2,
        "both (key, target_path) identities persist: {body}"
    );
    assert!(
        tags.iter().all(|t| t["key"] == "flag"),
        "the shared key survives on both: {body}"
    );
    let paths: Vec<Option<&str>> = tags
        .iter()
        .map(|t| t.get("target_path").and_then(Value::as_str))
        .collect();
    assert!(
        paths.contains(&None) && paths.contains(&Some("/details")),
        "the two identities are distinguished by target_path: {body}"
    );
}
