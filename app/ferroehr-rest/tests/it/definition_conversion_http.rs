// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the ADL 1.4 to ADL 2 **conversion** extension
//! routes, `GET /definition/archetype/adl1.4/{archetype_id}/adl2` and
//! `GET /definition/template/adl1.4/{template_id}/adl2`, driven through the
//! assembled router over a real `FerroEhrService` on a real `PostgreSQL`.
//!
//! No openEHR spec governs these routes or the conversion: both are our own
//! design. What is asserted is the shape the served OpenAPI declares: the
//! ADL 2 sources, each root's log and conversion details, and the fill
//! structure.
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use serde_json::Value;

use crate::common;
use crate::common::{BASE, EXT};

const ADL14_ID: &str = "openEHR-EHR-COMPOSITION.prescription.v1";

/// A file under the shared corpus at `corpus/fixtures/`.
fn corpus(rel: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/fixtures")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Upload an ADL 1.4 source archetype over the archetype extension.
async fn upload_archetype(app: &Router, adl: String) {
    let req = Request::builder()
        .method("POST")
        .uri(format!("{EXT}/definition/archetype/adl1.4"))
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(adl))
        .expect("request");
    let (status, _, body) = common::send(app, req).await;
    assert_eq!(status, StatusCode::CREATED, "archetype upload: {body}");
}

/// Upload an OPT 1.4 over the released template upload and return its
/// `template_id`.
async fn upload_template(app: &Router, xml: String) -> String {
    let template_id = openehr_its::opt14::from_xml(&xml)
        .expect("the vendored OPT parses")
        .template_id
        .value;
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/definition/template/adl1.4"))
        .header(header::CONTENT_TYPE, "application/xml")
        .body(Body::from(xml))
        .expect("request");
    let (status, _, body) = common::send(app, req).await;
    assert_eq!(status, StatusCode::CREATED, "template upload: {body}");
    template_id
}

/// `GET` one conversion route below the extensions root and parse the body.
async fn convert(app: &Router, relative: &str) -> (StatusCode, Option<String>, Value) {
    let uri = format!("{EXT}{relative}");
    let (status, headers, body) = common::send(app, common::get(&uri)).await;
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let json = serde_json::from_str(&body).unwrap_or(Value::Null);
    (status, content_type, json)
}

/// Every root carries its ADL 2 text, a log with its three members and the
/// conversion details.
fn assert_roots_well_formed(conversion: &Value) {
    let roots = conversion["roots"].as_array().expect("roots is an array");
    assert!(!roots.is_empty(), "at least one root: {conversion}");
    for root in roots {
        assert!(
            root["archetype_id"].is_string(),
            "root names itself: {root}"
        );
        assert!(
            root["adl2"]
                .as_str()
                .is_some_and(|s| s.contains("archetype")),
            "the root carries ADL 2 source: {root}"
        );
        let log = &root["log"];
        assert!(log["external_at_codes"].is_object(), "log codes: {root}");
        assert!(log["value_sets"].is_object(), "log value sets: {root}");
        assert!(log["notes"].is_array(), "log notes: {root}");
        assert!(root["conversion_details"].is_object(), "details: {root}");
    }
}

/// A stored ADL 1.4 archetype converts to one ADL 2 root with its log, as
/// `application/json`, with no fill structure; an unknown id is a `404`.
#[tokio::test]
async fn a_stored_archetype_converts_to_adl2_with_its_log() {
    let (_pg, app) = common::test_router().await;
    upload_archetype(
        &app,
        corpus(&format!("service/knowledge/archetypes/{ADL14_ID}.adl")),
    )
    .await;

    let (status, content_type, conversion) = convert(
        &app,
        &format!("/definition/archetype/adl1.4/{ADL14_ID}/adl2"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{conversion}");
    assert!(
        content_type.is_some_and(|c| c.starts_with("application/json")),
        "the conversion is JSON"
    );
    assert_roots_well_formed(&conversion);
    assert_eq!(conversion["roots"].as_array().map(Vec::len), Some(1));
    assert_eq!(conversion["roots"][0]["archetype_id"], ADL14_ID);
    assert_eq!(conversion["structure"], serde_json::json!([]));

    let (status, _, body) = convert(
        &app,
        "/definition/archetype/adl1.4/openEHR-EHR-OBSERVATION.absent.v1/adl2",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

/// A stored ADL 1.4 operational template converts to one ADL 2 root per
/// embedded archetype root, with the slot fills that joined them; an unknown
/// template id is a `404`.
#[tokio::test]
async fn a_stored_template_converts_to_adl2_roots_and_their_structure() {
    let (_pg, app) = common::test_router().await;
    let template_id = upload_template(
        &app,
        corpus("service/knowledge/opt/minimal_observation.opt"),
    )
    .await;

    let (status, content_type, conversion) = convert(
        &app,
        &format!(
            "/definition/template/adl1.4/{}/adl2",
            urlencoding::encode(&template_id)
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{conversion}");
    assert!(
        content_type.is_some_and(|c| c.starts_with("application/json")),
        "the conversion is JSON"
    );
    assert_roots_well_formed(&conversion);
    let ids: Vec<&str> = conversion["roots"]
        .as_array()
        .expect("roots")
        .iter()
        .filter_map(|r| r["archetype_id"].as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "openEHR-EHR-COMPOSITION.minimal.v1",
            "openEHR-EHR-OBSERVATION.minimal.v1"
        ],
        "the top root first, then the embedded one"
    );
    let structure = conversion["structure"].as_array().expect("structure");
    assert!(
        structure.iter().any(|e| {
            e["parent_archetype_id"] == "openEHR-EHR-COMPOSITION.minimal.v1"
                && e["child_archetype_id"] == "openEHR-EHR-OBSERVATION.minimal.v1"
                && e["slot_node_id"].is_string()
                && e["parent_path"].is_string()
        }),
        "the COMPOSITION to OBSERVATION fill is recorded: {conversion}"
    );

    let (status, _, body) = convert(&app, "/definition/template/adl1.4/unknown.v0/adl2").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

/// The conversion log carries what the ADL 2 form drops: the Role template's
/// dangling `ac0001` reference and its capped single-valued attributes are
/// reported in the ROLE root's `conversion_details`.
#[tokio::test]
async fn the_conversion_log_reports_what_adl2_cannot_express() {
    let (_pg, app) = common::test_router().await;
    let template_id = upload_template(&app, corpus("opt/valid/role.v2.opt")).await;

    let (status, _, conversion) = convert(
        &app,
        &format!(
            "/definition/template/adl1.4/{}/adl2",
            urlencoding::encode(&template_id)
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{conversion}");
    let role = conversion["roots"]
        .as_array()
        .expect("roots")
        .iter()
        .find(|r| r["archetype_id"] == "openEHR-DEMOGRAPHIC-ROLE.Role.v1")
        .unwrap_or_else(|| panic!("the ROLE root converts: {conversion}"));
    let details = role["conversion_details"]
        .as_object()
        .expect("conversion details");
    assert!(
        details
            .values()
            .any(|v| v.as_str().is_some_and(|s| s.contains("ac0001"))),
        "the dangling ac0001 is reported: {details:?}"
    );
    assert!(
        details
            .keys()
            .any(|k| k.starts_with("single_attribute_occurrences.")),
        "the occurrences cap is reported: {details:?}"
    );
}
