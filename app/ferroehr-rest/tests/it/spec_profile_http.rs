// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests of the `spec_profile` read-time refusal, driven
//! through the assembled router over a real `FerroEhrService` on a real
//! `PostgreSQL` 18 (auth disabled).
//!
//! **Our own extension** — no openEHR spec governs runtime
//! specification-generation selection. The status adjudication is HTTP's own:
//! RFC 9110 §15.5.10 assigns `409` to "a conflict with the current state of
//! the target resource" that the response should describe well enough to
//! resolve, which is what a stored body outside the deployment's declared
//! generation set is. The ITS-REST overview
//! (`docs/specs/openehr/ITS-REST/specifications/docs/overview/
//! Requests_and_responses.md` §"HTTP status codes") glosses `409` the same
//! way; its `406` row is proactive content negotiation
//! (RFC 9110 §15.5.7) and no request header changes this outcome.
//!
//! The development-only construct committed here is `GENERIC_ENTRY.data`
//! holding a `CLUSTER`: RM 1.1.0 types that attribute `ITEM_TREE`, and
//! SPECRM-18 retyped it to the abstract `ITEM` (= `CLUSTER` | `ELEMENT`) after
//! that release (`docs/specs/openehr/RM/docs/integration/
//! master00-amendment_record.adoc`, issue 1.0, above the `RM Release 1.1.0`
//! marker), so the two generations disagree about exactly this body.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use ferroehr::config::profile::SpecProfile;
use ferroehr::service::FerroEhrService;
use ferroehr_rest::config::AppConfig;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";

/// Send one request and collect status + body text.
async fn send(app: &Router, req: Request<Body>) -> (StatusCode, String) {
    let resp = app.clone().oneshot(req).await.expect("response");
    let status = resp.status();
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// A COMPOSITION whose one content item is a `GENERIC_ENTRY` carrying a
/// `CLUSTER` — surface only the development generation set defines.
fn development_only_composition() -> Value {
    json!({
        "_type": "COMPOSITION",
        "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-EHR-COMPOSITION.encounter.v1" },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": "development surface" },
        "language": { "_type": "CODE_PHRASE",
                      "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" },
                      "code_string": "en" },
        "territory": { "_type": "CODE_PHRASE",
                       "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_3166-1" },
                       "code_string": "NL" },
        "category": { "_type": "DV_CODED_TEXT", "value": "event",
                      "defining_code": { "_type": "CODE_PHRASE",
                                         "terminology_id": { "_type": "TERMINOLOGY_ID",
                                                             "value": "openehr" },
                                         "code_string": "433" } },
        "composer": { "_type": "PARTY_IDENTIFIED", "name": "conformance tester" },
        "content": [ {
            "_type": "GENERIC_ENTRY",
            "name": { "_type": "DV_TEXT", "value": "entry" },
            "archetype_node_id": "openEHR-EHR-GENERIC_ENTRY.msg.v1",
            "data": {
                "_type": "CLUSTER",
                "name": { "_type": "DV_TEXT", "value": "data" },
                "archetype_node_id": "at0000",
                "items": [ {
                    "_type": "ELEMENT",
                    "name": { "_type": "DV_TEXT", "value": "leaf" },
                    "archetype_node_id": "at0001",
                    "value": { "_type": "DV_TEXT", "value": "x" }
                } ]
            }
        } ]
    })
}

/// A router over the given profile, on an existing pool.
fn router_for(profile: SpecProfile, pool: &sqlx::PgPool) -> Router {
    let mut config = AppConfig::default();
    config.auth.enabled = false;
    config.spec_profile = profile;
    common::router_with(
        config,
        Arc::new(
            FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool))
                .with_spec_profile(profile),
        ),
    )
}

