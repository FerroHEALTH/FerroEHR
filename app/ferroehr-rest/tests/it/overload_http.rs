// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Ingress overload-shedding tests, driven through the fully assembled router
//! over the **real** `FerroEhrService` (the scripted `Mock`/parking
//! hook is gone).
//!
//! No openEHR spec governs server overload — this is our own design (RFC 9110
//! §15.6.4 for the `503` status). These tests exercise the real router stack
//! (`build_with`): the family stack's overload bound (`server.
//! max_concurrent_requests`) over every FerroEHR route, never the health
//! routes.
//!
//! Reproducing real contention without the removed in-handler hook: a request
//! whose handler buffers a **never-ending request body**
//! (`futures::stream::pending`) parks inside the API permit (the body is read
//! while the concurrency permit is held), so a further request to the **same
//! route** is shed. (The parked requests and the shed probe target the same
//! operation, `POST …/composition`, whose handler reads the body.) The per-request timeout
//! (30 s) is far longer than the probe window, and the parked tasks are aborted
//! at the end of each test, so no permit leaks.
//!
//! * beyond the `max_concurrent_requests` bound, an API request is shed
//!   immediately with `503 Service Unavailable` + `Retry-After: 1` and the
//!   problem details;
//! * with the bound unset concurrency is unbounded;
//! * the health routes are outside the bound and are never shed, even while
//!   the bound is fully taken.
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use http::{Request, Response, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use ferroehr::config::rest::RestConfig;
use ferroehr_rest::config::AppConfig;
use ferrohealth_server::section::ServerSection;
use ferrohealth_smart::config::AuthSection;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";
/// A syntactically valid EHR id (the dispatcher decodes it before the backend).
const EHR: &str = "3fa85f64-5717-4562-b3fc-2c963f66afa6";

/// Auth-disabled config with an explicit concurrency bound; `0` sets none.
fn config(max_in_flight: usize) -> AppConfig {
    AppConfig {
        server: ServerSection {
            max_concurrent_requests: std::num::NonZeroUsize::new(max_in_flight),
            ..ServerSection::default()
        },
        rest: RestConfig {
            base_path: BASE.to_owned(),
            swagger_ui: ferroehr::config::management::AccessLevel::Off,
            ..Default::default()
        },
        auth: AuthSection {
            enabled: false,
            basic: None,
            ..AuthSection::default()
        },
        ..Default::default()
    }
}

async fn app(max_in_flight: usize) -> (testkit::TestDb, Router) {
    let (pg, service) = common::test_service().await;
    (
        pg,
        ferroehr_rest::build_with(config(max_in_flight), service)
            .await
            .expect("router builds"),
    )
}

/// A request whose handler parks holding its API permit: a composition create
/// whose body never ends, so the body-buffering extractor (`into_parts`) awaits
/// forever (well within the 30 s request timeout).
fn parking_request() -> Request<Body> {
    let never = futures::stream::pending::<Result<Bytes, std::io::Error>>();
    Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr/{EHR}/composition"))
        .header("content-type", "application/json")
        .body(Body::from_stream(never))
        .expect("request")
}

/// A shed-probe on the **same route** as [`parking_request`]. When the route's
/// permits are exhausted this is shed at `poll_ready` (before its body is read),
/// so an empty body suffices and the `503` returns immediately.
fn probe_request() -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr/{EHR}/composition"))
        .header("content-type", "application/json")
        .body(Body::empty())
        .expect("request")
}

fn get_ehr() -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(format!("{BASE}/ehr/{EHR}"))
        .body(Body::empty())
        .expect("request")
}

/// Probe the (parked) route until a request is shed (`503`), giving the parked
/// handlers time to acquire their permits. Fails if no shed is observed within
/// ~5 s.
async fn probe_until_shed(app: &Router) -> Response<Body> {
    for _ in 0..250 {
        let resp = app
            .clone()
            .oneshot(probe_request())
            .await
            .expect("response");
        if resp.status() == StatusCode::SERVICE_UNAVAILABLE {
            return resp;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no request was shed within the probe window (permits never saturated)");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn requests_beyond_limit_are_shed_with_503() {
    let (_pg, app) = app(2).await;

    // Two parked handlers occupy both permits of the composition route. Let them
    // acquire the permits *before* probing: `LoadShed` sheds a request that can't
    // immediately get a permit, so a probe racing the parked tasks could steal a
    // permit and get one of them shed instead of parked.
    let p1 = tokio::spawn(app.clone().oneshot(parking_request()));
    let p2 = tokio::spawn(app.clone().oneshot(parking_request()));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !p1.is_finished() && !p2.is_finished(),
        "both parking requests must be holding permits (not shed)"
    );

    // A further request to the same route finds no free permit and is shed.
    let resp = probe_until_shed(&app).await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        resp.headers()
            .get(header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok()),
        Some("1")
    );
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    let body: Value = serde_json::from_slice(&bytes).expect("problem details");
    // RFC 9457 problem details, FerroEHR's `unavailable` type.
    assert_eq!(body["status"], 503);
    assert!(body.get("detail").and_then(Value::as_str).is_some());

    // Release the parked handlers (aborting drops their futures → frees permits).
    p1.abort();
    p2.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn limit_zero_disables_shedding() {
    // With no `max_concurrent_requests` no bound is installed, so concurrency is
    // unbounded: many concurrent real requests all complete, none is shed.
    let (_pg, app) = app(0).await;

    let mut handles = Vec::new();
    for _ in 0..8 {
        handles.push(tokio::spawn(app.clone().oneshot(get_ehr())));
    }
    for h in handles {
        let status = h.await.expect("join").expect("response").status();
        assert_ne!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "no request may be shed when the limit is disabled"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_health_routes_are_never_shed() {
    let (_pg, app) = app(1).await;

    // Saturate the single permit of the composition route with one parked
    // handler; let it acquire the permit before probing (see the note in
    // `requests_beyond_limit_are_shed_with_503`).
    let parked = tokio::spawn(app.clone().oneshot(parking_request()));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !parked.is_finished(),
        "the parking request must hold the permit"
    );

    // A second request to that route is shed — the limit is genuinely saturated.
    let shed = probe_until_shed(&app).await;
    assert_eq!(shed.status(), StatusCode::SERVICE_UNAVAILABLE);

    // …but the health routes are outside the bound and answer normally, so an
    // orchestrator can always probe an overloaded server.
    for path in ["/health/readiness", "/health/liveness", "/health"] {
        let probe = Request::builder()
            .method("GET")
            .uri(path)
            .body(Body::empty())
            .expect("request");
        let answered = app.clone().oneshot(probe).await.expect("response");
        assert_eq!(answered.status(), StatusCode::OK, "{path}");
    }

    parked.abort();
}
