// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the **MESSAGE** extension group — the SM
//! EHR-Extract service (`/message/export*`, `/message/import*`,
//! `i_ehr_extract_service.adoc`) — driven through the assembled router over a real
//! `FerroEhrService` on a real `PostgreSQL`.
//!
//! **No openEHR spec governs these routes**: ITS-REST 1.1.0 publishes seven API
//! groups and no message/extract API at all, so the operation SEMANTICS
//! come from the vendored SM chapter
//! (`docs/specs/openehr/SM/docs/openehr_platform/master09-message_service.adoc`)
//! and the RM EHR-Extract IM (`docs/specs/openehr/RM/docs/ehr_extract/`), while
//! the wire shape is our own design/extension. What is asserted here is exactly
//! what the CNF extension bindings drive: the `message-extract` family in
//! Veredictum's `artifacts/vocab/wire_surface.yaml`, over the same corpus
//! fixtures the CNF cases carry.
#![expect(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";
/// A syntactically valid id that is never created — the "unknown" probe.
const ABSENT: &str = "00000000-0000-0000-0000-000000000000";

/// The CNF catalogue's own messaging corpus — the same bytes the CNF cases
/// send, so a green case here and a green case there mean the same thing.
fn corpus(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/fixtures/messaging")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, String) {
    let resp = app.clone().oneshot(req).await.expect("response");
    let status = resp.status();
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn get(path: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(format!("{BASE}{path}"))
        .body(Body::empty())
        .expect("request")
}

fn post(path: &str, content_type: &str, body: String) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("{BASE}{path}"))
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))
        .expect("request")
}