/// A stored version the active profile cannot express is a `409`, not a served
/// body and not a down-conversion; the same object reads normally on the
/// profile that accepted it.
#[tokio::test]
async fn a_development_only_composition_is_a_conflict_under_the_stable_profile() {
    let db = common::test_db().await;
    let development = router_for(SpecProfile::Development, &db.pool());

    let (status, body) = send(
        &development,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/ehr"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json")
            .header("Prefer", "return=representation")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let ehr_id = serde_json::from_str::<Value>(&body).expect("ehr body")["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned();

    let (status, body) = send(
        &development,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/ehr/{ehr_id}/composition"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json")
            .header("Prefer", "return=representation")
            .body(Body::from(development_only_composition().to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let version_uid =
        serde_json::from_str::<Value>(&body).expect("composition body")["uid"]["value"]
            .as_str()
            .expect("uid.value")
            .to_owned();
    let vo_id = version_uid
        .split("::")
        .next()
        .expect("object id")
        .to_owned();

    // The profile that accepted it still serves it.
    let (status, _) = send(
        &development,
        Request::builder()
            .method("GET")
            .uri(format!("{BASE}/ehr/{ehr_id}/composition/{vo_id}"))
            .header(header::ACCEPT, "application/json")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // The stable profile refuses it — loudly, with the remedy in the body.
    let stable = router_for(SpecProfile::Stable, &db.pool());
    let (status, body) = send(
        &stable,
        Request::builder()
            .method("GET")
            .uri(format!("{BASE}/ehr/{ehr_id}/composition/{vo_id}"))
            .header(header::ACCEPT, "application/json")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.contains("spec_profile"), "{body}");
    assert!(body.contains(&version_uid), "{body}");
    assert!(body.contains("development"), "{body}");

    // An `Accept` change cannot make the refusal go away — this is not content
    // negotiation (RFC 9110 §15.5.7), which is why it is not a `406`.
    let (status, _) = send(
        &stable,
        Request::builder()
            .method("GET")
            .uri(format!("{BASE}/ehr/{ehr_id}/composition/{vo_id}"))
            .header(header::ACCEPT, "application/xml")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

/// An AQL whole-object projection reaches the same stored body the version read
/// reaches, so the query endpoint surfaces the same `409` — while a leaf
/// projection over the identical rows still answers `200` (the honest boundary:
/// it serves data values, not version bodies).
#[tokio::test]
async fn an_aql_whole_object_projection_is_a_conflict_under_the_stable_profile() {
    let db = common::test_db().await;
    let development = router_for(SpecProfile::Development, &db.pool());

    let (status, body) = send(
        &development,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/ehr"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json")
            .header("Prefer", "return=representation")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let ehr_id = serde_json::from_str::<Value>(&body).expect("ehr body")["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned();

    let (status, body) = send(
        &development,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/ehr/{ehr_id}/composition"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json")
            .header("Prefer", "return=representation")
            .body(Body::from(development_only_composition().to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let version_uid =
        serde_json::from_str::<Value>(&body).expect("composition body")["uid"]["value"]
            .as_str()
            .expect("uid.value")
            .to_owned();

    let query = |aql: &str| {
        format!(
            "{BASE}/query/aql?ehr_id={ehr_id}&q={}",
            urlencoding::encode(aql)
        )
    };
    let whole_object = query("SELECT c FROM EHR e CONTAINS COMPOSITION c");
    let leaf = query("SELECT c/name/value FROM EHR e CONTAINS COMPOSITION c");

    // The profile that accepted the body answers both projections.
    for uri in [&whole_object, &leaf] {
        let (status, body) = send(
            &development,
            Request::builder()
                .method("GET")
                .uri(uri)
                .header(header::ACCEPT, "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    }

    let stable = router_for(SpecProfile::Stable, &db.pool());
    let (status, body) = send(
        &stable,
        Request::builder()
            .method("GET")
            .uri(&whole_object)
            .header(header::ACCEPT, "application/json")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.contains("spec_profile"), "{body}");
    assert!(body.contains(&version_uid), "{body}");
    assert!(body.contains("development"), "{body}");

    let (status, body) = send(
        &stable,
        Request::builder()
            .method("GET")
            .uri(&leaf)
            .header(header::ACCEPT, "application/json")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a leaf projection serves values, not version bodies: {body}"
    );
}

/// The served `OpenAPI` declares the same `409` on every query operation, which
/// is where a whole-object projection can hit it.
#[tokio::test]
async fn the_served_openapi_declares_the_profile_conflict_on_the_query_operations() {
    let (_db, app) = common::test_router().await;
    let (status, body) = send(
        &app,
        Request::builder()
            .method("GET")
            .uri("/ferroehr/rest/api-docs/openapi.json")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let doc: Value = serde_json::from_str(&body).expect("served openapi json");

    for path in [
        "/query/aql",
        "/query/{qualified_query_name}",
        "/query/{qualified_query_name}/{version}",
    ] {
        for method in ["get", "post"] {
            let full = format!("{BASE}{path}");
            let op = doc["paths"][&full][method]
                .as_object()
                .unwrap_or_else(|| panic!("{method} {full} is served"));
            let description = op["responses"]["409"]["description"]
                .as_str()
                .unwrap_or_else(|| panic!("{method} {full} declares a 409"));
            assert!(
                description.contains("spec_profile"),
                "{method} {full}: the 409 must name the spec_profile cause: {description}"
            );
            assert!(
                description.contains("OUR OWN EXTENSION"),
                "{method} {full}: the 409 must carry the extension flag: {description}"
            );
        }
    }
}

/// The served `OpenAPI` declares the `409` on every version-read operation it
/// can occur on, flagged as our own extension (the served document is the only
/// one we publish — root `CLAUDE.md`).
#[tokio::test]
async fn the_served_openapi_declares_the_profile_conflict_on_version_reads() {
    let (_db, app) = common::test_router().await;
    let (status, body) = send(
        &app,
        Request::builder()
            .method("GET")
            .uri("/ferroehr/rest/api-docs/openapi.json")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let doc: Value = serde_json::from_str(&body).expect("served openapi json");

    // (path, method) of the version-read operations the gate sits behind.
    let reads = [
        ("/ehr/{ehr_id}/composition/{uid_based_id}", "get"),
        ("/ehr/{ehr_id}/ehr_status/{version_uid}", "get"),
        ("/ehr/{ehr_id}/directory", "get"),
        (
            "/ehr/{ehr_id}/versioned_composition/{versioned_object_uid}/version/{version_uid}",
            "get",
        ),
        (
            "/ehr/{ehr_id}/versioned_ehr_status/version/{version_uid}",
            "get",
        ),
        ("/ehr/{ehr_id}/directory/{version_uid}", "get"),
        ("/demographic/person/{uid_based_id}", "get"),
        (
            "/demographic/versioned_party/{versioned_object_uid}/version/{version_uid}",
            "get",
        ),
    ];
    for (path, method) in reads {
        let full = format!("{BASE}{path}");
        let op = doc["paths"][&full][method]
            .as_object()
            .unwrap_or_else(|| panic!("{method} {full} is served"));
        let description = op["responses"]["409"]["description"]
            .as_str()
            .unwrap_or_else(|| panic!("{method} {full} declares a 409"));
        assert!(
            description.contains("spec_profile"),
            "{method} {full}: the 409 must name the spec_profile cause: {description}"
        );
        assert!(
            description.contains("OUR OWN EXTENSION"),
            "{method} {full}: the 409 must carry the extension flag: {description}"
        );
    }
}
