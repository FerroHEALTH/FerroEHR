// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the ADMIN storage-integrity route
//! (`POST /admin/integrity/verify`) — **our own extension**; no ITS-REST
//! operation and no SM interface governs it.
//!
//! What the streaming half proves is a property of the MIDDLEWARE, so it is
//! asserted against the middleware rather than argued: `tower_http`'s
//! `TimeoutLayer` races the inner service's response FUTURE, so a handler that
//! computes its whole answer before returning is bounded by it and a handler
//! that returns a head immediately is not. Each test therefore wraps the
//! assembled router in a `TimeoutLayer` whose budget is deliberately far too
//! short, which is the same layer production applies at 30 s.
#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; a panicking assertion is the \
              intended shape here (the Rust Book ch11)"
)]

use std::time::Duration;

use axum::body::Body;
use axum::response::Response;
use http::{Request, StatusCode, header};
use serde_json::Value;
use tower::ServiceExt;
use tower::util::BoxCloneService;
use tower_http::timeout::TimeoutLayer;

use crate::common;
use crate::common::BASE;

/// A budget no sweep can meet: the aggregated form needs several sequential
/// round trips to a real database before it can answer at all.
const IMPOSSIBLE: Duration = Duration::from_millis(1);

/// The route under test.
fn verify(accept: Option<&str>) -> Request<Body> {
    let mut req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/admin/integrity/verify"));
    if let Some(accept) = accept {
        req = req.header(header::ACCEPT, accept);
    }
    req.body(Body::empty()).expect("request")
}

/// The assembled router behind a deliberately impossible timeout, plus a seeded
/// repository for the sweep to read.
async fn app_with_impossible_timeout() -> (
    testkit::TestDb,
    BoxCloneService<Request<Body>, Response, std::convert::Infallible>,
) {
    let (pg, service) = common::test_service().await;
    service.create_ehr(None).await.expect("seed an EHR");
    let router = common::router_with(common::api_config(true), service);
    let stack = tower::ServiceBuilder::new()
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            IMPOSSIBLE,
        ))
        .service(router);
    (pg, BoxCloneService::new(stack))
}

#[tokio::test]
async fn the_aggregated_sweep_is_bounded_by_the_request_timeout() {
    let (_pg, app) = app_with_impossible_timeout().await;

    let response = app.oneshot(verify(None)).await.expect("response");

    assert_eq!(
        response.status(),
        StatusCode::REQUEST_TIMEOUT,
        "the aggregated form computes its whole report before the response \
         exists, so the timeout applies to it — this is the ceiling the \
         streamed form exists to escape"
    );
}

#[tokio::test]
async fn the_streamed_sweep_completes_under_a_timeout_the_aggregated_one_cannot_meet() {
    let (_pg, app) = app_with_impossible_timeout().await;

    let response = app
        .oneshot(verify(Some("application/x-ndjson")))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/x-ndjson")
    );

    // Draining the body happens entirely outside the timeout, which is the
    // whole claim: the sweep ran to its summary under a 1 ms budget.
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("drain the stream");
    let body = String::from_utf8(bytes.to_vec()).expect("utf-8 lines");
    let lines: Vec<Value> = body
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("every line is one JSON object"))
        .collect();

    assert!(!lines.is_empty(), "the stream carried no lines: {body:?}");
    let last = lines.last().expect("a non-empty stream has a last line");
    assert_eq!(
        last["type"], "summary",
        "a completed sweep ends with its summary, not an error line: {body:?}"
    );
    assert!(
        last["versions_checked"].as_u64().unwrap_or(0) >= 2,
        "an EHR create stores at least its EHR_STATUS and EHR_ACCESS: {body:?}"
    );
    assert_eq!(last["mismatch_count"], 0, "{body:?}");
    assert!(
        lines.iter().any(|line| line["type"] == "progress"),
        "a sweep that read a page says so while it runs: {body:?}"
    );
}

#[tokio::test]
async fn a_wildcard_accept_keeps_the_aggregated_document() {
    let (_pg, service) = common::test_service().await;
    let app = common::router_with(common::api_config(true), service);

    let (status, body) = common::send_body(&app, verify(Some("*/*"))).await;

    assert_eq!(status, StatusCode::OK);
    let report: Value = serde_json::from_str(&body).expect("one JSON document");
    assert!(
        report.get("mismatches").is_some(),
        "a wildcard must not switch an existing caller onto the stream: {body}"
    );
}

// ── the rebuild (`POST /admin/integrity/rebuild-nodes`) ─────────────────────

/// The repair route, with an optional query string.
fn rebuild(query: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("{BASE}/admin/integrity/rebuild-nodes{query}"))
        .body(Body::empty())
        .expect("request")
}

#[tokio::test]
async fn the_rebuild_answers_a_report_over_an_undamaged_repository() {
    let (_pg, service) = common::test_service().await;
    service.create_ehr(None).await.expect("seed an EHR");
    let app = common::router_with(common::api_config(true), service);

    let (status, body) = common::send_body(&app, rebuild("")).await;

    assert_eq!(status, StatusCode::OK);
    let report: Value = serde_json::from_str(&body).expect("one JSON document");
    assert!(
        report["versions_checked"].as_u64().unwrap_or(0) >= 2,
        "the sweep behind the repair still read the repository: {body}"
    );
    assert_eq!(report["versions_damaged"], 0, "{body}");
    assert_eq!(report["versions_rebuilt"], 0, "{body}");
    assert_eq!(report["versions_refused"], 0, "{body}");
    assert_eq!(
        report["records"].as_array().map(Vec::len),
        Some(0),
        "an undamaged repository produces no records: {body}"
    );
}

#[tokio::test]
async fn a_sys_version_without_a_vo_id_is_refused() {
    let (_pg, service) = common::test_service().await;
    let app = common::router_with(common::api_config(true), service);

    let (status, body) = common::send_body(&app, rebuild("?sys_version=2")).await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "the ordinal is per-object, so alone it names one version of every \
         object — never what an operator naming a version means: {body}"
    );
    assert!(
        body.contains("vo_id"),
        "the refusal names the parameter it needs: {body}"
    );
}

#[tokio::test]
async fn the_rebuild_is_gated_with_the_rest_of_the_admin_group() {
    let (_pg, service) = common::test_service().await;
    let app = common::router_with(common::api_config(false), service);

    let (status, _) = common::send_body(&app, rebuild("")).await;

    assert_eq!(
        status,
        StatusCode::METHOD_NOT_ALLOWED,
        "the whole ADMIN group is opt-in and answers 405 with an empty Allow"
    );
}