/// Create an EHR through the released wire and return its id.
async fn create_ehr(app: &Router) -> String {
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr"))
        .header("Prefer", "return=representation")
        .body(Body::empty())
        .expect("request");
    let (status, body) = send(app, req).await;
    assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
    let value: Value = serde_json::from_str(&body).expect("EHR json");
    value["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned()
}

// ── I_EHR_EXTRACT_SERVICE ────────────────────────────────────────────────────

/// `export_ehrs(an_ehr_id)` answers `200` with the SM `List<EXTRACT>`: one
/// `EXTRACT` carrying the EHR's versioned objects (an EHR fresh off `POST /ehr`
/// holds its `EHR_STATUS` and `EHR_ACCESS` — RM ehr `master04` §EHR Creation).
#[tokio::test]
async fn export_ehrs_returns_one_extract_for_an_existing_ehr() {
    let (_pg, app) = common::test_router().await;
    let ehr_id = create_ehr(&app).await;

    let (status, body) = send(&app, get(&format!("/message/export/{ehr_id}"))).await;
    assert_eq!(status, StatusCode::OK, "export: {body}");
    let extracts: Value = serde_json::from_str(&body).expect("extract list json");
    let list = extracts.as_array().expect("a JSON array");
    assert_eq!(list.len(), 1, "one EXTRACT per exported EHR");
    assert_eq!(list[0]["_type"], "EXTRACT");
    let items = list[0]["chapters"][0]["items"]
        .as_array()
        .expect("chapter items");
    assert!(
        !items.is_empty(),
        "the openEHR chapter carries the EHR's versioned objects"
    );
}

/// SM `ehr_id_does_not_exist` → `404`.
#[tokio::test]
async fn export_ehrs_of_an_unknown_ehr_is_not_found() {
    let (_pg, app) = common::test_router().await;
    let (status, _body) = send(&app, get(&format!("/message/export/{ABSENT}"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// `export_ehr_extracts(extract_spec)` answers `200` with one `EXTRACT` per
/// manifest entity — the manifest naming the EHR by `ehr_id`
/// (`extract_manifest.adoc` / `extract_entity_manifest.adoc`).
#[tokio::test]
async fn export_ehr_extracts_honours_the_manifest() {
    let (_pg, app) = common::test_router().await;
    let ehr_id = create_ehr(&app).await;

    let spec = serde_json::json!({
        "_type": "EXTRACT_SPEC",
        "extract_type": {
            "_type": "DV_CODED_TEXT",
            "value": "openehr-ehr",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "openehr-ehr"
            }
        },
        "include_multimedia": false,
        "priority": 0,
        "link_depth": 0,
        "manifest": {
            "_type": "EXTRACT_MANIFEST",
            "entities": [ {
                "_type": "EXTRACT_ENTITY_MANIFEST",
                "extract_id_key": ehr_id,
                "ehr_id": ehr_id
            } ]
        }
    });
    let (status, body) = send(
        &app,
        post("/message/export", "application/json", spec.to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "export by spec: {body}");
    let extracts: Value = serde_json::from_str(&body).expect("extract list json");
    assert_eq!(extracts.as_array().expect("array").len(), 1);
}

/// An entity naming neither `ehr_id` nor `subject_id` cannot be resolved — SM
/// `precondition_violation` → `400`.
#[tokio::test]
async fn export_ehr_extracts_with_an_unresolvable_entity_is_bad_request() {
    let (_pg, app) = common::test_router().await;
    let spec = serde_json::json!({
        "_type": "EXTRACT_SPEC",
        "extract_type": {
            "_type": "DV_CODED_TEXT",
            "value": "openehr-ehr",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "openehr-ehr"
            }
        },
        "include_multimedia": false,
        "priority": 0,
        "link_depth": 0,
        "manifest": {
            "_type": "EXTRACT_MANIFEST",
            "entities": [ { "_type": "EXTRACT_ENTITY_MANIFEST", "extract_id_key": "1" } ]
        }
    });
    let (status, body) = send(
        &app,
        post("/message/export", "application/json", spec.to_string()),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "unresolvable entity: {body}"
    );
}

/// `import_ehr` with no `ehr_id` clones the EHR under the SOURCE identifier the
/// extract's `EXTRACT_SPEC` names (RM common `master06` §Copying Case 1), and
/// the response names what it created.
#[tokio::test]
async fn import_ehr_without_an_id_reuses_the_source_identifier() {
    let (_pg, app) = common::test_router().await;

    let (status, body) = send(
        &app,
        post(
            "/message/import",
            "application/json",
            corpus("ehr_extract.v1.json"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "import_ehr: {body}");
    let created: Value = serde_json::from_str(&body).expect("identifier json");
    assert_eq!(created["uid"], "9d1e6a7c-2f04-4c1b-9a41-0d5c7f2b8e10");

    // The clone is a real EHR on the released wire.
    let (status, _body) = send(&app, get("/ehr/9d1e6a7c-2f04-4c1b-9a41-0d5c7f2b8e10")).await;
    assert_eq!(status, StatusCode::OK, "the cloned EHR is readable");
}

/// `import_ehr` with a fixed `ehr_id` lands the clone under THAT identifier
/// (the SM's "same patient in other EHR services" case).
#[tokio::test]
async fn import_ehr_with_a_fixed_id_lands_under_it() {
    let (_pg, app) = common::test_router().await;
    let target = "7d44b88c-4199-4bad-97dc-d78268e01398";

    let (status, body) = send(
        &app,
        post(
            &format!("/message/import?ehr_id={target}"),
            "application/json",
            corpus("ehr_extract.v2.json"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "import_ehr with id: {body}");
    let created: Value = serde_json::from_str(&body).expect("identifier json");
    assert_eq!(created["uid"], target);
}

/// "import EHRs with duplicate EHR ids will fail" (`i_admin_dump_load.adoc`'s
/// sibling wording in `i_ehr_extract_service.adoc`'s Case-1 clone): the target
/// must be empty, so an existing id is `ehr_create_fail_duplicate_id` → `409`.
#[tokio::test]
async fn import_ehr_into_an_existing_id_is_a_conflict() {
    let (_pg, app) = common::test_router().await;
    let existing = create_ehr(&app).await;

    let (status, body) = send(
        &app,
        post(
            &format!("/message/import?ehr_id={existing}"),
            "application/json",
            corpus("ehr_extract.v1.json"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "duplicate id: {body}");
}

/// `import_ehr_extract` lands a new versioned object in an EXISTING EHR (RM
/// common `master06` §Copying Case 2) and answers `204`.
#[tokio::test]
async fn import_ehr_extract_lands_content_in_an_existing_ehr() {
    let (_pg, app) = common::test_router().await;
    let ehr_id = create_ehr(&app).await;

    let (status, body) = send(
        &app,
        post(
            &format!("/message/import/{ehr_id}"),
            "application/json",
            corpus("ehr_extract.folder.json"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "import extract: {body}");

    // The imported FOLDER hierarchy is readable on the released directory wire.
    let (status, body) = send(&app, get(&format!("/ehr/{ehr_id}/directory"))).await;
    assert_eq!(status, StatusCode::OK, "imported directory: {body}");
}

/// SM `ehr_id_does_not_exist` → `404`.
#[tokio::test]
async fn import_ehr_extract_into_an_unknown_ehr_is_not_found() {
    let (_pg, app) = common::test_router().await;
    let (status, _body) = send(
        &app,
        post(
            &format!("/message/import/{ABSENT}"),
            "application/json",
            corpus("ehr_extract.folder.json"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A payload that is not a well-formed `EXTRACT` at all (none of the mandatory
/// `time_created` / `system_id` / `sequence_nr`) is the ITS-REST `400` branch —
/// "malformed request syntax, syntactically invalid content" — not the `422`
/// well-formed-but-semantically-invalid one.
#[tokio::test]
async fn import_ehr_extract_of_a_malformed_extract_is_bad_request() {
    let (_pg, app) = common::test_router().await;
    let ehr_id = create_ehr(&app).await;

    let (status, body) = send(
        &app,
        post(
            &format!("/message/import/{ehr_id}"),
            "application/json",
            corpus("ehr_extract.invalid.json"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "malformed EXTRACT: {body}");
}

// ── Removed routes ───────────────────────────────────────────────────────────

/// The Template Data Document import routes were removed (#4199), so each
/// former path is no route at all: `404`, even for an EHR that exists and a
/// body the old route would have read.
#[tokio::test]
async fn the_former_tdd_routes_are_not_found() {
    let (_pg, app) = common::test_router().await;
    let ehr_id = create_ehr(&app).await;
    for path in [
        format!("/message/tdd/{ehr_id}"),
        format!("/message/tdd/{ehr_id}/batch"),
        "/message/tdd".to_owned(),
    ] {
        let (status, body) = send(&app, post(&path, "application/xml", "<x/>".to_owned())).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "POST {path}: {body}");
    }
}
