// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the DEFINITION group's **archetype + artefact
//! extension** routes — the ADL 1.4 archetype store
//! (`/definition/archetype/adl1.4*`) and the ADL 2 archetype/artefact views
//! (`/definition/archetype/adl2*`, `/definition/artefact/adl2*`) — driven
//! through the assembled router over a real `FerroEhrService` on a real
//! `PostgreSQL`.
//!
//! **No openEHR spec governs these routes**: the released Definition API
//! provisions operational templates only
//! (`specifications/operations/definition_template_adl{1.4,2}_*.yaml`). The
//! operation SEMANTICS come from
//! `docs/specs/openehr/SM/docs/UML/classes/i_definition_adl14.adoc` and
//! `i_definition_adl2.adoc`; the wire shape is our own design/extension
//! for both generations. What is asserted here is exactly what the CNF
//! `adl14-archetype` / `adl2-archetype` `served_extensions` families declare.
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
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";

/// A known-good ADL 1.4 source archetype: the same fixture the service-layer
/// Definitions battery uses, read from the shared corpus at
/// `corpus/fixtures/service/knowledge/archetypes`.
fn adl14_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/fixtures/service/knowledge/archetypes")
        .join("openEHR-EHR-COMPOSITION.prescription.v1.adl");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

const ADL14_ID: &str = "openEHR-EHR-COMPOSITION.prescription.v1";

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, http::HeaderMap, String) {
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

fn get(path: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(format!("{BASE}{path}"))
        .body(Body::empty())
        .expect("request")
}

fn delete(path: &str) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(format!("{BASE}{path}"))
        .body(Body::empty())
        .expect("request")
}

fn post_text(path: &str, body: String) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("{BASE}{path}"))
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(body))
        .expect("request")
}

/// The full ADL 1.4 archetype lifecycle over the extension wire: an empty store
/// lists `[]`, an upload answers `201` + `Location` + the stored
/// `ARCHETYPE_ID`, the source comes back byte-identical as `text/plain`, the id
/// appears in the list, and the delete answers `204` and makes both the read
/// and a second delete `404` (SM `Post_archetype_removed` /
/// `artefact_does_not_exist`).
#[tokio::test]
async fn the_adl14_archetype_store_round_trips_over_the_extension_wire() {
    let (_pg, app) = common::test_router().await;

    let (status, _, body) = send(&app, get("/definition/archetype/adl1.4")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "[]", "an empty store lists an empty collection");

    let adl = adl14_source();
    let (status, headers, body) =
        send(&app, post_text("/definition/archetype/adl1.4", adl.clone())).await;
    assert_eq!(status, StatusCode::CREATED, "upload: {body}");
    assert_eq!(
        body,
        format!("\"{ADL14_ID}\""),
        "the create returns the stored ARCHETYPE_ID"
    );
    let location = headers
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        location.ends_with(&format!("/definition/archetype/adl1.4/{ADL14_ID}")),
        "Location must address the stored archetype; has {location:?}"
    );

    let (status, headers, stored) = send(
        &app,
        get(&format!("/definition/archetype/adl1.4/{ADL14_ID}")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stored, adl, "the stored source is returned verbatim");
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        content_type.starts_with("text/plain"),
        "ADL source is served as text/plain; has {content_type:?}"
    );

    let (status, _, body) = send(&app, get("/definition/archetype/adl1.4")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, format!("[\"{ADL14_ID}\"]"));

    let (status, _, body) = send(
        &app,
        delete(&format!("/definition/archetype/adl1.4/{ADL14_ID}")),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "delete: {body}");

    for req in [
        get(&format!("/definition/archetype/adl1.4/{ADL14_ID}")),
        delete(&format!("/definition/archetype/adl1.4/{ADL14_ID}")),
    ] {
        let (status, _, body) = send(&app, req).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "a removed archetype is gone: {body}"
        );
    }
}

/// The upload's two refusals: a source that is not a valid ADL 1.4 archetype is
/// `422` (SM `Pre_valid_archetype`), and a payload DECLARING a media type the
/// route cannot process as ADL source is `415`.
#[tokio::test]
async fn the_adl14_upload_refuses_invalid_source_and_a_wrong_media_type() {
    let (_pg, app) = common::test_router().await;

    let (status, _, body) = send(
        &app,
        post_text(
            "/definition/archetype/adl1.4",
            "this is not an archetype".to_owned(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");

    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/definition/archetype/adl1.4"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(adl14_source()))
        .expect("request");
    let (status, _, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{body}");
}

/// The ADL 2 views on an empty store: both lists answer `[]` and both counts
/// the bare SM `Integer` `0`, and a DELETE of an absent artefact is `404`
/// (SM `artefact_does_not_exist`).
#[tokio::test]
async fn the_adl2_archetype_and_artefact_views_answer_the_sm_shapes() {
    let (_pg, app) = common::test_router().await;

    for path in ["/definition/archetype/adl2", "/definition/artefact/adl2"] {
        let (status, _, body) = send(&app, get(path)).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        assert_eq!(body, "[]", "{path}");
    }
    for path in [
        "/definition/archetype/adl2/count",
        "/definition/artefact/adl2/count",
    ] {
        let (status, _, body) = send(&app, get(path)).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        assert_eq!(body, "0", "{path} returns the bare SM Integer");
    }

    let (status, _, body) = send(
        &app,
        delete("/definition/artefact/adl2/openEHR-EHR-COMPOSITION.absent.v1.0.0"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

/// The SM list cursor (`master02-overview.adoc` §List Handling) is honoured on
/// the list routes, and a non-numeric cursor is a `400`.
#[tokio::test]
async fn the_list_cursor_pages_and_refuses_a_non_numeric_value() {
    let (_pg, app) = common::test_router().await;

    let (status, _, body) = send(
        &app,
        post_text("/definition/archetype/adl1.4", adl14_source()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let (status, _, body) = send(&app, get("/definition/archetype/adl1.4?offset=1")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "[]", "an offset past the end yields nothing");

    let (status, _, body) = send(&app, get("/definition/archetype/adl1.4?fetch=1")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, format!("[\"{ADL14_ID}\"]"));

    let (status, _, _) = send(&app, get("/definition/archetype/adl1.4?offset=many")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
